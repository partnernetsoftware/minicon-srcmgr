//! `minicon-srcmgr`: command-line slice of the Git adapter.
//!
//! This is not the GUI. It exercises the real domain contracts, the supervised
//! Git runner, the trust gate and the JSON codec before the window layer is
//! built on top of them. Every command that reads or writes a repository first
//! passes the trust gate, which checks only filesystem metadata before Git runs.

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use minicon_core::json::{self, JsonValue};
use srcmgr_domain::{
    Capability, CapabilityState, ChangeKind, Generation, HeadState, NativePath, StatusSnapshot,
    VcsBackend,
};
use srcmgr_git::{GitBackend, GitInstallation, RepositoryIdentity, commit, locate, read_status, stage, unstage};
use srcmgr_state::{TrustStore, default_root, fingerprint};
use srcmgr_ui::StatusView;

const USAGE: &str = "usage:
  minicon-srcmgr [--profile DIR] [--git EXE] status <worktree> [--json]
  minicon-srcmgr [--profile DIR] trust grant|revoke <worktree>
  minicon-srcmgr [--profile DIR] [--git EXE] stage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] unstage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] commit <worktree> -m <message>
  minicon-srcmgr [--profile DIR] [--git EXE] gui <worktree>";
const SCHEMA: &str = "minicon-srcmgr.status/1";
const TRUST_FILE: &str = "trust.v1";

#[derive(Default)]
struct Options {
    profile: Option<PathBuf>,
    git: Option<PathBuf>,
    json: bool,
    message: Option<OsString>,
    positional: Vec<OsString>,
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
    let options = parse(arguments)?;
    let command = options.positional.first().and_then(|word| word.to_str()).ok_or(USAGE)?;
    match command {
        "status" => status(&options),
        "trust" => trust(&options),
        "stage" => change_paths(&options, Change::Stage),
        "unstage" => change_paths(&options, Change::Unstage),
        "commit" => commit_command(&options),
        "gui" => gui(&options),
        _ => Err(USAGE.to_owned()),
    }
}

/// Shows the status in a native window. It goes through the same trust gate
/// as `status`, so the window never reads a repository without a grant.
fn gui(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let session = open_trusted(options, worktree)?;
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;
    let name = worktree
        .file_name()
        .map_or_else(|| "repository".to_owned(), |name| name.to_string_lossy().into_owned());
    let view = StatusView::from_snapshot(&snapshot, format!("MiniCon SrcMgr - {name}"));
    srcmgr_ui::show(view).map_err(|error| format!("the status window could not be shown: {error}"))
}

fn parse(arguments: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--profile") => {
                options.profile = Some(PathBuf::from(arguments.next().ok_or("--profile needs a directory")?));
            }
            Some("--git") => {
                options.git = Some(PathBuf::from(arguments.next().ok_or("--git needs an executable path")?));
            }
            Some("--json") => options.json = true,
            Some("-m") => options.message = Some(arguments.next().ok_or("-m needs a message")?),
            _ => options.positional.push(argument),
        }
    }
    Ok(options)
}

fn profile_root(options: &Options) -> Result<PathBuf, String> {
    options
        .profile
        .clone()
        .or_else(|| env::var_os("MINICON_SRCMGR_PROFILE").map(PathBuf::from))
        .or_else(default_root)
        .ok_or_else(|| "no profile directory: pass --profile".to_owned())
}

fn worktree_argument(options: &Options, index: usize) -> Result<&Path, String> {
    options
        .positional
        .get(index)
        .map(|value| Path::new(value.as_os_str()))
        .ok_or_else(|| USAGE.to_owned())
}

/// Metadata-only: the worktree's `.git` must be a plain directory. A `.git`
/// file, a link, or a parent-directory repository is refused, never followed.
fn git_dir_of(worktree: &Path) -> Result<PathBuf, String> {
    let dotgit = worktree.join(".git");
    let metadata = std::fs::symlink_metadata(&dotgit)
        .map_err(|_| "no readable .git directory at this path".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("no readable .git directory at this path".to_owned());
    }
    dotgit
        .canonicalize()
        .map_err(|error| format!("cannot resolve the .git directory: {error}"))
}

fn canonical_worktree(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("cannot open the worktree: {error}"))
}

fn trust_store(options: &Options) -> Result<(PathBuf, TrustStore), String> {
    let path = profile_root(options)?.join(TRUST_FILE);
    let store = TrustStore::open(&path).map_err(|error| error.to_string())?;
    Ok((path, store))
}

/// Everything a repository-touching command needs after the trust gate passed.
struct Session {
    installation: GitInstallation,
    identity: RepositoryIdentity,
}

