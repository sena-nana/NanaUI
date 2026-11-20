//! Running the platform tools signing adapters wrap.
//!
//! Adapters never call `std::process` themselves: they describe a
//! [`ToolCommand`] and a [`ToolRunner`] runs it, so tests can check every
//! command line on any host with a recording runner. Arguments that carry a
//! credential are marked secret and never appear in a description or an
//! error; where a tool can take a credential from its environment or stdin
//! instead of argv, the adapter uses that.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zeroize::Zeroizing;

/// The operating system the packager runs on. A platform tool runs only on
/// its own host; anywhere else the adapter reports `not executed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Macos,
    Windows,
    Linux,
    Other,
}

impl Host {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Other => "other",
        }
    }
}

/// Where adapters read credentials and tool locations from: the CI
/// environment. Never `nana-package.toml`.
pub trait Environment {
    fn var(&self, name: &str) -> Option<String>;
}

/// The process environment.
pub struct ProcessEnv;

impl Environment for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
}

impl Environment for std::collections::BTreeMap<String, String> {
    fn var(&self, name: &str) -> Option<String> {
        self.get(name).filter(|value| !value.is_empty()).cloned()
    }
}

pub struct Arg {
    pub value: OsString,
    pub secret: bool,
}

pub struct ToolCommand {
    pub program: PathBuf,
    pub args: Vec<Arg>,
    /// Extra environment for the child (credentials a tool reads by name).
    pub envs: Vec<(String, Zeroizing<String>)>,
    /// Bytes written to the child's stdin (a passphrase).
    pub stdin: Option<Zeroizing<Vec<u8>>>,
}

