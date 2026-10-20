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
use srcmgr_git::{
    GitBackend, GitInstallation, RepositoryIdentity, commit, locate, read_status, stage, unstage,
};
use srcmgr_state::{TrustStore, default_root, fingerprint};

mod window_backend;

const USAGE: &str = "usage:
  minicon-srcmgr [--profile DIR] [--git EXE] status <worktree> [--json]
  minicon-srcmgr [--profile DIR] trust grant|revoke <worktree>
  minicon-srcmgr [--profile DIR] [--git EXE] stage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] unstage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] commit <worktree> [-m <message>]
  minicon-srcmgr [--profile DIR] draft <worktree> save -m <message> | show | clear
  minicon-srcmgr [--profile DIR] [--git EXE] gui <worktree>
  minicon-srcmgr [--profile DIR] [--git EXE] diff <worktree> <path> [--staged]
  minicon-srcmgr [--profile DIR] [--git EXE] branch <worktree> list|create|switch <name>
  minicon-srcmgr [--profile DIR] [--git EXE] history <worktree> [--skip N] [--limit N]
  minicon-srcmgr [--profile DIR] setting get|set <key> [value]   (keys: history.limit)
  minicon-srcmgr [--profile DIR] [--git EXE] ignore <worktree> <path>
  minicon-srcmgr [--profile DIR] [--git EXE] hunk <worktree> list <path> [--staged]
  minicon-srcmgr [--profile DIR] [--git EXE] hunk <worktree> stage|unstage <path> <number>";
const SCHEMA: &str = "minicon-srcmgr.status/1";
const TRUST_FILE: &str = "trust.v1";

#[derive(Clone, Default)]
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
        "draft" => draft_command(&options),
        "gui" => gui(&options),
        "diff" => diff_command(&options),
        "branch" => branch_command(&options),
        "hunk" => hunk_command(&options),
        "ignore" => ignore_command(&options),
        "history" => history_command(&options),
        "setting" => setting_command(&options),
        _ => Err(USAGE.to_owned()),
    }
}

/// Opens the workspace window. The first snapshot goes through the trust gate,
/// so the window never shows a repository that is not granted; every later
/// action passes the gate again on the worker.
fn gui(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let backend = window_backend::WindowBackend {
        options: options.clone(),
        worktree: worktree.to_path_buf(),
    };
    let snapshot = backend.snapshot()?;
    srcmgr_ui::show(snapshot, Box::new(backend))
        .map_err(|error| format!("the status window could not be shown: {error}"))
}

/// `branch <worktree> create <name>` or `branch <worktree> switch <name>`.
fn branch_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let action = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    if action == "list" {
        for branch in srcmgr_git::local_branches(&session.installation, &session.identity)
            .map_err(|error| error.to_string())?
        {
            println!("{} {}", if branch.current { '*' } else { ' ' }, branch.label);
        }
        return Ok(());
    }
    let name = options.positional.get(3).ok_or(USAGE)?;
    match action {
        "create" => {
            srcmgr_git::create_branch(&session.installation, &session.identity, name)
                .map_err(|error| error.to_string())?;
            println!("created and switched to {}", name.to_string_lossy());
        }
        "switch" => {
            srcmgr_git::switch_branch(&session.installation, &session.identity, name)
                .map_err(|error| error.to_string())?;
            println!("switched to {}", name.to_string_lossy());
        }
        _ => return Err(USAGE.to_owned()),
    }
    Ok(())
}

/// `history <worktree> [--limit N]`: a newest-first commit list from HEAD.
/// Merges are marked `M`. This is a list, not a lane graph.
const HISTORY_LIMIT_KEY: &str = "history.limit";
const DEFAULT_HISTORY_LIMIT: usize = 20;
const MAX_HISTORY_LIMIT: usize = 200;

/// `setting get|set <key> [value]`: app preferences kept in the profile store.
/// Only the keys listed here are accepted, and each value is validated before
/// it is written, so a typo cannot silently persist.
fn setting_command(options: &Options) -> Result<(), String> {
    let action = options.positional.get(1).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let key = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    if key != HISTORY_LIMIT_KEY {
        return Err(format!("unknown setting '{key}'; known keys: {HISTORY_LIMIT_KEY}"));
    }
    let mut store = open_profile_store(options)?;
    match action {
        "get" => match store.get_setting(key).map_err(|error| error.to_string())? {
            Some(value) => println!("{value}"),
            None => println!("unset"),
        },
        "set" => {
            let value = options.positional.get(3).and_then(|word| word.to_str()).ok_or(USAGE)?;
            let limit = parse_history_limit(value)?;
            store
                .put_setting(key, &limit.to_string())
                .map_err(|error| error.to_string())?;
            println!("{key} = {limit}");
        }
        _ => return Err(USAGE.to_owned()),
    }
    Ok(())
}

fn parse_history_limit(text: &str) -> Result<usize, String> {
    match text.parse::<usize>() {
        Ok(limit) if (1..=MAX_HISTORY_LIMIT).contains(&limit) => Ok(limit),
        _ => Err(format!("{HISTORY_LIMIT_KEY} must be a number from 1 to {MAX_HISTORY_LIMIT}")),
    }
}