fn open_trusted(options: &Options, worktree: &Path) -> Result<Session, String> {
    let worktree = canonical_worktree(worktree)?;
    let git_dir = git_dir_of(&worktree)?;
    let (_, store) = trust_store(options)?;
    store
        .verify(&git_dir, &worktree)
        .map_err(|error| format!("{error}; run `minicon-srcmgr trust grant <worktree>` after review"))?;

    let installation = probe_git(options)?;
    let backend = GitBackend::new(installation.clone());
    if let CapabilityState::Unavailable { reason } | CapabilityState::Unsupported { reason } =
        backend.capability(Capability::ReadStatus)
    {
        return Err(format!("status is unavailable: {reason}"));
    }
    refuse_repository_local_executable(&installation, &worktree)?;

    let identity = locate(&installation, &worktree).map_err(|error| error.to_string())?;
    if identity.common_dir != git_dir {
        return Err("the repository identity changed after the trust check; nothing was run".to_owned());
    }
    Ok(Session { installation, identity })
}

fn probe_git(options: &Options) -> Result<GitInstallation, String> {
    let executable = match &options.git {
        Some(path) => path.clone(),
        None => find_git_on_path().ok_or("no git executable was found on PATH; pass --git")?,
    };
    GitInstallation::probe(&executable).map_err(|error| error.to_string())
}

fn status(options: &Options) -> Result<(), String> {
    let session = open_trusted(options, worktree_argument(options, 1)?)?;
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;
    let mut stdout = io::stdout().lock();
    if options.json {
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

fn trust(options: &Options) -> Result<(), String> {
    let action = options.positional.get(1).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let worktree = canonical_worktree(worktree_argument(options, 2)?)?;
    let git_dir = git_dir_of(&worktree)?;
    let (path, mut store) = trust_store(options)?;
    match action {
        "grant" => {
            let digest = fingerprint(&git_dir, &worktree).map_err(|error| error.to_string())?;
            store.grant(git_dir.clone(), digest.clone());
            store.save().map_err(|error| error.to_string())?;
            println!("trusted {} (fingerprint {})", display_path(&git_dir), &digest[..12]);
            println!("hooks, filters and helpers configured there may now run as you; re-grant after any change");
        }
        "revoke" => {
            let removed = store.revoke(&git_dir);
            store.save().map_err(|error| error.to_string())?;
            if removed {
                println!("revoked trust for {}", display_path(&git_dir));
            } else {
                println!("no grant existed for {} (store: {})", display_path(&git_dir), path.display());
            }
        }
        _ => return Err(USAGE.to_owned()),
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Change {
    Stage,
    Unstage,
}

fn change_paths(options: &Options, change: Change) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let paths: Vec<OsString> = options.positional.iter().skip(2).cloned().collect();
    if paths.is_empty() {
        return Err(USAGE.to_owned());
    }
    let session = open_trusted(options, worktree)?;
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;

    // Only paths the status read reports as changed may be targeted; the
    // paths are literal, never patterns.
    for path in &paths {
        let entry = snapshot
            .entries
            .iter()
            .find(|entry| entry.path.as_os_str() == path.as_os_str())
            .ok_or_else(|| format!("not a changed path: {}", NativePath::new(path.clone()).display_label()))?;
        if matches!(change, Change::Unstage) && entry.staged == ChangeKind::Unmodified {
            return Err(format!("not staged: {}", entry.path.display_label()));
        }
    }

    match change {
        Change::Stage => stage(&session.installation, &session.identity, &paths),
        Change::Unstage => unstage(&session.installation, &session.identity, &paths),
    }
    .map_err(|error| error.to_string())?;
    println!("{} {} path(s)", match change { Change::Stage => "staged", Change::Unstage => "unstaged" }, paths.len());
    Ok(())
}

fn commit_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let message = options
        .message
        .as_ref()
        .ok_or("commit needs -m <message>")?
        .to_str()
        .ok_or("the commit message must be valid UTF-8")?
        .to_owned();
    if message.trim().is_empty() {
        return Err("the commit message is empty".to_owned());
    }
    let session = open_trusted(options, worktree)?;
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;
    if !snapshot.entries.iter().any(|entry| entry.staged != ChangeKind::Unmodified) {
        return Err("nothing is staged to commit".to_owned());
    }
    let revision = commit(&session.installation, &session.identity, &message)
        .map_err(|error| error.to_string())?;
    println!("committed {}", short(revision.token()));
    Ok(())
}

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
            let at = revision.as_ref().map_or("no commits yet", |value| short(value.token()));
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

/// Shows a canonical path without the Windows verbatim prefix. Display only;
/// stored and compared paths keep their canonical form.
fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\?\").map_or(text.clone(), str::to_owned)
}
