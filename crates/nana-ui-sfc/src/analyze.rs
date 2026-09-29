//! What a component's signals are, how every binding reads them, and what
//! the compiler may therefore do: fold never-written signals to constants,
//! write constant-only bindings once, declare static dependencies, and
//! report the rest.
//!
//! Usage is found by scanning tokens, which cannot see shadowing. Every
//! uncertain case counts against optimizing: a token that might be a
//! signal handed elsewhere is an escape, a call the scanner cannot see into
//! makes a binding dynamic.

use std::collections::{HashMap, HashSet};

use proc_macro2::{Delimiter, Ident, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{Expr, Pat, PatType, Stmt, Type};

/// Methods that read a signal (and track the read).
const READS: &[&str] = &[
    "get",
    "with",
    "get_untracked",
    "with_untracked",
    "map",
    "dep",
];
/// Methods that write one.
const WRITES: &[&str] = &["set", "update"];
/// Macros that only format their arguments.
const PURE_MACROS: &[&str] = &["format", "format_args", "concat", "stringify"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Signal,
    Computed,
    /// A `Signal` / `Computed` handed in as a prop: never folded.
    Prop,
}

#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub reads: usize,
    pub writes: usize,
    pub escapes: usize,
}

pub struct Tracked {
    pub name: String,
    pub kind: Kind,
    /// The declaring statement, for locals.
    pub decl: Option<usize>,
    pub usage: Usage,
    /// Tracked names a computed reads.
    pub reads: Vec<String>,
    /// Whether the computed's body is pure (only reads and formatting).
    pub pure: bool,
    pub folded: bool,
    pub line: usize,
}

impl Tracked {
    fn new(ident: &Ident, kind: Kind, decl: Option<usize>) -> Self {
        Self {
            name: ident.to_string(),
            kind,
            decl,
            usage: Usage::default(),
            reads: Vec::new(),
            pure: false,
            folded: false,
            line: ident.span().start().line,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Read,
    Write,
    Escape,
}

#[derive(Default)]
struct Scan {
    uses: Vec<(String, Use)>,
    /// No call the scanner cannot see into.
    pure: bool,
}

fn scan(tokens: TokenStream, names: &HashMap<String, usize>) -> Scan {
    let mut out = Scan {
        uses: Vec::new(),
        pure: true,
    };
    scan_into(tokens, names, &mut out);
    out
}

fn is_punct(token: Option<&TokenTree>, ch: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ch)
}

fn scan_into(tokens: TokenStream, names: &HashMap<String, usize>, out: &mut Scan) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|i| tokens.get(i));
        let next = tokens.get(index + 1);
        match token {
            TokenTree::Group(group) => scan_into(group.stream(), names, out),
            TokenTree::Literal(literal) => {
                // `format!("{name}")` reads `name` through `Display`.
                let text = literal.to_string();
                if text.starts_with('"') || text.starts_with('r') {
                    for name in inline_arguments(&text) {
                        if names.contains_key(name) {
                            out.uses.push((name.to_owned(), Use::Read));
                        }
                    }
                }
            }
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let after_dot = is_punct(previous, '.');
                let called = matches!(next, Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis);
                if after_dot {
                    // A method or field of something else. Reading a signal
                    // that is not a tracked name (a struct field, a closure
                    // parameter) is invisible here.
                    if called && READS.contains(&name.as_str()) {
                        let receiver = index.checked_sub(2).and_then(|i| tokens.get(i));
                        let tracked = matches!(receiver, Some(TokenTree::Ident(r)) if names.contains_key(&r.to_string()))
                            && !is_punct(index.checked_sub(3).and_then(|i| tokens.get(i)), '.');
                        if !tracked {
                            out.pure = false;
                        }
                    }
                    continue;
                }
                if is_punct(next, '!') {
                    if !PURE_MACROS.contains(&name.as_str()) {
                        out.pure = false;
                    }
                    continue;
                }
                if called
                    && name.starts_with(|c: char| c.is_lowercase())
                    && !names.contains_key(&name)
                {
                    out.pure = false;
                    continue;
                }
                if !names.contains_key(&name) {
                    continue;
                }
                let method = match (next, tokens.get(index + 2)) {
                    (Some(TokenTree::Punct(dot)), Some(TokenTree::Ident(method)))
                        if dot.as_char() == '.' =>
                    {
                        Some(method.to_string())
                    }
                    _ => None,
                };
                let used = match method.as_deref() {
                    Some(method) if READS.contains(&method) => Use::Read,
                    Some(method) if WRITES.contains(&method) => Use::Write,
                    _ => Use::Escape,
                };
                out.uses.push((name, used));
            }
            TokenTree::Punct(_) => {}
        }
    }
}