fn history_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let limit: usize = match options.positional.iter().position(|word| word == "--limit") {
        Some(position) => options
            .positional
            .get(position + 1)
            .and_then(|word| word.to_str())
            .and_then(|text| text.parse().ok())
            .ok_or("--limit needs a number")?,
        // The store is opened only when no --limit is given, so an explicit
        // limit never waits on the profile lock.
        None => {
            let store = open_profile_store(options)?;
            match store.get_setting(HISTORY_LIMIT_KEY).map_err(|error| error.to_string())? {
                Some(saved) => parse_history_limit(&saved)?,
                None => DEFAULT_HISTORY_LIMIT,
            }
        }
    };
    let skip: usize = match options.positional.iter().position(|word| word == "--skip") {
        Some(position) => options
            .positional
            .get(position + 1)
            .and_then(|word| word.to_str())
            .and_then(|text| text.parse().ok())
            .ok_or("--skip needs a number")?,
        None => 0,
    };
    let session = open_trusted(options, worktree)?;
    let commits = srcmgr_git::log(&session.installation, &session.identity, skip, limit)
        .map_err(|error| error.to_string())?;
    if commits.is_empty() {
        println!("no commits yet");
    }
    for commit in &commits {
        let marker = if commit.parents.len() > 1 { 'M' } else { ' ' };
        println!("{marker} {} {}", short(commit.id.token()), commit.subject);
    }
    Ok(())
}

/// `ignore <worktree> <path>`: names the rule that ignores the path, or says
/// that none does. Git decides the match; this only reports it.
fn ignore_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let path = options.positional.get(2).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    match srcmgr_git::ignore_match(&session.installation, &session.identity, path)
        .map_err(|error| error.to_string())?
    {
        Some(rule) => println!("ignored by {rule}"),
        None => println!("not ignored"),
    }
    Ok(())
}

/// `hunk <worktree> list <path> [--staged]`, or
/// `hunk <worktree> stage|unstage <path> <number>`. Hunk numbers start at 1.
/// Staging applies to the index only; unstaging reverses a staged hunk.
fn hunk_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let action = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let path = options.positional.get(3).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    match action {
        "list" => {
            let staged = options.positional.iter().any(|word| word == "--staged");
            let hunks = srcmgr_git::hunks(&session.installation, &session.identity, path, staged)
                .map_err(|error| error.to_string())?;
            for (index, hunk) in hunks.iter().enumerate() {
                println!("hunk {}", index + 1);
                print!("{hunk}");
            }
        }
        "stage" | "unstage" => {
            let number: usize = options
                .positional
                .get(4)
                .and_then(|word| word.to_str())
                .and_then(|text| text.parse().ok())
                .filter(|number| *number >= 1)
                .ok_or("give the hunk number, starting at 1")?;
            srcmgr_git::apply_hunk(
                &session.installation,
                &session.identity,
                path,
                number - 1,
                action == "unstage",
            )
            .map_err(|error| error.to_string())?;
            println!("{action}d hunk {number}");
        }
        _ => return Err(USAGE.to_owned()),
    }
    Ok(())
}

/// Prints the textual diff of one path, unstaged by default and staged with
/// `--staged`. The same trust gate as `status` applies.
fn diff_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let path = options.positional.get(2).ok_or(USAGE)?;
    let staged = options.positional.iter().any(|word| word == "--staged");
    let session = open_trusted(options, worktree)?;
    let text = srcmgr_git::diff(&session.installation, &session.identity, path, staged)
        .map_err(|error| error.to_string())?;
    print!("{text}");
    Ok(())
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

/// Commits the index. The message comes from `-m` or, when that is absent, from
/// the saved draft for this repository. A successful commit clears the draft.
/// A refused commit leaves the draft in place.
fn commit_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let key = repository_key(worktree)?;
    let mut store = open_profile_store(options)?;
    let message = match &options.message {
        Some(given) => given
            .to_str()
            .ok_or("the commit message must be valid UTF-8")?
            .to_owned(),
        None => store
            .get_draft(BACKEND_GIT, &key)
            .map_err(|error| error.to_string())?
            .ok_or("commit needs -m <message> or a saved draft")?,
    };
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
    store
        .delete_draft(BACKEND_GIT, &key)
        .map_err(|error| error.to_string())?;
    println!("committed {}", short(revision.token()));
    Ok(())
}

const BACKEND_GIT: &str = "git";

/// The repository key used for drafts: the canonical `.git` directory, found
/// from metadata alone so that saving a draft never runs Git.
fn repository_key(worktree: &Path) -> Result<String, String> {
    let canonical = canonical_worktree(worktree)?;
    let git_dir = git_dir_of(&canonical)?;
    git_dir
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "the repository path cannot be stored as UTF-8".to_owned())
}

fn open_profile_store(options: &Options) -> Result<srcmgr_state::ProfileStore, String> {
    let root = profile_root(options)?;
    srcmgr_state::ProfileStore::open(&root).map_err(|error| error.to_string())
}

/// `draft <worktree> save -m <message> | show | clear`. Drafts store text only
/// and never run Git, so they need no trust grant.
fn draft_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let action = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let key = repository_key(worktree)?;
    let mut store = open_profile_store(options)?;
    match action {
        "save" => {
            let message = options
                .message
                .as_ref()
                .ok_or("draft save needs -m <message>")?
                .to_str()
                .ok_or("the draft must be valid UTF-8")?;
            store
                .put_draft(BACKEND_GIT, &key, message)
                .map_err(|error| error.to_string())?;
            println!("draft saved");
        }
        "show" => match store.get_draft(BACKEND_GIT, &key).map_err(|error| error.to_string())? {
            Some(message) => print!("{message}"),
            None => println!("no draft"),
        },
        "clear" => {
            store
                .delete_draft(BACKEND_GIT, &key)
                .map_err(|error| error.to_string())?;
            println!("draft cleared");
        }
        _ => return Err(USAGE.to_owned()),
    }
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
