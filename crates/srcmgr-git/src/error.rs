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
    /// Git exited unsuccessfully. `detail` is the first error line Git printed,
    /// empty when it printed none.
    Exit { code: i32, detail: String },
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
    /// The revision is not a hexadecimal object id.
    InvalidRevision,
    /// The diff has no text hunks (for example a binary change or no change).
    NoHunks,
    /// The requested hunk index is outside the diff's hunks.
    HunkOutOfRange,
    /// The selection holds no changed line, so there is nothing to stage.
    NoChangedLines,
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
            Self::Exit { code, detail } if detail.is_empty() => write!(formatter, "Git exited with status {code}"),
            Self::Exit { code, detail } => write!(formatter, "{detail} (Git exited with status {code})"),
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
            Self::InvalidRevision => formatter.write_str("the commit id is not valid"),
            Self::NoHunks => formatter.write_str("this change has no text hunks to stage"),
            Self::HunkOutOfRange => formatter.write_str("no hunk has that number"),
            Self::NoChangedLines => formatter.write_str("select at least one changed line"),
            Self::UnrepresentableName => {
                formatter.write_str("a path or branch name cannot be represented on this platform")
            }
            Self::NotARepository => formatter.write_str("no readable .git directory was found at this path"),
        }
    }
}

impl std::error::Error for GitError {}
