//! Reviewed remote configuration writes (PRD 07, PRD 11).
//!
//! A change is validated from the reviewed input, previewed as the exact local
//! effect and only then applied with Git. No command here contacts a server, and
//! a configuration write invalidates the repository trust fingerprint, so the
//! next repository action asks for a fresh review.

use srcmgr_domain::NativePath;

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::remotes::redact_url;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits};

/// One reviewed local change to the remote configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteChange {
    Add { name: String, url: String, push_url: Option<String> },
    SetUrl { name: String, url: String, push: bool },
    Rename { from: String, to: String },
    Remove { name: String },
}

/// What the user sees before approving: the action and the local values it
/// writes, with credentials already removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemotePreview {
    pub lines: Vec<String>,
}

impl RemoteChange {
    fn name(&self) -> &str {
        match self {
            Self::Add { name, .. } | Self::SetUrl { name, .. } | Self::Remove { name } => name,
            Self::Rename { from, .. } => from,
        }
    }
}

/// Builds the preview and refuses invalid names or transports before Git runs.
pub fn preview(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    change: &RemoteChange,
) -> Result<RemotePreview, GitError> {
    validate_change(git, repo, change)?;
    let existing = crate::remotes::list(git, repo)?;
    let find = |name: &str| existing.iter().find(|remote| remote.name == name);
    let mut lines = Vec::new();
    match change {
        RemoteChange::Add { name, url, push_url } => {
            if find(name).is_some() {
                return Err(GitError::RemotePolicy("a remote with that name already exists; choose another name or edit the existing remote"));
            }
            lines.push(format!("add remote '{name}'"));
            lines.push(format!("  fetch URL: {}", label(url)));
            lines.push(format!("  push URL:  {}", label(push_url.as_deref().unwrap_or(url))));
        }
        RemoteChange::SetUrl { name, url, push } => {
            let remote = find(name).ok_or(GitError::UnknownRemote)?;
            let current = if *push { remote.push_url.as_deref().unwrap_or(&remote.fetch_url) } else { &remote.fetch_url };
            lines.push(format!("set {} URL of remote '{name}'", if *push { "push" } else { "fetch" }));
            lines.push(format!("  before: {current}"));
            lines.push(format!("  after:  {}", label(url)));
        }
        RemoteChange::Rename { from, to } => {
            let remote = find(from).ok_or(GitError::UnknownRemote)?;
            if find(to).is_some() {
                return Err(GitError::RemotePolicy("a remote with the new name already exists"));
            }
            lines.push(format!("rename remote '{from}' to '{to}'"));
            lines.push(format!("  fetch URL stays: {}", remote.fetch_url));
            lines.push("  no server repository is renamed".to_owned());
        }
        RemoteChange::Remove { name } => {
            let remote = find(name).ok_or(GitError::UnknownRemote)?;
            lines.push(format!("remove remote '{name}'"));
            lines.push(format!("  fetch URL: {}", remote.fetch_url));
            if let Some(push) = &remote.push_url {
                lines.push(format!("  push URL:  {push}"));
            }
            lines.push("  the server repository and its refs are not touched".to_owned());
        }
    }
    Ok(RemotePreview { lines })
}

/// Applies the reviewed change. The caller must have displayed the matching
/// preview; this function revalidates the same input before running Git.
pub fn apply(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    change: &RemoteChange,
) -> Result<(), GitError> {
    validate_change(git, repo, change)?;
    match change {
        RemoteChange::Add { name, url, push_url } => {
            run(git, repo, &["remote", "add", name, url])?;
            if let Some(push) = push_url {
                run(git, repo, &["remote", "set-url", "--push", name, push])?;
            }
        }
        RemoteChange::SetUrl { name, url, push } => {
            if *push {
                run(git, repo, &["remote", "set-url", "--push", name, url])?;
            } else {
                run(git, repo, &["remote", "set-url", name, url])?;
            }
        }
        RemoteChange::Rename { from, to } => run(git, repo, &["remote", "rename", from, to])?,
        RemoteChange::Remove { name } => run(git, repo, &["remote", "remove", name])?,
    }
    Ok(())
}

/// The remote that a change targets, so a caller can reject a change against a
/// stale selection before showing the preview.
#[must_use]
pub fn target(change: &RemoteChange) -> &str {
    change.name()
}

fn validate_change(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    change: &RemoteChange,
) -> Result<(), GitError> {
    match change {
        RemoteChange::Add { name, url, push_url } => {
            validate_name(git, repo, name)?;
            validate_url(url)?;
            if let Some(push) = push_url {
                validate_url(push)?;
            }
        }
        RemoteChange::SetUrl { name, url, .. } => {
            validate_name(git, repo, name)?;
            validate_url(url)?;
        }
        RemoteChange::Rename { from, to } => {
            validate_name(git, repo, from)?;
            validate_name(git, repo, to)?;
        }
        RemoteChange::Remove { name } => validate_name(git, repo, name)?,
    }
    Ok(())
}

/// Uses Git's own ref rules so a name that would create an invalid tracking ref
/// never reaches a write.
fn validate_name(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    name: &str,
) -> Result<(), GitError> {
    if name.is_empty() || name.starts_with('-') || name.chars().any(char::is_control) {
        return Err(GitError::InvalidRemote);
    }
    let reference = format!("refs/remotes/{name}/probe");
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["check-ref-format", &reference],
        None,
        Limits::METADATA,
    )
    .map_err(|_| GitError::InvalidRemote)?;
    Ok(())
}

/// Accepts the transports the product supports: HTTPS, SSH, git+ssh, file and
/// deliberate local paths. Unknown or insecure protocols are refused rather
/// than handed to a helper.
pub(crate) fn validate_url(url: &str) -> Result<(), GitError> {
    if url.is_empty() || url.chars().any(|character| character.is_control() || character == ' ') {
        return Err(GitError::InvalidRemoteUrl);
    }
    let Some(scheme_end) = url.find("://") else {
        // scp-like user@host:path or a local absolute path.
        return Ok(());
    };
    let scheme = &url[..scheme_end];
    match scheme.to_ascii_lowercase().as_str() {
        "https" | "ssh" | "git+ssh" | "file" => Ok(()),
        _ => Err(GitError::InvalidRemoteUrl),
    }
}

fn label(url: &str) -> String {
    NativePath::new(redact_url(url)).display_label()
}

fn run(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    arguments: &[&str],
) -> Result<(), GitError> {
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        arguments,
        None,
        Limits::MUTATION,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_url;
    use crate::error::GitError;

    #[test]
    fn supported_transports_are_accepted() {
        for url in [
            "https://example.test/team/repo.git",
            "ssh://git@example.test/team/repo.git",
            "git+ssh://git@example.test/repo.git",
            "file:///D:/work/repo.git",
            "git@example.test:team/repo.git",
            "D:/work/repo.git",
        ] {
            assert!(validate_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn unknown_or_insecure_transports_are_refused() {
        for url in [
            "http://example.test/repo.git",
            "ext::sh -c touch% /tmp/pwned",
            "ftp://example.test/repo.git",
            "https://example.test/repo with space.git",
            "https://example.test/repo\nsecond",
            "",
        ] {
            assert!(matches!(validate_url(url), Err(GitError::InvalidRemoteUrl)), "{url}");
        }
    }
}
