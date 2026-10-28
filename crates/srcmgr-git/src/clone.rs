//! Clone with deferred checkout (PRD 03, PRD 07, PRD 11).
//!
//! A clone is a network authority and a target authority of its own; it is not
//! repository trust. Checkout is disabled so no configured filter runs before
//! the downloaded repository is reviewed and trusted.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::remote_edit::validate_url;
use crate::runner::{self, ProgressSink};

/// One reviewed clone request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloneRequest {
    pub url: String,
    pub destination: PathBuf,
    pub branch: Option<String>,
    pub depth: Option<u32>,
}

/// The reviewed effect shown before the transfer starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClonePreview {
    pub destination: PathBuf,
    pub lines: Vec<String>,
}

/// Validates the transport, destination and options without touching the
/// network or creating anything.
pub fn preview(git: &GitInstallation, request: &CloneRequest) -> Result<ClonePreview, GitError> {
    validate_url(&request.url)?;
    let destination = &request.destination;
    if !destination.is_absolute() {
        return Err(GitError::CloneTarget("the destination must be an absolute path"));
    }
    if let Some(branch) = &request.branch {
        if branch.is_empty() || branch.starts_with('-') || branch.chars().any(char::is_control) {
            return Err(GitError::CloneTarget("the branch name is not valid"));
        }
        runner::run(
            git.executable(),
            None,
            &["check-ref-format", "--branch", branch],
            None,
            runner::Limits::METADATA,
        )
        .map_err(|_| GitError::CloneTarget("the branch name is not valid"))?;
    }
    if request.depth.is_some_and(|depth| depth == 0) {
        return Err(GitError::CloneTarget("depth must be at least 1"));
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(GitError::CloneTarget("the destination exists and is not a directory"));
        }
        Ok(_) => {
            let mut entries = fs::read_dir(destination).map_err(GitError::Io)?;
            if entries.next().is_some() {
                return Err(GitError::CloneTarget(
                    "the destination is not empty; choose a new or empty directory",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(GitError::Io(error)),
    }
    let parent = destination.parent().ok_or(GitError::CloneTarget("the destination has no parent directory"))?;
    if !parent.is_dir() {
        return Err(GitError::CloneTarget("the destination's parent directory does not exist"));
    }

    let mut lines = vec![
        format!("clone {url}", url = crate::remotes::redact_url(&request.url)),
        format!("  destination: {}", destination.display()),
        format!("  branch: {}", request.branch.as_deref().unwrap_or("remote default")),
        format!("  depth: {}", request.depth.map_or_else(|| "full history".to_owned(), |depth| depth.to_string())),
        "  checkout is disabled until the downloaded repository is reviewed and trusted".to_owned(),
    ];
    lines.push("  no existing file is moved or deleted".to_owned());
    Ok(ClonePreview { destination: destination.clone(), lines })
}

/// Clones into the reviewed destination without checkout. A failure leaves
/// whatever Git wrote in place so it can be inspected; nothing is deleted.
pub fn clone(
    git: &GitInstallation,
    request: &CloneRequest,
    cancel: &AtomicBool,
    sink: Option<&ProgressSink>,
) -> Result<(), GitError> {
    let preview = preview(git, request)?;
    let mut arguments = vec![
        "clone".to_owned(),
        "--no-checkout".to_owned(),
        "--no-recurse-submodules".to_owned(),
        "--progress".to_owned(),
    ];
    if let Some(branch) = &request.branch {
        arguments.push("--branch".to_owned());
        arguments.push(branch.clone());
    }
    if let Some(depth) = request.depth {
        arguments.push("--depth".to_owned());
        arguments.push(depth.to_string());
    }
    arguments.push("--".to_owned());
    arguments.push(request.url.clone());
    arguments.push(preview.destination.to_string_lossy().into_owned());

    runner::run_network(git.executable(), None, &arguments, cancel, sink)?;
    let metadata = fs::symlink_metadata(preview.destination.join(".git")).map_err(GitError::Io)?;
    if !metadata.is_dir() {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    Ok(())
}
