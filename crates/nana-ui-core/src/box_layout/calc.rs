//! Full CSS `calc()` AST (plus nested `min` / `max` / `clamp`).
//!
//! [`CalcExpr`] is a boxed tree. [`LengthSpec::Calc`] stores an interned
//! `&'static` handle so the [`LengthSpec`] enum stays `Copy` (layout copies
//! specs; Runtime `layout_engine` is not in this crate's ownership).
//! Resolution is [`CalcExpr::resolve_with_fonts`]; the layout engine only
//! consumes px.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

use super::{FontSizeContext, LengthAtom, LengthSpec, ViewportAxes, ViewportAxis};

/// Binary operator inside [`CalcExpr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcBinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Nested calc / min / max / clamp expression. Children are boxed.
#[derive(Debug, Clone, PartialEq)]
pub enum CalcExpr {
    Number(f32),
    Px(f32),
    Percent(f32),
    Em(f32),
    Rem(f32),
    Viewport {
        axis: ViewportAxis,
        value: f32,
    },
    Binary {
        op: CalcBinOp,
        left: Box<CalcExpr>,
        right: Box<CalcExpr>,
    },
    Neg(Box<CalcExpr>),
    Min(Box<CalcExpr>, Box<CalcExpr>),
    Max(Box<CalcExpr>, Box<CalcExpr>),
    Clamp {
        min: Box<CalcExpr>,
        val: Box<CalcExpr>,
        max: Box<CalcExpr>,
    },
}

/// Compact 4-byte interned `Box<CalcExpr>` handle with a zero niche, keeping
/// `Option<LengthSpec>` at 8 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CalcExprRef(NonZeroU32);

const _: () = assert!(std::mem::size_of::<CalcExprRef>() == 4);
const _: () = assert!(std::mem::size_of::<Option<CalcExprRef>>() == 4);

const PAGE_SHIFT: usize = 10;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const MAX_PAGES: usize = 1024;

static CALC_PAGES: [OnceLock<Box<[AtomicPtr<CalcExpr>]>>; MAX_PAGES] =
    [const { OnceLock::new() }; MAX_PAGES];

#[derive(Default)]
struct CalcInternTable {
    buckets: HashMap<u64, Vec<CalcExprRef>>,
    next_index: u32,
}

static CALC_INTERN: OnceLock<Mutex<CalcInternTable>> = OnceLock::new();

fn calc_intern() -> &'static Mutex<CalcInternTable> {
    CALC_INTERN.get_or_init(|| Mutex::new(CalcInternTable::default()))
}

impl CalcExprRef {
    pub fn inner(self) -> &'static CalcExpr {
        let idx = (self.0.get() - 1) as usize;
        let page = CALC_PAGES[idx >> PAGE_SHIFT]
            .get()
            .expect("valid CalcExprRef page");
        let ptr = page[idx & (PAGE_SIZE - 1)].load(Ordering::Acquire);
        // SAFETY: `intern_calc` publishes a non-null `Box::leak` `'static`
        // pointer with `Ordering::Release` before returning `CalcExprRef`.
        unsafe { &*ptr }
    }

    pub const fn raw(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Debug for CalcExprRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner().fmt(f)
    }
}

fn hash_f32_canonical(val: f32, state: &mut impl Hasher) {
    let bits = if val == 0.0 { 0u32 } else { val.to_bits() };
    bits.hash(state);
}

