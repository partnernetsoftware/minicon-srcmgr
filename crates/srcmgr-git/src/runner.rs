//! Supervised execution of an installed Git (PRD 02 runner, PRD 11 environment).
//!
//! Every command is an argument vector: nothing goes through a shell. The child
//! runs inside MiniCon's owned containment, so a timeout or an output overrun
//! terminates the whole tree rather than only the root process.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use minicon_platform::contained_process::{
    ContainedChildOutput, ContainedHeadlessCommand, ProcessExit,
};

use crate::error::GitError;

/// Variables that could retarget a read to another repository, redirect its
/// object or index storage, inject configuration, or write traces. Removing
/// them keeps the selected worktree the only repository a command can see.
const REMOVED_ENVIRONMENT: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_EXTERNAL_DIFF",
    "GIT_TRACE",
    "GIT_TRACE2",
    "GIT_TRACE2_EVENT",
    "GIT_TRACE2_PERF",
    "GIT_TRACE_PACK_ACCESS",
    "GIT_TRACE_PERFORMANCE",
    "GIT_TRACE_SETUP",
];

/// Values this product always sets. Reads never prompt, never page, never take
/// optional index locks, and never depend on the user's display language.
const PINNED_ENVIRONMENT: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_PAGER", "cat"),
    ("LC_ALL", "C"),
];

const KILL_GRACE: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Hard bounds for one command. Exceeding any of them is a typed failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

impl Limits {
    /// Bounds for the version probe.
    pub const PROBE: Self = Self {
        timeout: Duration::from_secs(10),
        stdout_bytes: 4 * 1024,
        stderr_bytes: 4 * 1024,
    };

    /// Bounds for a rev-parse identity read: two path lines.
    pub const METADATA: Self = Self {
        timeout: Duration::from_secs(10),
        stdout_bytes: 64 * 1024,
        stderr_bytes: 4 * 1024,
    };

    /// Bounds for a mutation. Hooks may print, so stderr is allowed more room;
    /// the timeout covers hook execution.
    pub const MUTATION: Self = Self {
        timeout: Duration::from_secs(300),
        stdout_bytes: 1024 * 1024,
        stderr_bytes: 256 * 1024,
    };

    /// Bounds for one network transfer. A fetch may legitimately run longer
    /// than a local mutation, so the deadline is wider and both streams are
    /// allowed more room; cancellation stays available through run_network.
    pub const NETWORK: Self = Self {
        timeout: Duration::from_secs(900),
        stdout_bytes: 8 * 1024 * 1024,
        stderr_bytes: 4 * 1024 * 1024,
    };

    /// Bounds for a status read. Large repositories still fit; a runaway
    /// stream does not.
    pub const STATUS: Self = Self {
        timeout: Duration::from_secs(60),
        stdout_bytes: 32 * 1024 * 1024,
        stderr_bytes: 64 * 1024,
    };
}

/// The captured result of a successful Git command.
#[derive(Debug, Eq, PartialEq)]
pub struct Output {
    pub stdout: Vec<u8>,
}

/// Runs `executable` with `arguments` and returns its stdout on exit code 0.
///
/// The child inherits the environment except for the removed and pinned
/// variables above. `current_dir` is the worktree for repository commands and
/// `None` for the version probe.
pub(crate) fn run<S: AsRef<OsStr>>(
    executable: &Path,
    current_dir: Option<&Path>,
    arguments: &[S],
    input: Option<&str>,
    limits: Limits,
) -> Result<Output, GitError> {
    run_input(
        executable,
        current_dir,
        arguments,
        limits,
        RunOptions { text: input, bytes: None, cancel: None, sink: None },
    )
}

/// One progress chunk as Git printed it. The sink runs on the reader thread,
/// so it must not block; the caller filters and throttles before display.
pub type ProgressSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Runs one network command with a deadline, output bounds, cooperative
/// cancellation and optional stderr progress. Cancellation terminates the
/// owned tree through the same supervisor as a timeout.
pub(crate) fn run_network<S: AsRef<OsStr>>(
    executable: &Path,
    current_dir: Option<&Path>,
    arguments: &[S],
    cancel: &AtomicBool,
    sink: Option<&ProgressSink>,
) -> Result<Output, GitError> {
    run_input(
        executable,
        current_dir,
        arguments,
        Limits::NETWORK,
        RunOptions { text: None, bytes: None, cancel: Some(cancel), sink },
    )
}

