//! Remote configuration and observation (PRD 07, PRD 11).
//!
//! Only machine formats are read: `git remote -v`, `git config --get-all` and
//! `git fetch --porcelain`. Credential-bearing URL parts are removed before a
//! value can reach a label, and the fetch result names the exact refs Git
//! reported rather than claiming a broader success.

use std::sync::atomic::AtomicBool;

use srcmgr_domain::NativePath;

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits, ProgressSink};

/// One configured remote. URLs are already redacted for display; the raw
/// values are never retained by this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSummary {
    pub name: String,
    pub fetch_url: String,
    pub push_url: Option<String>,
    pub fetch_refspecs: Vec<String>,
}

/// The current branch's upstream and its distance from the fetched ref.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackingState {
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

impl TrackingState {
    const fn none() -> Self {
        Self { upstream: None, ahead: 0, behind: 0 }
    }
}

/// One ref Git reported after a fetch. `old` and `new` are opaque object ids;
/// an all-zero value marks creation or deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchUpdate {
    pub old: String,
    pub new: String,
    pub reference: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchReport {
    pub remote: String,
    pub updates: Vec<FetchUpdate>,
}

/// Lists the configured remotes in Git's own order, with redacted URLs and
/// their fetch refspecs.
pub fn list(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<Vec<RemoteSummary>, GitError> {
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["remote", "-v"],
        None,
        Limits::METADATA,
    )?;
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::UnrepresentableName)?;
    let mut remotes: Vec<RemoteSummary> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [name, url_and_kind] = fields.as_slice() else {
            return Err(GitError::MalformedOutput { record: index });
        };
        if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_control) {
            return Err(GitError::InvalidRemote);
        }
        let (url, kind) = url_and_kind.rsplit_once(' ').ok_or(GitError::MalformedOutput { record: index })?;
        let label = label(url);
        match remotes.iter_mut().find(|remote| remote.name == *name) {
            Some(remote) => {
                if kind == "(push)" && label != remote.fetch_url {
                    remote.push_url = Some(label);
                }
            }
            None => {
                if kind != "(fetch)" {
                    return Err(GitError::MalformedOutput { record: index });
                }
                remotes.push(RemoteSummary {
                    name: (*name).to_owned(),
                    fetch_url: label.clone(),
                    push_url: None,
                    fetch_refspecs: refspecs(git, repo, name)?,
                });
            }
        }
    }
    Ok(remotes)
}

fn refspecs(git: &GitInstallation, repo: &RepositoryIdentity, name: &str) -> Result<Vec<String>, GitError> {
    let key = format!("remote.{name}.fetch");
    match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["config", "--get-all", &key],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()),
        // Git exits 1 for a missing key; that is an empty list, not a failure.
        Err(GitError::Exit { code: 1, .. }) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Removes credentials from a URL before it is displayed. A password or token
/// after `user:` is dropped, query and fragment parts are dropped, and any
/// remaining text is escaped for display.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let without_suffix = url.split(['?', '#']).next().unwrap_or_default();
    let Some(scheme_end) = without_suffix.find("://") else {
        return without_suffix.to_owned();
    };
    let head = &without_suffix[..scheme_end + 3];
    let rest = &without_suffix[scheme_end + 3..];
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let Some(at) = authority.rfind('@') else {
        return format!("{head}{authority}{path}");
    };
    let userinfo = &authority[..at];
    let host = &authority[at + 1..];
    match userinfo.split_once(':') {
        Some((user, _secret)) => format!("{head}{user}@{host}{path}"),
        None => format!("{head}{authority}{path}"),
    }
}

fn label(url: &str) -> String {
    NativePath::new(redact_url(url)).display_label()
}

/// Reads the current branch's upstream and how far it is from the fetched
/// tracking ref. A missing upstream, detached HEAD or unborn branch reports
/// `None` instead of inventing a comparison.
pub fn tracking(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<TrackingState, GitError> {
    let upstream = match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8(output.stdout)
            .map_err(|_| GitError::UnrepresentableName)?
            .trim()
            .to_owned(),
        Err(GitError::Exit { .. }) => return Ok(TrackingState::none()),
        Err(error) => return Err(error),
    };
    if upstream.is_empty() {
        return Ok(TrackingState::none());
    }
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        None,
        Limits::METADATA,
    )?;
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    let mut counts = text.split_whitespace();
    let behind = counts.next().and_then(|value| value.parse::<u32>().ok());
    let ahead = counts.next().and_then(|value| value.parse::<u32>().ok());
    let (Some(behind), Some(ahead)) = (behind, ahead) else {
        return Err(GitError::MalformedOutput { record: 0 });
    };
    Ok(TrackingState { upstream: Some(NativePath::new(&upstream).display_label()), ahead, behind })
}