fn hash_calc_expr(expr: &CalcExpr, state: &mut impl Hasher) {
    match expr {
        CalcExpr::Number(v) => {
            0u8.hash(state);
            hash_f32_canonical(*v, state);
        }
        CalcExpr::Px(v) => {
            1u8.hash(state);
            hash_f32_canonical(*v, state);
        }
        CalcExpr::Percent(v) => {
            2u8.hash(state);
            hash_f32_canonical(*v, state);
        }
        CalcExpr::Em(v) => {
            3u8.hash(state);
            hash_f32_canonical(*v, state);
        }
        CalcExpr::Rem(v) => {
            4u8.hash(state);
            hash_f32_canonical(*v, state);
        }
        CalcExpr::Viewport { axis, value } => {
            5u8.hash(state);
            (*axis as u8).hash(state);
            hash_f32_canonical(*value, state);
        }
        CalcExpr::Binary { op, left, right } => {
            6u8.hash(state);
            (*op as u8).hash(state);
            hash_calc_expr(left, state);
            hash_calc_expr(right, state);
        }
        CalcExpr::Neg(inner) => {
            7u8.hash(state);
            hash_calc_expr(inner, state);
        }
        CalcExpr::Min(a, b) => {
            8u8.hash(state);
            hash_calc_expr(a, state);
            hash_calc_expr(b, state);
        }
        CalcExpr::Max(a, b) => {
            9u8.hash(state);
            hash_calc_expr(a, state);
            hash_calc_expr(b, state);
        }
        CalcExpr::Clamp { min, val, max } => {
            10u8.hash(state);
            hash_calc_expr(min, state);
            hash_calc_expr(val, state);
            hash_calc_expr(max, state);
        }
    }
}

/// Box and intern `expr` so it can live on a Copy [`LengthSpec`].
pub fn intern_calc(expr: Box<CalcExpr>) -> CalcExprRef {
    let mut hasher = DefaultHasher::new();
    hash_calc_expr(&expr, &mut hasher);
    let hash = hasher.finish();
    let mut table = calc_intern().lock().expect("calc intern");
    if let Some(bucket) = table.buckets.get(&hash)
        && let Some(&existing) = bucket.iter().find(|r| r.inner() == expr.as_ref())
    {
        return existing;
    }
    let idx = table.next_index;
    let page_idx = (idx as usize) >> PAGE_SHIFT;
    let slot_idx = (idx as usize) & (PAGE_SIZE - 1);
    assert!(
        page_idx < MAX_PAGES,
        "calc intern table overflow ({} expressions)",
        idx
    );
    let page = CALC_PAGES[page_idx].get_or_init(|| {
        (0..PAGE_SIZE)
            .map(|_| AtomicPtr::new(std::ptr::null_mut()))
            .collect::<Vec<_>>()
            .into_boxed_slice()
    });
    let leaked: &'static CalcExpr = Box::leak(expr);
    page[slot_idx].store(
        leaked as *const CalcExpr as *mut CalcExpr,
        Ordering::Release,
    );
    let id = CalcExprRef(NonZeroU32::new(idx + 1).expect("calc index + 1 is non-zero"));
    table.next_index = idx + 1;
    table.buckets.entry(hash).or_default().push(id);
    id
}

impl LengthAtom {
    pub fn to_calc_expr(self) -> CalcExpr {
        let offset = |base: CalcExpr, offset_px: f32| CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(base),
            right: Box::new(CalcExpr::Px(offset_px)),
        };
        match self {
            Self::Px(v) => CalcExpr::Px(v),
            Self::Percent(p) => CalcExpr::Percent(p),
            Self::Em(v) => CalcExpr::Em(v),
            Self::Rem(v) => CalcExpr::Rem(v),
            Self::Viewport { axis, value } => CalcExpr::Viewport { axis, value },
            Self::CalcPercent { percent, offset_px } => {
                offset(CalcExpr::Percent(percent), offset_px)
            }
            Self::CalcViewport {
                axis,
                value,
                offset_px,
            } => offset(CalcExpr::Viewport { axis, value }, offset_px),
            Self::CalcEm { em, offset_px } => offset(CalcExpr::Em(em), offset_px),
            Self::CalcRem { rem, offset_px } => offset(CalcExpr::Rem(rem), offset_px),
        }
    }
}

#[derive(Clone, Copy)]
enum CalcType {
    Number,
    Length,
}

