//! `minicon-srcmgr`: read-only command-line slice of the Git adapter.
//!
//! This is not the GUI. It exists to exercise the real domain contracts, the
//! supervised Git runner and the JSON codec from MiniCon before the window
//! layer is built on top of them.

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use minicon_core::json::{self, JsonValue};
use srcmgr_domain::{
    Capability, CapabilityState, ChangeKind, Generation, HeadState, StatusSnapshot, VcsBackend,
};
use srcmgr_git::{GitBackend, GitInstallation, locate, read_status};

const USAGE: &str = "usage: minicon-srcmgr status <worktree> [--git <executable>] [--json] --approve-git-execution";
const SCHEMA: &str = "minicon-srcmgr.status/1";

struct StatusRequest {
    worktree: PathBuf,
    git: Option<PathBuf>,
    json: bool,
    approved: bool,
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("minicon-srcmgr: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), String> {
    let request = parse_status(arguments)?;
    if !request.approved {
        return Err(
            "reading this repository runs Git and any configured hooks or helpers; \
             pass --approve-git-execution to allow it for this invocation (interim until the trust gate lands)"
                .to_owned(),
        );
    }

    let executable = match &request.git {
        Some(path) => path.clone(),
        None => find_git_on_path().ok_or("no git executable was found on PATH; pass --git")?,
    };
    let installation = GitInstallation::probe(&executable).map_err(|error| error.to_string())?;
    let backend = GitBackend::new(installation.clone());
    if let CapabilityState::Unavailable { reason } | CapabilityState::Unsupported { reason } =
        backend.capability(Capability::ReadStatus)
    {
        return Err(format!("status is unavailable: {reason}"));
    }

    let worktree = request
        .worktree
        .canonicalize()
        .map_err(|error| format!("cannot open the worktree: {error}"))?;
    refuse_repository_local_executable(&installation, &worktree)?;

    let identity = locate(&installation, &worktree).map_err(|error| error.to_string())?;
    let snapshot =
        read_status(&installation, &identity, Generation(1)).map_err(|error| error.to_string())?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if request.json {
        let mut bytes = json::to_vec_pretty(&status_json(&snapshot));
        bytes.push(b'\n');
        stdout.write_all(&bytes).map_err(|error| error.to_string())?;
    } else {
        stdout
            .write_all(status_text(&snapshot).as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn parse_status(arguments: Vec<OsString>) -> Result<StatusRequest, String> {
    let mut arguments = arguments.into_iter();
    match arguments.next() {
        Some(command) if command == "status" => {}
        _ => return Err(USAGE.to_owned()),
    }

    let mut worktree = None;
    let mut git = None;
    let mut json = false;
    let mut approved = false;
    while let Some(argument) = arguments.next() {
        if argument == "--git" {
            git = Some(PathBuf::from(
                arguments.next().ok_or("--git needs an executable path")?,
            ));
        } else if argument == "--json" {
            json = true;
        } else if argument == "--approve-git-execution" {
            approved = true;
        } else if worktree.is_none() {
            worktree = Some(PathBuf::from(argument));
        } else {
            return Err(USAGE.to_owned());
        }
    }
    Ok(StatusRequest {
        worktree: worktree.ok_or(USAGE)?,
        git,
        json,
        approved,
    })
}

/// Finds the first `git` on PATH. Only the process environment is consulted;
/// the current directory and any repository are never searched.
fn find_git_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["git.exe"] } else { &["git"] };
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// PRD 11: a Git executable inside the repository under inspection could be
/// planted by that repository, so it is refused.
fn refuse_repository_local_executable(installation: &GitInstallation, worktree: &Path) -> Result<(), String> {
    let executable = installation
        .executable()
        .canonicalize()
        .map_err(|error| format!("cannot resolve the git executable: {error}"))?;
    if executable.starts_with(worktree) {
        return Err("the selected git executable lies inside the repository being inspected; choose an installed Git".to_owned());
    }
    Ok(())
}

fn status_json(snapshot: &StatusSnapshot) -> JsonValue {
    let head = match &snapshot.head {
        HeadState::Branch { name, revision } => json::object(vec![
            ("kind", "branch".into()),
            ("name", name.as_str().into()),
            ("revision", json::nullable(revision.as_ref().map(|value| value.token()))),
        ]),
        HeadState::Detached { revision } => json::object(vec![
            ("kind", "detached".into()),
            ("name", JsonValue::Null),
            ("revision", revision.token().into()),
        ]),
    };
    let entries = snapshot
        .entries
        .iter()
        .map(|entry| {
            json::object(vec![
                ("id", entry.id.as_str().into()),
                ("staged", kind_name(entry.staged).into()),
                ("unstaged", kind_name(entry.unstaged).into()),
                ("path", entry.path.display_label().into()),
                (
                    "original_path",
                    json::nullable(entry.original_path.as_ref().map(|path| path.display_label())),
                ),
            ])
        })
        .collect();
    json::object(vec![
        ("schema", SCHEMA.into()),
        ("backend", snapshot.backend.id().into()),
        ("generation", snapshot.generation.0.into()),
        ("head", head),
        ("entries", JsonValue::Array(entries)),
    ])
}

fn status_text(snapshot: &StatusSnapshot) -> String {
    let mut text = String::new();
    match &snapshot.head {
        HeadState::Branch { name, revision } => {
            let at = revision
                .as_ref()
                .map_or("no commits yet", |value| short(value.token()));
            text.push_str(&format!("On branch {name} ({at})\n"));
        }
        HeadState::Detached { revision } => {
            text.push_str(&format!("HEAD detached at {}\n", short(revision.token())));
        }
    }
    if snapshot.entries.is_empty() {
        text.push_str("Working tree clean\n");
    }
    for entry in &snapshot.entries {
        let original = entry
            .original_path
            .as_ref()
            .map_or(String::new(), |path| format!(" (from {})", path.display_label()));
        text.push_str(&format!(
            "{}{} {}{original}\n",
            code(entry.staged),
            code(entry.unstaged),
            entry.path.display_label(),
        ));
    }
    text
}

fn short(token: &str) -> &str {
    &token[..token.len().min(12)]
}

fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Unmodified => "unmodified",
        ChangeKind::Modified => "modified",
        ChangeKind::TypeChanged => "type-changed",
        ChangeKind::Added => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Copied => "copied",
        ChangeKind::Unmerged => "unmerged",
        ChangeKind::Untracked => "untracked",
        ChangeKind::Ignored => "ignored",
    }
}

fn code(kind: ChangeKind) -> char {
    match kind {
        ChangeKind::Unmodified => '.',
        ChangeKind::Modified => 'M',
        ChangeKind::TypeChanged => 'T',
        ChangeKind::Added => 'A',
        ChangeKind::Deleted => 'D',
        ChangeKind::Renamed => 'R',
        ChangeKind::Copied => 'C',
        ChangeKind::Unmerged => 'U',
        ChangeKind::Untracked => '?',
        ChangeKind::Ignored => '!',
    }
}