/// `{name}` / `{name:…}` arguments of a format string literal.
fn inline_arguments(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if rest.starts_with('{') {
            rest = &rest[1..];
            continue;
        }
        let end = rest.find(['}', ':']).unwrap_or(rest.len());
        let name = rest[..end].trim();
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            names.push(name);
        }
    }
    names
}

fn single_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) if path.qself.is_none() => path.path.get_ident().map(ToString::to_string),
        _ => None,
    }
}

/// The callee's last segment when `expr` is `name(…)`.
fn call_name(expr: &Expr) -> Option<(String, &syn::ExprCall)> {
    match expr {
        Expr::Call(call) => match &*call.func {
            Expr::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| (segment.ident.to_string(), call)),
            _ => None,
        },
        _ => None,
    }
}

fn is_watch(stmt: &Stmt) -> bool {
    let expr = match stmt {
        Stmt::Expr(expr, _) => expr,
        Stmt::Local(local) => match &local.init {
            Some(init) => &*init.expr,
            None => return false,
        },
        _ => return false,
    };
    call_name(expr).is_some_and(|(name, _)| name == "watch_effect")
}

fn is_reactive_type(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Signal" || segment.ident == "Computed"),
        _ => false,
    }
}

/// How a template binding reads signals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// A literal, a plain value or a folded constant: written once.
    Value,
    /// Exactly one live signal or computed: a direct binding.
    Direct(String),
    /// Reads exactly these live signals, through code the compiler sees.
    Static(Vec<String>),
    /// Anything else: tracked at run time.
    Dynamic,
}

impl Class {
    fn from_deps(deps: Vec<String>) -> Self {
        if deps.is_empty() {
            Class::Value
        } else {
            Class::Static(deps)
        }
    }

    pub fn label(&self) -> String {
        match self {
            Class::Value => "常量".to_owned(),
            Class::Direct(name) => format!("直接（{name}）"),
            Class::Static(deps) => format!("静态依赖（{}）", deps.join(", ")),
            Class::Dynamic => "动态".to_owned(),
        }
    }
}

pub struct Analysis {
    pub tracked: Vec<Tracked>,
    index: HashMap<String, usize>,
    pub warnings: Vec<String>,
}

impl Analysis {
    pub fn new(props: &[PatType], script: &[Stmt]) -> Self {
        let mut tracked = Vec::new();
        for prop in props {
            if let Pat::Ident(pat) = &*prop.pat
                && is_reactive_type(&prop.ty)
            {
                tracked.push(Tracked::new(&pat.ident, Kind::Prop, None));
            }
        }
        for (index, stmt) in script.iter().enumerate() {
            let Stmt::Local(local) = stmt else { continue };
            // `let x = …` or `let x: T = …`.
            let pat = match &local.pat {
                Pat::Type(typed) => &*typed.pat,
                other => other,
            };
            let (Pat::Ident(pat), Some(init)) = (pat, &local.init) else {
                continue;
            };
            let kind = match call_name(&init.expr) {
                Some((name, _)) if name == "signal" => Kind::Signal,
                Some((name, _)) if name == "computed" => Kind::Computed,
                _ => continue,
            };
            tracked.push(Tracked::new(&pat.ident, kind, Some(index)));
        }
        let index = tracked
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        Self {
            tracked,
            index,
            warnings: Vec::new(),
        }
    }

    fn bump(&mut self, name: &str, used: Use) {
        if let Some(&i) = self.index.get(name) {
            let usage = &mut self.tracked[i].usage;
            match used {
                Use::Read => usage.reads += 1,
                Use::Write => usage.writes += 1,
                Use::Escape => usage.escapes += 1,
            }
        }
    }

    fn record(&mut self, scan: &Scan) {
        for (name, used) in &scan.uses {
            self.bump(name, *used);
        }
    }

