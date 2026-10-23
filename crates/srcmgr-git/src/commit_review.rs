//! Complete staged-index review and observed ordinary-commit outcomes.
//! The live index remains Git's input; preflight is not snapshot isolation.
use std::collections::BTreeMap;
use std::ffi::OsString;
use srcmgr_domain::NativePath;
use crate::{GitError, GitInstallation, RepositoryIdentity, runner};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexBinding {
    pub head: Option<String>,
    pub reference: Option<String>,
    pub merge_parents: Vec<String>,
    raw: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedFile {
    pub path: NativePath,
    pub change: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedReview {
    pub binding: IndexBinding,
    pub files: Vec<StagedFile>,
    pub blocked: Option<String>,
}

#[derive(Debug)]
pub enum ReviewedCommit {
    Observed { oid: String, warning: Option<String> },
    Unknown(String),
}

fn run(git: &GitInstallation, repo: &RepositoryIdentity, args: &[OsString]) -> Result<Vec<u8>, GitError> {
    runner::run(git.executable(), Some(&repo.worktree_root), args, None, runner::Limits::STATUS).map(|output| output.stdout)
}

fn optional_ref(git: &GitInstallation, repo: &RepositoryIdentity, args: &[&str]) -> Result<Option<String>, GitError> {
    let args: Vec<_> = args.iter().map(OsString::from).collect();
    match run(git, repo, &args) {
        Ok(bytes) => Ok(Some(std::str::from_utf8(&bytes).map_err(|_| GitError::UnrepresentableName)?.trim().to_owned())),
        Err(GitError::Exit { code: 1, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

const DIFF_FLAGS: &[&str] = &["--raw", "-z", "--no-abbrev", "--no-renames", "--no-ext-diff", "--no-textconv", "--no-color", "--ignore-submodules=none"];

pub fn index_binding(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<IndexBinding, GitError> {
    git.require_supported()?;
    let head = optional_ref(git, repo, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    if let Some(head) = &head { crate::status::revision(head.as_bytes())?; }
    let reference = optional_ref(git, repo, &["symbolic-ref", "-q", "HEAD"])?;
    let mut args = vec!["diff".into(), "--cached".into()];
    args.extend(DIFF_FLAGS.iter().map(OsString::from));
    args.push("--".into());
    let raw = run(git, repo, &args)?;
    // This adapter currently admits plain .git directories only; the caller
    // verifies common_dir equals that reviewed directory before Git work.
    let merge_path = repo.common_dir.join("MERGE_HEAD");
    let mut merge_parents = Vec::new();
    match std::fs::symlink_metadata(&merge_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(GitError::Io(error)),
        Ok(metadata) => {
            use std::io::Read;
            if !metadata.is_file() || minicon_platform::filesystem::metadata_is_link_like(&metadata) || metadata.len() > 65536 { return Err(GitError::StaleCommitReview); }
            let mut bytes = Vec::new();
            std::fs::File::open(merge_path).map_err(GitError::Io)?.take(65537).read_to_end(&mut bytes).map_err(GitError::Io)?;
            if bytes.len() > 65536 { return Err(GitError::StaleCommitReview); }
            for oid in std::str::from_utf8(&bytes).map_err(|_| bad())?.split_whitespace() {
                crate::status::revision(oid.as_bytes())?;
                merge_parents.push(oid.to_ascii_lowercase());
            }
        }
    }
    Ok(IndexBinding { head, reference, merge_parents, raw })
}

impl StagedReview {
    pub fn read(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<Self, GitError> {
        let binding = index_binding(git, repo)?;
        if nul_rows(&binding.raw)?.iter().any(|row| row.starts_with(b":") && row.ends_with(b" U")) {
            return Ok(Self { binding, files: Vec::new(), blocked: Some("Unresolved index conflicts; resolve them before committing".into()) });
        }
        let args: Vec<_> = ["diff", "--cached", "--numstat", "-z", "--no-renames", "--no-ext-diff", "--no-textconv", "--no-color", "--ignore-submodules=none", "--"].into_iter().map(OsString::from).collect();
        let counts = run(git, repo, &args)?;
        let mut counts_by_path = BTreeMap::new();
        for row in nul_rows(&counts)? {
            let fields: Vec<_> = row.splitn(3, |byte| *byte == b'\t').collect();
            if fields.len() != 3 || fields[2].is_empty() { return Err(bad()); }
            let count = |bytes: &[u8]| -> Result<Option<u64>, GitError> {
                if bytes == b"-" { Ok(None) } else { std::str::from_utf8(bytes).ok().and_then(|text| text.parse().ok()).map(Some).ok_or_else(bad) }
            };
            if counts_by_path.insert(fields[2].to_vec(), (count(fields[0])?, count(fields[1])?)).is_some() { return Err(bad()); }
        }
        let rows = nul_rows(&binding.raw)?;
        if !rows.len().is_multiple_of(2) { return Err(bad()); }
        let mut files = Vec::new();
        let mut conflict = false;
        for pair in rows.chunks_exact(2) {
            let header = std::str::from_utf8(pair[0]).map_err(|_| bad())?;
            let fields: Vec<_> = header.strip_prefix(':').ok_or_else(bad)?.split(' ').collect();
            if fields.len() != 5 || fields[..2].iter().any(|mode| mode.len() != 6 || !mode.bytes().all(|byte| matches!(byte, b'0'..=b'7'))) { return Err(bad()); }
            crate::status::revision(fields[2].as_bytes())?;
            crate::status::revision(fields[3].as_bytes())?;
            if !matches!(fields[4], "A" | "D" | "M" | "T" | "U") { return Err(bad()); }
            conflict |= fields[4] == "U";
            let (added, deleted) = counts_by_path.remove(pair[1]).ok_or_else(bad)?;
            files.push(StagedFile { path: NativePath::from_tool_bytes(pair[1]).map_err(|_| GitError::UnrepresentableName)?, change: fields[4].into(), added, deleted });
        }
        if !counts_by_path.is_empty() || index_binding(git, repo)? != binding { return Err(GitError::StaleCommitReview); }
        let blocked = if conflict { Some("Unresolved index conflicts; resolve them before committing".into()) } else if files.is_empty() { Some("Nothing is staged to commit".into()) } else { None };
        Ok(Self { binding, files, blocked })
    }

    pub fn diff(&self, git: &GitInstallation, repo: &RepositoryIdentity, index: usize) -> Result<String, GitError> {
        let file = self.files.get(index).ok_or(GitError::StaleCommitReview)?;
        if index_binding(git, repo)? != self.binding { return Err(GitError::StaleCommitReview); }
        let diff = crate::diff(git, repo, file.path.as_os_str(), true)?;
        if index_binding(git, repo)? != self.binding { return Err(GitError::StaleCommitReview); }
        Ok(diff)
    }

    pub fn commit(self, git: &GitInstallation, repo: &RepositoryIdentity, message: &str) -> Result<ReviewedCommit, GitError> {
        if self.blocked.is_some() || message.trim().is_empty() || index_binding(git, repo)? != self.binding { return Err(GitError::StaleCommitReview); }
        let result = runner::run(git.executable(), Some(&repo.worktree_root), &["commit", "--cleanup=strip", "-F", "-"], Some(message), runner::Limits::MUTATION);
        let observed = match optional_ref(git, repo, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(head) => head,
            Err(error) => return Ok(ReviewedCommit::Unknown(format!("Commit outcome unknown; HEAD could not be read: {error}. Inspect before retrying."))),
        };
        if observed == self.binding.head {
            return match result {
                Err(error) => Err(error),
                Ok(_) => Ok(ReviewedCommit::Unknown("Git returned success but no new HEAD was observed. Inspect before retrying.".into())),
            };
        }
        let Some(oid) = observed else { return Ok(ReviewedCommit::Unknown("HEAD disappeared during commit; inspect before retrying".into())); };
        if crate::status::revision(oid.as_bytes()).is_err() { return Ok(ReviewedCommit::Unknown("Commit outcome unknown; observed HEAD was not a valid object identity".into())); }
        let mut warnings = Vec::new();
        if let Err(error) = result { warnings.push(format!("Git reported an error, but HEAD changed: {error}")); }
        let mut args: Vec<OsString> = if let Some(before) = &self.binding.head { vec!["diff".into(), before.into(), oid.clone().into()] } else { vec!["diff-tree".into(), "--root".into(), "--no-commit-id".into(), "-r".into(), oid.clone().into()] };
        args.extend(DIFF_FLAGS.iter().map(OsString::from));
        args.push("--".into());
        match run(git, repo, &args) {
            Ok(raw) if raw == self.binding.raw => {}
            Ok(_) => warnings.push("Observed commit tree differs from the reviewed index; inspect the commit. No reset was requested.".into()),
            Err(error) => warnings.push(format!("Observed commit tree comparison unavailable: {error}")),
        }
        let parent_args = ["rev-list", "--parents", "-n", "1", &oid, "--"].map(OsString::from);
        match run(git, repo, &parent_args) {
            Ok(bytes) => {
                match std::str::from_utf8(&bytes) {
                    Ok(parents) if parents.split_whitespace().skip(1).eq(self.binding.head.iter().chain(self.binding.merge_parents.iter()).map(String::as_str)) => {}
                    Ok(_) => warnings.push("Observed commit parent differs from the reviewed HEAD; inspect history.".into()),
                    Err(_) => warnings.push("Observed commit parent could not be decoded; inspect history.".into()),
                }
            }
            Err(error) => warnings.push(format!("Observed commit parent unavailable: {error}")),
        }
        match optional_ref(git, repo, &["symbolic-ref", "-q", "HEAD"]) {
            Ok(reference) if reference == self.binding.reference => {}
            _ => warnings.push("HEAD reference changed or could not be verified after commit; inspect the destination.".into()),
        }
        Ok(ReviewedCommit::Observed { oid, warning: (!warnings.is_empty()).then(|| warnings.join("; ")) })
    }
}

fn nul_rows(bytes: &[u8]) -> Result<Vec<&[u8]>, GitError> {
    if bytes.is_empty() { return Ok(Vec::new()); }
    if !bytes.ends_with(&[0]) { return Err(bad()); }
    Ok(bytes[..bytes.len() - 1].split(|byte| *byte == 0).collect())
}

fn bad() -> GitError { GitError::MalformedOutput { record: 0 } }