/// Byte-safe stdin for NUL-delimited native paths, through the same owned
/// MiniCon pipe and timeout/output supervision as text commands.
pub(crate) fn run_bytes<S: AsRef<OsStr>>(
    executable:&Path,current_dir:Option<&Path>,arguments:&[S],input:&[u8],limits:Limits,
) -> Result<Output,GitError> {
    run_input(
        executable,
        current_dir,
        arguments,
        limits,
        RunOptions { text: None, bytes: Some(input), cancel: None, sink: None },
    )
}

/// Optional inputs and supervision for one command.
struct RunOptions<'a> {
    text: Option<&'a str>,
    bytes: Option<&'a [u8]>,
    cancel: Option<&'a AtomicBool>,
    sink: Option<&'a ProgressSink>,
}

fn run_input<S: AsRef<OsStr>>(
    executable: &Path,
    current_dir: Option<&Path>,
    arguments: &[S],
    limits: Limits,
    options: RunOptions<'_>,
) -> Result<Output, GitError> {
    let mut command = ContainedHeadlessCommand::new(executable);
    command
        .args(arguments.iter().map(|argument| argument.as_ref().to_os_string()))
        .capture_output();
    if let Some(text) = options.text {
        command.stdin_text(text);
    }
    if options.bytes.is_some(){command.pipe_stdin();}
    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }
    for key in REMOVED_ENVIRONMENT {
        command.env_remove(*key);
    }
    for (key, value) in PINNED_ENVIRONMENT {
        command.env(*key, *value);
    }

    let mut child = command.spawn().map_err(GitError::Spawn)?;
    let stdout = child.take_stdout().ok_or(GitError::Supervision)?;
    let stderr = child.take_stderr().ok_or(GitError::Supervision)?;
    let overrun = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(stdout, limits.stdout_bytes, Arc::clone(&overrun), None);
    let stderr_reader = spawn_reader(stderr, limits.stderr_bytes, Arc::clone(&overrun), options.sink.cloned());
    let input_writer=if let Some(bytes)=options.bytes {
        let mut writer=child.take_stdin().ok_or(GitError::Supervision)?;
        let bytes=bytes.to_vec();
        Some(thread::spawn(move||writer.write_all(&bytes)))
    }else{None};

    let deadline = Instant::now() + limits.timeout;
    let mut exit = None;
    let mut expired = false;
    let mut cancelled = false;
    loop {
        if let Some(status) = child.try_wait().map_err(GitError::Io)? {
            exit = Some(status);
            break;
        }
        expired = Instant::now() >= deadline;
        cancelled = options.cancel.is_some_and(|flag| flag.load(Ordering::Acquire));
        if expired || cancelled || overrun.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }

    // Terminating the owned tree also reaps detached descendants and closes
    // the pipes, so the readers below always reach end of stream.
    let cleanup = child.terminate_and_wait(KILL_GRACE);
    let (stdout, stdout_overrun) = join_reader(stdout_reader)?;
    let (stderr, stderr_overrun) = join_reader(stderr_reader)?;
    let input_result=input_writer.map(|writer|writer.join().map_err(|_|GitError::Supervision)).transpose()?;
    cleanup.map_err(GitError::Io)?;

    if cancelled && exit.is_none() {
        return Err(GitError::Cancelled);
    }
    if expired && exit.is_none() {
        return Err(GitError::Timeout);
    }
    if stdout_overrun || stderr_overrun {
        return Err(GitError::OutputLimit);
    }
    match exit {
        Some(ProcessExit::Code(0)) => {if let Some(result)=input_result {result.map_err(GitError::Io)?;}Ok(Output { stdout })},
        Some(ProcessExit::Code(code)) => Err(GitError::Exit { code, detail: first_error_line(&stderr) }),
        _ => Err(GitError::Terminated),
    }
}