    /// Count the script's uses. A declaration contributes its initializer
    /// only; a computed's reads are kept for folding and cycle checks.
    pub fn scan_script(&mut self, script: &[Stmt]) {
        for (index, stmt) in script.iter().enumerate() {
            let tokens = match stmt {
                // A declaration's own call (`signal(…)`, `computed(…)`) is
                // not a use; its arguments are.
                Stmt::Local(local) => match &local.init {
                    Some(init) => match call_name(&init.expr) {
                        Some((name, call)) if name == "signal" || name == "computed" => {
                            call.args.to_token_stream()
                        }
                        _ => init.expr.to_token_stream(),
                    },
                    None => continue,
                },
                other => other.to_token_stream(),
            };
            let found = scan(tokens, &self.index);
            if let Some(owner) = self
                .tracked
                .iter_mut()
                .find(|t| t.decl == Some(index) && t.kind == Kind::Computed)
            {
                owner.reads = found
                    .uses
                    .iter()
                    .filter(|(_, used)| *used == Use::Read)
                    .map(|(name, _)| name.clone())
                    .collect();
                owner.pure = found.pure && found.uses.iter().all(|(_, used)| *used == Use::Read);
            }
            if is_watch(stmt) {
                self.watch_self_write(&found);
            }
            self.record(&found);
        }
    }

    /// A `watch_effect` that writes what it reads re-queues itself.
    fn watch_self_write(&mut self, found: &Scan) {
        let reads: HashSet<&String> = found
            .uses
            .iter()
            .filter(|(_, u)| *u == Use::Read)
            .map(|(n, _)| n)
            .collect();
        for (written, _) in found.uses.iter().filter(|(_, u)| *u == Use::Write) {
            if reads.contains(written) {
                self.warnings.push(format!(
                    "`watch_effect` reads and writes `{written}`; each write queues it again"
                ));
            }
        }
    }

    /// Count a template expression that is not itself a binding value
    /// (handlers, keys, component arguments, loop sources): a bare tracked
    /// name counts as `bare`.
    pub fn scan_use(&mut self, expr: &Expr, bare: &[Use]) {
        if let Some(name) = single_ident(expr)
            && self.index.contains_key(&name)
        {
            for used in bare {
                self.bump(&name, *used);
            }
            return;
        }
        let found = scan(expr.to_token_stream(), &self.index);
        self.record(&found);
    }

    /// Count a binding value's uses (first pass, before folding).
    pub fn count_binding(&mut self, expr: &Expr) {
        if !matches!(expr, Expr::Lit(_)) {
            self.scan_use(expr, &[Use::Read]);
        }
    }

    /// How a binding value reads signals (second pass, after folding).
    pub fn classify(&self, expr: &Expr) -> Class {
        match expr {
            Expr::Lit(_) | Expr::Field(_) => return Class::Value,
            Expr::Closure(_) => return Class::Dynamic,
            _ => {}
        }
        if let Some(name) = single_ident(expr) {
            return match self.index.get(&name) {
                Some(&i) if !self.tracked[i].folded => Class::Direct(name),
                _ => Class::Value,
            };
        }
        self.class_of(&scan(expr.to_token_stream(), &self.index))
    }

    fn class_of(&self, found: &Scan) -> Class {
        if !found.pure || found.uses.iter().any(|(_, used)| *used != Use::Read) {
            return Class::Dynamic;
        }
        let mut deps: Vec<String> = Vec::new();
        for (name, _) in &found.uses {
            let tracked = &self.tracked[self.index[name]];
            if !tracked.folded && !deps.contains(name) {
                deps.push(name.clone());
            }
        }
        Class::from_deps(deps)
    }

    /// `{{ a }} and {{ b }}`: the parts' classes combined.
    pub fn classify_all(&self, exprs: &[&Expr]) -> Class {
        let mut deps = Vec::new();
        for expr in exprs {
            let names = match self.classify(expr) {
                Class::Value => continue,
                Class::Direct(name) => vec![name],
                Class::Static(names) => names,
                Class::Dynamic => return Class::Dynamic,
            };
            for name in names {
                if !deps.contains(&name) {
                    deps.push(name);
                }
            }
        }
        Class::from_deps(deps)
    }