impl ToolCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
            stdin: None,
        }
    }

    pub fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(Arg {
            value: value.into(),
            secret: false,
        });
        self
    }

    pub fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        for value in values {
            self = self.arg(value);
        }
        self
    }

    /// An argument that carries a credential.
    pub fn secret_arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(Arg {
            value: value.into(),
            secret: true,
        });
        self
    }

    pub fn env(mut self, name: &str, value: String) -> Self {
        self.envs.push((name.to_owned(), Zeroizing::new(value)));
        self
    }

    pub fn stdin(mut self, bytes: Vec<u8>) -> Self {
        self.stdin = Some(Zeroizing::new(bytes));
        self
    }

    /// The command line with secret arguments masked.
    pub fn describe(&self) -> String {
        let mut text = self.program.display().to_string();
        for arg in &self.args {
            text.push(' ');
            if arg.secret {
                text.push_str("***");
            } else {
                text.push_str(&arg.value.to_string_lossy());
            }
        }
        text
    }

    /// The non-secret arguments, for tests.
    pub fn visible_args(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| {
                if arg.secret {
                    "***".to_owned()
                } else {
                    arg.value.to_string_lossy().into_owned()
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait ToolRunner {
    fn run(&self, command: &ToolCommand) -> std::io::Result<ToolOutput>;
    /// `program` on `PATH`.
    fn find(&self, program: &str) -> Option<PathBuf>;
}

/// Runs the real tools.
pub struct SystemRunner;

impl ToolRunner for SystemRunner {
    fn run(&self, command: &ToolCommand) -> std::io::Result<ToolOutput> {
        let mut child = Command::new(&command.program);
        child
            .args(command.args.iter().map(|arg| &arg.value))
            .stdin(if command.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in &command.envs {
            child.env(name, value.as_str());
        }
        let mut child = child.spawn()?;
        if let (Some(bytes), Some(mut stdin)) = (&command.stdin, child.stdin.take()) {
            stdin.write_all(bytes)?;
        }
        let output = child.wait_with_output()?;
        Ok(ToolOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn find(&self, program: &str) -> Option<PathBuf> {
        find_on_path(program, std::env::var_os("PATH")?.as_os_str())
    }
}

pub(crate) fn find_on_path(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(program).extension().is_none() {
        vec![
            format!("{program}.exe"),
            format!("{program}.cmd"),
            format!("{program}.bat"),
        ]
    } else {
        vec![program.to_owned()]
    };
    std::env::split_paths(path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// A failed or unrunnable tool as a signing failure detail. The command is
/// described with its secrets masked; output is cut to its last 2 KB.
pub(crate) fn failure(command: &ToolCommand, result: &std::io::Result<ToolOutput>) -> String {
    match result {
        Err(error) => format!("cannot run `{}`: {error}", command.describe()),
        Ok(output) => {
            let mut text = output.stderr.trim().to_owned();
            if text.is_empty() {
                text = output.stdout.trim().to_owned();
            }
            if text.len() > 2048 {
                let mut cut = text.len() - 2048;
                while !text.is_char_boundary(cut) {
                    cut += 1;
                }
                text = format!("…{}", &text[cut..]);
            }
            format!(
                "`{}` exited with {}: {text}",
                command.describe(),
                output
                    .code
                    .map_or_else(|| "a signal".to_owned(), |code| code.to_string())
            )
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// One recorded call: program, visible arguments, extra environment
    /// names, and whether stdin was fed.
    pub(crate) type Call = (String, Vec<String>, Vec<String>, bool);

    /// Records every command and answers from a script: the first rule
    /// whose needle appears in the described command line decides.
    #[derive(Default)]
    pub(crate) struct Recorder {
        pub calls: RefCell<Vec<Call>>,
        pub rules: Vec<(String, ToolOutput)>,
        pub tools: BTreeMap<String, PathBuf>,
        /// Side effect: a file a tool would write (`--output`, `-outFile`).
        pub creates: RefCell<Vec<PathBuf>>,
    }

    impl Recorder {
        pub(crate) fn with_tools(names: &[&str]) -> Self {
            Self {
                tools: names
                    .iter()
                    .map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
                    .collect(),
                ..Self::default()
            }
        }

        pub(crate) fn answer(mut self, needle: &str, output: ToolOutput) -> Self {
            self.rules.push((needle.to_owned(), output));
            self
        }

        pub(crate) fn lines(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|(program, args, _, _)| format!("{program} {}", args.join(" ")))
                .collect()
        }
    }

    pub(crate) fn ok(stdout: &str) -> ToolOutput {
        ToolOutput {
            success: true,
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub(crate) fn fail(stderr: &str) -> ToolOutput {
        ToolOutput {
            success: false,
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    impl ToolRunner for Recorder {
        fn run(&self, command: &ToolCommand) -> std::io::Result<ToolOutput> {
            let line = command.describe();
            self.calls.borrow_mut().push((
                command.program.display().to_string(),
                command.visible_args(),
                command.envs.iter().map(|(name, _)| name.clone()).collect(),
                command.stdin.is_some(),
            ));
            // Tools that write an output file next to their input.
            let args = command.visible_args();
            for flag in ["--output", "-outFile"] {
                if let Some(at) = args.iter().position(|arg| arg == flag)
                    && let Some(path) = args.get(at + 1)
                {
                    let path = PathBuf::from(path);
                    let _ = std::fs::write(&path, b"signed");
                    self.creates.borrow_mut().push(path);
                }
            }
            Ok(self
                .rules
                .iter()
                .find(|(needle, _)| line.contains(needle.as_str()))
                .map_or_else(|| ok(""), |(_, output)| output.clone()))
        }

        fn find(&self, program: &str) -> Option<PathBuf> {
            self.tools.get(program).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_masked_in_descriptions_and_failures() {
        let command = ToolCommand::new("signtool")
            .arg("/p")
            .secret_arg("hunter2")
            .arg("App.exe");
        assert_eq!(command.describe(), "signtool /p *** App.exe");
        let detail = failure(&command, &Ok(testing::fail("bad password")));
        assert!(!detail.contains("hunter2"), "{detail}");
        assert!(detail.contains("bad password"), "{detail}");
    }

    #[test]
    fn the_real_runner_runs_a_tool_with_stdin_and_env() {
        // `sh` exists on every Unix CI host; Windows has no `sh` guarantee.
        if cfg!(windows) {
            return;
        }
        let sh = SystemRunner.find("sh").expect("sh on PATH");
        let output = SystemRunner
            .run(
                &ToolCommand::new(sh)
                    .arg("-c")
                    .arg("read line; printf '%s-%s' \"$line\" \"$NANA_TOOL_TEST\"")
                    .env("NANA_TOOL_TEST", "env".into())
                    .stdin(b"stdin\n".to_vec()),
            )
            .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, "stdin-env");
    }
}
