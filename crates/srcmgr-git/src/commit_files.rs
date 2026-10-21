use crate::{GitError, GitInstallation, RepositoryIdentity, runner};
use srcmgr_domain::NativePath;
use std::ffi::OsString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitFile {
    pub status: String,
    pub old_mode: String,
    pub new_mode: String,
    pub path: NativePath,
    pub old_path: Option<NativePath>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitFiles {
    pub id: String,
    pub parent: Option<String>,
    pub merge: bool,
    pub files: Vec<CommitFile>,
    pub truncated: bool,
}

fn run(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    args: &[OsString],
) -> Result<Vec<u8>, GitError> {
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        args,
        None,
        runner::Limits::MUTATION,
    )
    .map(|o| o.stdout)
}

fn base(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    id: &str,
) -> Result<(Option<String>, bool), GitError> {
    crate::status::revision(id.as_bytes())?;
    let parents = run(
        git,
        repo,
        &[
            "rev-list".into(),
            "--parents".into(),
            "-n".into(),
            "1".into(),
            id.into(),
            "--".into(),
        ],
    )?;
    let text = std::str::from_utf8(&parents).map_err(|_| GitError::InvalidRevision)?;
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.first() != Some(&id) {
        return Err(GitError::InvalidRevision);
    }
    Ok((parts.get(1).map(|s| (*s).to_owned()), parts.len() > 2))
}

pub fn commit_files(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    id: &str,
) -> Result<CommitFiles, GitError> {
    let (parent, merge) = base(git, repo, id)?;
    let mut args: Vec<OsString> = if let Some(parent) = &parent {
        ["diff", "--raw", "--no-abbrev", "-z", "-M", parent, id, "--"]
            .into_iter()
            .map(Into::into)
            .collect()
    } else {
        [
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--raw",
            "--no-abbrev",
            "-z",
            "-M",
            id,
            "--",
        ]
        .into_iter()
        .map(Into::into)
        .collect()
    };
    args.insert(0, "--literal-pathspecs".into());
    let raw = run(git, repo, &args)?;
    if !raw.is_empty() && !raw.ends_with(&[0]) {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    let mut records = raw.split(|b| *b == 0).peekable();
    let mut files = Vec::new();
    let mut truncated = false;
    while let Some(header) = records.next() {
        if header.is_empty() && records.peek().is_none() {
            break;
        }
        let bad = || GitError::MalformedOutput {
            record: files.len(),
        };
        let header = std::str::from_utf8(header).map_err(|_| bad())?;
        let fields: Vec<_> = header
            .strip_prefix(':')
            .ok_or_else(bad)?
            .split(' ')
            .collect();
        if fields.len() != 5
            || fields[..2]
                .iter()
                .any(|mode| mode.len() != 6 || !mode.bytes().all(|b| matches!(b, b'0'..=b'7')))
        {
            return Err(bad());
        }
        crate::status::revision(fields[2].as_bytes())?;
        crate::status::revision(fields[3].as_bytes())?;
        let status = fields[4];
        if status.is_empty() || !status.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(bad());
        }
        let first = records.next().filter(|p| !p.is_empty()).ok_or_else(bad)?;
        let (path, old) = if status.starts_with(['R', 'C']) {
            (
                records.next().filter(|p| !p.is_empty()).ok_or_else(bad)?,
                Some(first),
            )
        } else {
            (first, None)
        };
        if files.len() == 2000 {
            truncated = true;
            continue;
        }
        files.push(CommitFile {
            status: status.to_owned(),
            old_mode: fields[0].to_owned(),
            new_mode: fields[1].to_owned(),
            path: NativePath::from_tool_bytes(path).map_err(|_| GitError::UnrepresentableName)?,
            old_path: old
                .map(NativePath::from_tool_bytes)
                .transpose()
                .map_err(|_| GitError::UnrepresentableName)?,
        });
    }
    Ok(CommitFiles {
        id: id.to_owned(),
        parent,
        merge,
        files,
        truncated,
    })
}

pub fn commit_file_diff(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    id: &str,
    file: &CommitFile,
) -> Result<String, GitError> {
    let (parent, _) = base(git, repo, id)?;
    let mut args: Vec<OsString> = [
        "--literal-pathspecs",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-M",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if let Some(parent) = parent {
        args.extend([parent.into(), id.into()]);
    } else {
        args = [
            "--literal-pathspecs",
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "-p",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            id,
        ]
        .into_iter()
        .map(Into::into)
        .collect();
    }
    args.push("--".into());
    if let Some(old) = &file.old_path {
        args.push(old.as_os_str().to_owned());
    }
    args.push(file.path.as_os_str().to_owned());
    Ok(String::from_utf8_lossy(&run(git, repo, &args)?).into_owned())
}