/// Fetches one configured remote and returns exactly the refs Git reported.
/// Progress is filtered to transfer lines and never carries the remote URL.
pub fn fetch(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    remote: &str,
    cancel: &AtomicBool,
    sink: Option<&ProgressSink>,
) -> Result<FetchReport, GitError> {
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(char::is_control) {
        return Err(GitError::InvalidRemote);
    }
    let known = list(git, repo)?;
    if !known.iter().any(|entry| entry.name == remote) {
        return Err(GitError::UnknownRemote);
    }
    if mirror(git, repo, remote)? {
        return Err(GitError::RemotePolicy(
            "this remote is configured as a mirror, which would update refs outside the reviewed fetch scope; change the remote configuration first",
        ));
    }
    let output = runner::run_network(
        git.executable(),
        Some(&repo.worktree_root),
        &[
            "fetch",
            "--porcelain",
            "--progress",
            "--no-prune",
            "--no-recurse-submodules",
            remote,
        ],
        cancel,
        sink,
    )?;
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    let mut updates = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (flag, old, new, reference) = match fields.as_slice() {
            [old, new, reference] => (" ", *old, *new, *reference),
            [flag, old, new, reference] => (*flag, *old, *new, *reference),
            _ => return Err(GitError::MalformedOutput { record: index }),
        };
        if !reference.starts_with("refs/") || !is_object_id(old) || !is_object_id(new) {
            return Err(GitError::MalformedOutput { record: index });
        }
        // '=' means the remote ref is unchanged; it is not an update.
        if flag == "=" {
            continue;
        }
        updates.push(FetchUpdate {
            old: old.to_owned(),
            new: new.to_owned(),
            reference: reference.to_owned(),
        });
    }
    Ok(FetchReport { remote: remote.to_owned(), updates })
}

fn mirror(git: &GitInstallation, repo: &RepositoryIdentity, name: &str) -> Result<bool, GitError> {
    let key = format!("remote.{name}.mirror");
    match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["config", "--bool", "--get", &key],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => Ok(String::from_utf8_lossy(&output.stdout).trim() == "true"),
        Err(GitError::Exit { code: 1, .. }) => Ok(false),
        Err(error) => Err(error),
    }
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Keeps only transfer-progress lines, so remote banners and URLs never reach
/// the window.
#[must_use]
pub fn progress_line(chunk: &str) -> Option<String> {
    const PREFIXES: [&str; 6] = [
        "Receiving objects:",
        "Resolving deltas:",
        "Unpacking objects:",
        "Counting objects:",
        "Compressing objects:",
        "remote: Counting objects:",
    ];
    chunk
        .split(['\r', '\n'])
        .map(str::trim)
        .find(|line| PREFIXES.iter().any(|prefix| line.starts_with(prefix)))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{progress_line, redact_url};

    #[test]
    fn a_password_is_removed_but_the_user_and_host_remain() {
        assert_eq!(redact_url("https://alice:s3cret@example.test/repo.git"), "https://alice@example.test/repo.git");
        assert_eq!(redact_url("ssh://git@example.test:22/repo.git"), "ssh://git@example.test:22/repo.git");
    }

    #[test]
    fn query_parameters_are_removed() {
        assert_eq!(redact_url("https://example.test/repo.git?private_token=abc"), "https://example.test/repo.git");
        assert_eq!(redact_url("https://example.test/repo.git#frag"), "https://example.test/repo.git");
    }

    #[test]
    fn scp_syntax_and_plain_paths_are_kept() {
        assert_eq!(redact_url("git@example.test:team/repo.git"), "git@example.test:team/repo.git");
        assert_eq!(redact_url("D:/work/repo.git"), "D:/work/repo.git");
    }

    #[test]
    fn only_transfer_lines_become_progress() {
        assert_eq!(progress_line("Receiving objects: 42% (21/50)"), Some("Receiving objects: 42% (21/50)".to_owned()));
        assert_eq!(progress_line("From https://alice:token@example.test/repo"), None);
    }
}