    /// Fold what may be folded: a signal nothing writes or hands out, and a
    /// computed that only reads folded values. Repeats until nothing more
    /// folds, so chains of computeds collapse.
    pub fn fold(&mut self) {
        loop {
            let mut changed = false;
            for i in 0..self.tracked.len() {
                let tracked = &self.tracked[i];
                if tracked.folded || tracked.usage.writes > 0 || tracked.usage.escapes > 0 {
                    continue;
                }
                let foldable = match tracked.kind {
                    Kind::Signal => true,
                    Kind::Computed => {
                        tracked.pure
                            && tracked.reads.iter().all(|name| {
                                self.index
                                    .get(name)
                                    .is_some_and(|&j| self.tracked[j].folded)
                            })
                    }
                    Kind::Prop => false,
                };
                if foldable {
                    self.tracked[i].folded = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub fn is_folded(&self, stmt: usize) -> Option<Kind> {
        self.tracked
            .iter()
            .find(|t| t.decl == Some(stmt) && t.folded)
            .map(|t| t.kind)
    }

    /// Computeds that read each other in a loop.
    pub fn cycles(&self) -> Option<Vec<String>> {
        fn visit(
            analysis: &Analysis,
            name: &str,
            stack: &mut Vec<String>,
            done: &mut HashSet<String>,
        ) -> Option<Vec<String>> {
            if let Some(start) = stack.iter().position(|n| n == name) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(name.to_owned());
                return Some(cycle);
            }
            if !done.insert(name.to_owned()) {
                return None;
            }
            let tracked = &analysis.tracked[*analysis.index.get(name)?];
            if tracked.kind != Kind::Computed {
                return None;
            }
            stack.push(name.to_owned());
            for read in &tracked.reads {
                if let Some(cycle) = visit(analysis, read, stack, done) {
                    return Some(cycle);
                }
            }
            stack.pop();
            None
        }
        let mut done = HashSet::new();
        for tracked in &self.tracked {
            if let Some(cycle) = visit(self, &tracked.name, &mut Vec::new(), &mut done) {
                return Some(cycle);
            }
        }
        None
    }

    /// Signals written but read by nothing.
    pub fn finish(&mut self) {
        for tracked in &self.tracked {
            if tracked.kind == Kind::Signal
                && tracked.usage.writes > 0
                && tracked.usage.reads == 0
                && tracked.usage.escapes == 0
            {
                self.warnings.push(format!(
                    "`{}` is written but nothing reads it",
                    tracked.name
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> HashMap<String, usize> {
        list.iter()
            .enumerate()
            .map(|(i, s)| (s.to_string(), i))
            .collect()
    }

    fn uses(code: &str, list: &[&str]) -> (Vec<(String, Use)>, bool) {
        let tokens: TokenStream = code.parse().unwrap();
        let found = scan(tokens, &names(list));
        (found.uses, found.pure)
    }

    #[test]
    fn methods_decide_read_write_and_escape() {
        let (found, pure) = uses("a.get() + b.with(|v| v.len()) ", &["a", "b"]);
        assert_eq!(found, [("a".into(), Use::Read), ("b".into(), Use::Read)]);
        assert!(pure);
        let (found, _) = uses("a.set(1); b.update(|v| *v += 1)", &["a", "b"]);
        assert_eq!(found, [("a".into(), Use::Write), ("b".into(), Use::Write)]);
        let (found, _) = uses("child(a)", &["a"]);
        assert_eq!(found, [("a".into(), Use::Escape)]);
        let (found, _) = uses("todo.a.get()", &["a"]);
        assert!(found.is_empty(), "a field named like a signal is not it");
    }

    #[test]
    fn unseen_calls_make_code_impure() {
        assert!(uses("format!(\"{a} {}\", b.get())", &["a", "b"]).1);
        assert_eq!(
            uses("format!(\"{a} {}\", b.get())", &["a", "b"]).0,
            [("a".into(), Use::Read), ("b".into(), Use::Read)]
        );
        assert!(
            !uses("helper(a.get())", &["a"]).1,
            "a free function may read anything"
        );
        assert!(
            !uses("row.title.get()", &["a"]).1,
            "an untracked signal read"
        );
        assert!(!uses("println!(\"x\")", &[]).1);
        assert!(
            uses("Some(a.get())", &["a"]).1,
            "a constructor is not a call"
        );
        assert!(uses("d.trim().is_empty()", &[]).1);
    }
}
