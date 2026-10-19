use std::fmt;
use std::io;

use minicon_core::version::Version;

/// A typed Git adapter failure. Messages never carry raw stderr, credential
/// bearing URLs or private path text (PRD 11 diagnostics); callers get the
/// category and, where useful, the exit code.
#[derive(Debug)]
pub enum GitError {
    /// The executable could not be started.
    Spawn(io::Error),
    /// Supervision of the started child failed (pipes or containment).
    Io(io::Error),
    /// The owned child did not expose its output streams for capture.
    Supervision,
    /// The command exceeded its deadline and its tree was terminated.
    Timeout,
    /// A captured stream exceeded its byte bound and was terminated.
    OutputLimit,
    /// Git exited unsuccessfully.
    Exit { code: i32 },
    /// Git ended by a signal or without a reportable status.
    Terminated,
    /// The installed Git is older than the product floor (PRD 02).
    UnsupportedVersion { found: Version, minimum: Version },
    /// The version text did not start with a recognizable `X.Y.Z`.
    UnrecognizedVersion,
    /// Plumbing output did not match the documented format. The offending
    /// record index is reported; its contents are not.
    MalformedOutput { record: usize },
    /// The status report omitted the `# branch.oid` or `# branch.head` header
    /// that `--branch` guarantees.
    MissingBranchHeader,
    /// A path or branch name cannot be represented on this platform.
    UnrepresentableName,
    /// The name is not a valid branch name under Git's rules.
    InvalidName,
    /// The worktree has no `.git` entry that this slice can read directly.
    NotARepository,
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(_) => formatter.write_str("the configured Git executable could not be started"),
            Self::Io(_) => formatter.write_str("supervising the Git process failed"),
            Self::Supervision => formatter.write_str("the Git process did not expose captured output"),
            Self::Timeout => formatter.write_str("Git did not finish before the deadline; it was terminated"),
            Self::OutputLimit => formatter.write_str("Git produced more output than allowed; it was terminated"),
            Self::Exit { code } => write!(formatter, "Git exited with status {code}"),
            Self::Terminated => formatter.write_str("Git ended without a success status"),
            Self::UnsupportedVersion { found, minimum } => write!(
                formatter,
                "installed Git {found} is older than the required {minimum}"
            ),
            Self::UnrecognizedVersion => formatter.write_str("the Git version report was not recognized"),
            Self::MissingBranchHeader => formatter.write_str("the Git status report had no branch header"),
            Self::MalformedOutput { record } => {
                write!(formatter, "Git status output record {record} did not match the expected format")
            }
            Self::InvalidName => formatter.write_str("the branch name is not valid"),
            Self::UnrepresentableName => {
                formatter.write_str("a path or branch name cannot be represented on this platform")
            }
            Self::NotARepository => formatter.write_str("no readable .git directory was found at this path"),
        }
    }
}

impl std::error::Error for GitError {}