impl CalcExpr {
    pub fn depends_on_viewport(&self) -> bool {
        self.viewport_axes().any()
    }

    pub fn reads_percent(&self) -> bool {
        match self {
            Self::Percent(_) => true,
            Self::Number(_) | Self::Px(_) | Self::Em(_) | Self::Rem(_) | Self::Viewport { .. } => {
                false
            }
            Self::Binary { left, right, .. } => left.reads_percent() || right.reads_percent(),
            Self::Neg(inner) => inner.reads_percent(),
            Self::Min(a, b) | Self::Max(a, b) => a.reads_percent() || b.reads_percent(),
            Self::Clamp { min, val, max } => {
                min.reads_percent() || val.reads_percent() || max.reads_percent()
            }
        }
    }

    pub fn as_length_atom(&self) -> Option<LengthAtom> {
        match self {
            Self::Px(v) => Some(LengthAtom::Px(*v)),
            Self::Percent(p) => Some(LengthAtom::Percent(*p)),
            Self::Em(v) => Some(LengthAtom::Em(*v)),
            Self::Rem(v) => Some(LengthAtom::Rem(*v)),
            Self::Viewport { axis, value } => Some(LengthAtom::Viewport {
                axis: *axis,
                value: *value,
            }),
            Self::Binary {
                op: CalcBinOp::Add,
                left,
                right,
            } => {
                let Self::Px(offset_px) = right.as_ref() else {
                    return None;
                };
                match left.as_ref() {
                    Self::Percent(percent) => Some(LengthAtom::CalcPercent {
                        percent: *percent,
                        offset_px: *offset_px,
                    }),
                    Self::Em(em) => Some(LengthAtom::CalcEm {
                        em: *em,
                        offset_px: *offset_px,
                    }),
                    Self::Rem(rem) => Some(LengthAtom::CalcRem {
                        rem: *rem,
                        offset_px: *offset_px,
                    }),
                    Self::Viewport { axis, value } => Some(LengthAtom::CalcViewport {
                        axis: *axis,
                        value: *value,
                        offset_px: *offset_px,
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The sides of the viewport the expression reads.
    pub fn viewport_axes(&self) -> ViewportAxes {
        match self {
            Self::Viewport { axis, .. } => axis.reads(),
            Self::Number(_) | Self::Px(_) | Self::Percent(_) | Self::Em(_) | Self::Rem(_) => {
                ViewportAxes::NONE
            }
            Self::Binary { left, right, .. } => left.viewport_axes().union(right.viewport_axes()),
            Self::Neg(inner) => inner.viewport_axes(),
            Self::Min(a, b) | Self::Max(a, b) => a.viewport_axes().union(b.viewport_axes()),
            Self::Clamp { min, val, max } => min
                .viewport_axes()
                .union(val.viewport_axes())
                .union(max.viewport_axes()),
        }
    }

    /// Typed walk: `None` if the tree is not a valid CSS math expression.
    fn expr_type(&self) -> Option<CalcType> {
        match self {
            Self::Number(_) => Some(CalcType::Number),
            Self::Px(_) | Self::Percent(_) | Self::Em(_) | Self::Rem(_) | Self::Viewport { .. } => {
                Some(CalcType::Length)
            }
            Self::Neg(inner) => inner.expr_type(),
            Self::Binary { op, left, right } => {
                let lt = left.expr_type()?;
                let rt = right.expr_type()?;
                match op {
                    CalcBinOp::Add | CalcBinOp::Sub => {
                        if same_type_or_zero(left, lt, right, rt) {
                            if matches!(lt, CalcType::Length) || matches!(rt, CalcType::Length) {
                                Some(CalcType::Length)
                            } else {
                                Some(CalcType::Number)
                            }
                        } else {
                            None
                        }
                    }
                    CalcBinOp::Mul => match (lt, rt) {
                        (CalcType::Number, CalcType::Number) => Some(CalcType::Number),
                        (CalcType::Number, CalcType::Length)
                        | (CalcType::Length, CalcType::Number) => Some(CalcType::Length),
                        (CalcType::Length, CalcType::Length) => None,
                    },
                    CalcBinOp::Div => match (lt, rt) {
                        (_, CalcType::Length) => None,
                        (CalcType::Number, CalcType::Number) => Some(CalcType::Number),
                        (CalcType::Length, CalcType::Number) => Some(CalcType::Length),
                    },
                }
            }
            Self::Min(a, b) | Self::Max(a, b) => {
                let at = a.expr_type()?;
                let bt = b.expr_type()?;
                if matches!((at, bt), (CalcType::Length, CalcType::Length)) {
                    Some(CalcType::Length)
                } else {
                    None
                }
            }
            Self::Clamp { min, val, max } => {
                let a = min.expr_type()?;
                let b = val.expr_type()?;
                let c = max.expr_type()?;
                if matches!(
                    (a, b, c),
                    (CalcType::Length, CalcType::Length, CalcType::Length)
                ) {
                    Some(CalcType::Length)
                } else {
                    None
                }
            }
        }
    }

    /// `true` when this tree is a valid length-valued calc (not a leftover number).
    pub fn is_length_typed(&self) -> bool {
        matches!(self.expr_type(), Some(CalcType::Length))
    }

    /// Percent-only expression equivalent to `100%` / `calc(100% - Npx)` (`N ≥ 0`).
    pub fn is_full_percent_fill(&self) -> bool {
        match self {
            Self::Percent(percent) => (percent - 100.0).abs() < 0.5,
            Self::Min(a, b) | Self::Max(a, b) => {
                a.is_full_percent_fill() && b.is_full_percent_fill()
            }
            Self::Clamp { min, val, max } => {
                min.is_full_percent_fill()
                    && val.is_full_percent_fill()
                    && max.is_full_percent_fill()
            }
            other => match linearize(other) {
                Some(t) => {
                    (t.percent - 100.0).abs() < 0.5
                        && t.px <= 0.0
                        && t.em == 0.0
                        && t.rem == 0.0
                        && t.viewport.is_none()
                }
                None => false,
            },
        }
    }

    pub fn resolve_with_fonts(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<f32> {
        match self.resolve_typed(percent_base, viewport, fonts)? {
            TypedValue::Length(px) => Some(px),
            TypedValue::Number(_) => None,
        }
    }

    fn resolve_typed(
        &self,
        percent_base: Option<f32>,
        viewport: Option<(f32, f32)>,
        fonts: FontSizeContext,
    ) -> Option<TypedValue> {
        match self {
            Self::Number(n) => Some(TypedValue::Number(*n)),
            Self::Px(v) => Some(TypedValue::Length(*v)),
            Self::Percent(p) => percent_base.map(|base| TypedValue::Length(base * p / 100.0)),
            Self::Em(v) => Some(TypedValue::Length(fonts.element_px * v)),
            Self::Rem(v) => Some(TypedValue::Length(fonts.root_px * v)),
            Self::Viewport { axis, value } => {
                viewport.map(|(w, h)| TypedValue::Length(axis.base(w, h) * value / 100.0))
            }
            Self::Neg(inner) => inner
                .resolve_typed(percent_base, viewport, fonts)
                .map(TypedValue::neg),
            Self::Binary { op, left, right } => {
                let l = left.resolve_typed(percent_base, viewport, fonts)?;
                let r = right.resolve_typed(percent_base, viewport, fonts)?;
                match op {
                    CalcBinOp::Add => TypedValue::add(l, r),
                    CalcBinOp::Sub => TypedValue::add(l, r.neg()),
                    CalcBinOp::Mul => TypedValue::mul(l, r),
                    CalcBinOp::Div => TypedValue::div(l, r),
                }
            }
            Self::Min(a, b) => {
                let av = a
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                let bv = b
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                Some(TypedValue::Length(av.min(bv)))
            }
            Self::Max(a, b) => {
                let av = a
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                let bv = b
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                Some(TypedValue::Length(av.max(bv)))
            }
            Self::Clamp { min, val, max } => {
                let lo = min
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                let v = val
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                let hi = max
                    .resolve_typed(percent_base, viewport, fonts)?
                    .as_length()?;
                Some(TypedValue::Length(v.clamp(lo.min(hi), lo.max(hi))))
            }
        }
    }

    /// Fold a length-typed tree into a Copy [`LengthSpec`] when it matches an
    /// existing simple variant (`Px`, `%±px`, `min` of two atoms, …).
    pub fn simplify_to_length_spec(&self) -> Option<LengthSpec> {
        match self {
            Self::Min(a, b) => Some(LengthSpec::min2(expr_to_atom(a)?, expr_to_atom(b)?)),
            Self::Max(a, b) => Some(LengthSpec::max2(expr_to_atom(a)?, expr_to_atom(b)?)),
            Self::Clamp { min, val, max } => Some(LengthSpec::clamp3(
                expr_to_atom(min)?,
                expr_to_atom(val)?,
                expr_to_atom(max)?,
            )),
            other => terms_to_spec(linearize(other)?),
        }
    }
}

fn same_type_or_zero(left: &CalcExpr, lt: CalcType, right: &CalcExpr, rt: CalcType) -> bool {
    if matches!((lt, rt), (CalcType::Number, CalcType::Number))
        || matches!((lt, rt), (CalcType::Length, CalcType::Length))
    {
        return true;
    }
    is_zero_number(left) || is_zero_number(right)
}

fn is_zero_number(expr: &CalcExpr) -> bool {
    matches!(expr, CalcExpr::Number(0.0))
}

#[derive(Clone, Copy)]
enum TypedValue {
    Number(f32),
    Length(f32),
}

impl TypedValue {
    fn neg(self) -> Self {
        match self {
            Self::Number(n) => Self::Number(-n),
            Self::Length(v) => Self::Length(-v),
        }
    }

    fn as_length(self) -> Option<f32> {
        match self {
            Self::Length(v) => Some(v),
            Self::Number(0.0) => Some(0.0),
            Self::Number(_) => None,
        }
    }

    fn add(l: Self, r: Self) -> Option<Self> {
        match (l, r) {
            (Self::Number(a), Self::Number(b)) => Some(Self::Number(a + b)),
            (Self::Length(a), Self::Length(b)) => Some(Self::Length(a + b)),
            (Self::Number(0.0), Self::Length(b)) => Some(Self::Length(b)),
            (Self::Length(a), Self::Number(0.0)) => Some(Self::Length(a)),
            _ => None,
        }
    }

    fn mul(l: Self, r: Self) -> Option<Self> {
        match (l, r) {
            (Self::Number(a), Self::Number(b)) => Some(Self::Number(a * b)),
            (Self::Number(k), Self::Length(v)) | (Self::Length(v), Self::Number(k)) => {
                Some(Self::Length(v * k))
            }
            _ => None,
        }
    }

    fn div(l: Self, r: Self) -> Option<Self> {
        match (l, r) {
            (_, Self::Number(n)) if n == 0.0 || !n.is_finite() => None,
            (Self::Number(a), Self::Number(b)) => Some(Self::Number(a / b)),
            (Self::Length(v), Self::Number(n)) => Some(Self::Length(v / n)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct LengthTerms {
    px: f32,
    percent: f32,
    em: f32,
    rem: f32,
    viewport: Option<(ViewportAxis, f32)>,
}

fn linearize(expr: &CalcExpr) -> Option<LengthTerms> {
    match expr {
        CalcExpr::Number(0.0) => Some(LengthTerms::default()),
        CalcExpr::Number(_) => None,
        CalcExpr::Px(v) => Some(LengthTerms {
            px: *v,
            ..LengthTerms::default()
        }),
        CalcExpr::Percent(p) => Some(LengthTerms {
            percent: *p,
            ..LengthTerms::default()
        }),
        CalcExpr::Em(v) => Some(LengthTerms {
            em: *v,
            ..LengthTerms::default()
        }),
        CalcExpr::Rem(v) => Some(LengthTerms {
            rem: *v,
            ..LengthTerms::default()
        }),
        CalcExpr::Viewport { axis, value } => Some(LengthTerms {
            viewport: Some((*axis, *value)),
            ..LengthTerms::default()
        }),
        CalcExpr::Neg(inner) => {
            let mut t = linearize(inner)?;
            scale_terms(&mut t, -1.0);
            Some(t)
        }
        CalcExpr::Binary { op, left, right } => match op {
            CalcBinOp::Add => add_terms(linearize(left)?, linearize(right)?),
            CalcBinOp::Sub => {
                let mut r = linearize(right)?;
                scale_terms(&mut r, -1.0);
                add_terms(linearize(left)?, r)
            }
            CalcBinOp::Mul => {
                if let Some(k) = as_number(left) {
                    let mut t = linearize(right)?;
                    scale_terms(&mut t, k);
                    Some(t)
                } else if let Some(k) = as_number(right) {
                    let mut t = linearize(left)?;
                    scale_terms(&mut t, k);
                    Some(t)
                } else {
                    None
                }
            }
            CalcBinOp::Div => {
                let k = as_number(right)?;
                if k == 0.0 || !k.is_finite() {
                    return None;
                }
                let mut t = linearize(left)?;
                scale_terms(&mut t, 1.0 / k);
                Some(t)
            }
        },
        CalcExpr::Min(_, _) | CalcExpr::Max(_, _) | CalcExpr::Clamp { .. } => None,
    }
}

fn as_number(expr: &CalcExpr) -> Option<f32> {
    match expr {
        CalcExpr::Number(n) => Some(*n),
        CalcExpr::Neg(inner) => Some(-as_number(inner)?),
        CalcExpr::Binary {
            op: CalcBinOp::Add,
            left,
            right,
        } => Some(as_number(left)? + as_number(right)?),
        CalcExpr::Binary {
            op: CalcBinOp::Sub,
            left,
            right,
        } => Some(as_number(left)? - as_number(right)?),
        CalcExpr::Binary {
            op: CalcBinOp::Mul,
            left,
            right,
        } => Some(as_number(left)? * as_number(right)?),
        CalcExpr::Binary {
            op: CalcBinOp::Div,
            left,
            right,
        } => {
            let d = as_number(right)?;
            if d == 0.0 {
                None
            } else {
                Some(as_number(left)? / d)
            }
        }
        _ => None,
    }
}

fn scale_terms(t: &mut LengthTerms, k: f32) {
    t.px *= k;
    t.percent *= k;
    t.em *= k;
    t.rem *= k;
    if let Some((_, v)) = &mut t.viewport {
        *v *= k;
    }
}

fn add_terms(a: LengthTerms, b: LengthTerms) -> Option<LengthTerms> {
    let viewport = match (a.viewport, b.viewport) {
        (None, other) | (other, None) => other,
        (Some((ax, av)), Some((bx, bv))) if ax == bx => Some((ax, av + bv)),
        _ => return None,
    };
    Some(LengthTerms {
        px: a.px + b.px,
        percent: a.percent + b.percent,
        em: a.em + b.em,
        rem: a.rem + b.rem,
        viewport,
    })
}

fn terms_to_spec(t: LengthTerms) -> Option<LengthSpec> {
    let rel_count = (t.percent != 0.0) as u8
        + (t.em != 0.0) as u8
        + (t.rem != 0.0) as u8
        + t.viewport.is_some() as u8;
    if rel_count > 1 {
        return None;
    }
    if t.percent != 0.0 {
        return if t.px == 0.0 {
            if (t.percent - 100.0).abs() < 0.5 {
                Some(LengthSpec::Fill)
            } else {
                Some(LengthSpec::Percent(t.percent.clamp(0.0, 100.0)))
            }
        } else {
            Some(LengthSpec::percent_offset(t.percent, t.px))
        };
    }
    if t.em != 0.0 {
        return if t.px == 0.0 {
            Some(LengthSpec::Em(t.em))
        } else {
            Some(LengthSpec::em_offset(t.em, t.px))
        };
    }
    if t.rem != 0.0 {
        return if t.px == 0.0 {
            Some(LengthSpec::Rem(t.rem))
        } else {
            Some(LengthSpec::rem_offset(t.rem, t.px))
        };
    }
    if let Some((axis, value)) = t.viewport {
        return if t.px == 0.0 {
            Some(LengthSpec::Viewport { axis, value })
        } else {
            Some(LengthSpec::viewport_offset(axis, value, t.px))
        };
    }
    Some(LengthSpec::Px(t.px.max(0.0)))
}

fn expr_to_atom(expr: &CalcExpr) -> Option<LengthAtom> {
    expr.simplify_to_length_spec()?.as_atom()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_mul_add_resolves_to_px() {
        let expr = CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Binary {
                op: CalcBinOp::Mul,
                left: Box::new(CalcExpr::Number(2.0)),
                right: Box::new(CalcExpr::Percent(50.0)),
            }),
            right: Box::new(CalcExpr::Px(10.0)),
        };
        assert!(expr.is_length_typed());
        assert_eq!(
            expr.resolve_with_fonts(Some(200.0), None, FontSizeContext::default()),
            Some(210.0)
        );
        assert_eq!(
            expr.simplify_to_length_spec(),
            Some(LengthSpec::percent_offset(100.0, 10.0))
        );
    }

    #[test]
    fn length_times_length_is_not_typed() {
        let expr = CalcExpr::Binary {
            op: CalcBinOp::Mul,
            left: Box::new(CalcExpr::Px(2.0)),
            right: Box::new(CalcExpr::Px(3.0)),
        };
        assert!(!expr.is_length_typed());
        assert_eq!(
            expr.resolve_with_fonts(None, None, FontSizeContext::default()),
            None
        );
    }

    #[test]
    fn div_by_zero_does_not_resolve() {
        let expr = CalcExpr::Binary {
            op: CalcBinOp::Div,
            left: Box::new(CalcExpr::Px(10.0)),
            right: Box::new(CalcExpr::Number(0.0)),
        };
        assert_eq!(
            expr.resolve_with_fonts(None, None, FontSizeContext::default()),
            None
        );
    }

    #[test]
    fn interned_equal_trees_share_identity() {
        let a = intern_calc(Box::new(CalcExpr::Px(4.0)));
        let b = intern_calc(Box::new(CalcExpr::Px(4.0)));
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.inner(), b.inner()));
    }

    #[test]
    fn length_spec_calc_stays_copy() {
        fn assert_copy<T: Copy>(value: T) -> T {
            value
        }
        let spec = assert_copy(LengthSpec::from_calc(CalcExpr::Binary {
            op: CalcBinOp::Add,
            left: Box::new(CalcExpr::Min(
                Box::new(CalcExpr::Min(
                    Box::new(CalcExpr::Px(8.0)),
                    Box::new(CalcExpr::Px(20.0)),
                )),
                Box::new(CalcExpr::Px(12.0)),
            )),
            right: Box::new(CalcExpr::Px(4.0)),
        }));
        assert!(matches!(spec, LengthSpec::Calc(_)));
        assert_eq!(
            spec.resolve_with_fonts(None, None, FontSizeContext::default()),
            Some(12.0)
        );
    }
}
