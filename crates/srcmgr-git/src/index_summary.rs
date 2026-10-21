use std::ffi::{OsStr, OsString};

use crate::{GitError, GitInstallation, RepositoryIdentity, runner};

/// Observed index-to-HEAD delta. Binary line counts are unknown, never zero.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexSummary {
    pub files: usize,
    pub added: u64,
    pub deleted: u64,
    pub binary_files: usize,
}

impl IndexSummary {
    pub fn label(&self, scope: &str) -> String {
        let lines = if self.binary_files == 0 {
            format!("+{}/-{}", self.added, self.deleted)
        } else {
            format!(
                "+{}/-{} text; {} binary (lines unknown)",
                self.added, self.deleted, self.binary_files
            )
        };
        let unit = if self.files == 1 { "file" } else { "files" };
        format!("Index vs HEAD [{scope}]: {} {unit} {lines}; worktree writes not requested", self.files)
    }
}

/// Reads a literal path's current staged delta, or the whole index. Omitting
/// an explicit HEAD lets Git use the empty tree for an unborn repository.
pub fn index_summary(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    path: Option<&OsStr>,
) -> Result<IndexSummary, GitError> {
    let mut args: Vec<OsString> = [
        "--literal-pathspecs",
        "diff",
        "--cached",
        "--numstat",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.push("--".into());
    if let Some(path) = path {
        args.push(path.to_owned());
    }
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &args,
        None,
        runner::Limits::STATUS,
    )?;
    parse(&output.stdout)
}

fn parse(bytes: &[u8]) -> Result<IndexSummary, GitError> {
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    let mut fields = bytes.split(|b| *b == 0).peekable();
    let mut result = IndexSummary::default();
    while let Some(field) = fields.next() {
        if field.is_empty() && fields.peek().is_none() {
            break;
        }
        let bad = || GitError::MalformedOutput {
            record: result.files,
        };
        let mut parts = field.splitn(3, |b| *b == b'\t');
        let add = parts.next().ok_or_else(bad)?;
        let del = parts.next().ok_or_else(bad)?;
        let path = parts.next().ok_or_else(bad)?;
        if path.is_empty() {
            // A rename uses an empty path field followed by two NUL paths.
            if fields.next().is_none_or(|p| p.is_empty())
                || fields.next().is_none_or(|p| p.is_empty())
            {
                return Err(bad());
            }
        }
        if add == b"-" && del == b"-" {
            result.binary_files += 1;
        } else {
            let count = |value: &[u8]| {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
            };
            result.added = result
                .added
                .checked_add(count(add).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            result.deleted = result
                .deleted
                .checked_add(count(del).ok_or_else(bad)?)
                .ok_or_else(bad)?;
        }
        result.files += 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_paths_and_rename_records_do_not_become_extra_files() {
        let summary = parse(b"2\t1\t\0old\tname\0new\nname\0-\t-\t\xff\0").unwrap();
        assert_eq!(
            summary,
            IndexSummary {
                files: 2,
                added: 2,
                deleted: 1,
                binary_files: 1
            }
        );
        assert!(summary.label("all").contains("lines unknown"));
        assert!(parse(b"2\t1\tx").is_err());
        assert!(parse(b"-\t1\tx\0").is_err());
    }
}