/// The first line Git printed as an error, without its `error:` or `fatal:`
/// prefix and without control characters. Empty when there is none. Git's own
/// wording is shown to the user, so the text is cut to a readable length.
fn first_error_line(stderr: &[u8]) -> String {
    const LIMIT: usize = 160;
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let line = lines
        .iter()
        .find(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .or_else(|| lines.first())
        .map_or("", |line| line.trim_start_matches("error:").trim_start_matches("fatal:").trim());
    let clean: String = line.chars().map(|character| if character.is_control() { ' ' } else { character }).collect();
    if clean.chars().count() <= LIMIT {
        clean
    } else {
        let cut: String = clean.chars().take(LIMIT - 2).collect();
        format!("{cut}..")
    }
}

/// Drains one stream up to `limit` bytes. Reaching the limit raises the shared
/// overrun flag so the supervisor terminates the tree.
fn spawn_reader(
    mut stream: ContainedChildOutput,
    limit: usize,
    overrun: Arc<AtomicBool>,
    sink: Option<ProgressSink>,
) -> JoinHandle<io::Result<(Vec<u8>, bool)>> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            let read = stream.read(&mut chunk)?;
            if read == 0 {
                return Ok((captured, false));
            }
            if let Some(sink) = &sink {
                sink(&String::from_utf8_lossy(&chunk[..read]));
            }
            if captured.len() + read > limit {
                let room = limit - captured.len();
                captured.extend_from_slice(&chunk[..room]);
                overrun.store(true, Ordering::Release);
                return Ok((captured, true));
            }
            captured.extend_from_slice(&chunk[..read]);
        }
    })
}

fn join_reader(reader: JoinHandle<io::Result<(Vec<u8>, bool)>>) -> Result<(Vec<u8>, bool), GitError> {
    reader.join().map_err(|_| GitError::Supervision)?.map_err(GitError::Io)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn the_first_git_error_line_is_kept_without_its_prefix_and_cut_to_a_readable_length() {
        let text = b"hint: try something\nerror: Your local changes would be overwritten\n\ta.txt\n";
        assert_eq!(first_error_line(text), "Your local changes would be overwritten");
        assert_eq!(first_error_line(b""), "");
        assert_eq!(first_error_line(b"fatal: bad\x07object\n"), "bad object");
        let long = "x".repeat(300);
        assert_eq!(first_error_line(long.as_bytes()).chars().count(), 160);
    }

    fn git_executable() -> PathBuf {
        env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
    }

    #[test]
    fn output_bound_terminates_and_reports_overrun() {
        let limits = Limits {
            timeout: Duration::from_secs(30),
            stdout_bytes: 8,
            stderr_bytes: 4 * 1024,
        };
        assert!(matches!(
            run(&git_executable(), None, &["--version"], None, limits),
            Err(GitError::OutputLimit)
        ));
    }

    #[test]
    fn deadline_terminates_and_reports_timeout() {
        // A real `git --version` may finish before the first deadline poll,
        // especially on Linux. Use this test executable as an owned blocking
        // child instead; the marker is carried through libtest's --skip value
        // without changing process-global environment or skipping this test.
        const CHILD_MARKER: &str = "srcmgr-owned-deadline-fixture";
        if env::args().any(|argument| argument == CHILD_MARKER) {
            thread::sleep(Duration::from_secs(2));
            return;
        }
        let limits = Limits {
            timeout: Duration::from_millis(20),
            ..Limits::PROBE
        };
        assert!(matches!(
            run(&env::current_exe().expect("owned test executable"), None,
                &["--exact", "runner::tests::deadline_terminates_and_reports_timeout", "--skip", CHILD_MARKER], None, limits),
            Err(GitError::Timeout)
        ));
    }

    #[test]
    fn successful_probe_returns_its_captured_stdout() {
        let output = run(&git_executable(), None, &["--version"], None, Limits::PROBE)
            .expect("git --version");
        assert!(output.stdout.starts_with(b"git version "));
    }

    #[test]
    fn cancellation_terminates_the_owned_tree_and_reports_it() {
        const CHILD_MARKER: &str = "srcmgr-owned-cancel-fixture";
        if env::args().any(|argument| argument == CHILD_MARKER) {
            thread::sleep(Duration::from_secs(5));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancel);
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            trigger.store(true, Ordering::Release);
        });
        let result = run_network(
            &env::current_exe().expect("owned test executable"),
            None,
            &[
                "--exact",
                "runner::tests::cancellation_terminates_the_owned_tree_and_reports_it",
                "--skip",
                CHILD_MARKER,
            ],
            &cancel,
            None,
        );
        trigger.join().expect("cancel trigger");
        assert!(matches!(result, Err(GitError::Cancelled)), "got {result:?}");
    }
}
