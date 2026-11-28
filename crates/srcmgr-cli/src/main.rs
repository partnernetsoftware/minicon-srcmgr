//! `minicon-srcmgr`: command-line slice of the Git adapter.
//!
//! This is not the GUI. It exercises the real domain contracts, the supervised
//! Git runner, the trust gate and the JSON codec before the window layer is
//! built on top of them. Every command that reads or writes a repository first
//! passes the trust gate, which checks only filesystem metadata before Git runs.

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use minicon_core::json::{self, JsonValue};
use srcmgr_domain::{
    Capability, CapabilityState, ChangeKind, Generation, HeadState, NativePath, StatusSnapshot,
    VcsBackend,
};
use srcmgr_git::{
    GitBackend, GitInstallation, RepositoryIdentity, locate, read_status, stage, unstage,
};
use srcmgr_state::{TrustStore, default_root, fingerprint};

mod args;
mod text_file;
mod window_backend;
mod clone_cleanup;
mod confirmation;
mod entry;
mod tools;

use args::{BranchAction, Command, ConfigAction, DraftAction, HunkAction, Porcelain, RemoteAction, RemoteChange, TrustAction, Usage};

const SCHEMA: &str = "minicon-srcmgr.status/1";
const TRUST_FILE: &str = "trust.v1";

/// The global options. Everything after them is parsed per verb in [`args`].
#[derive(Clone, Default)]
struct Options {
    profile: Option<PathBuf>,
    git: Option<PathBuf>,
    /// `-C <path>`; empty means "the current directory, as git reads it".
    directory: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Refusal::Usage(message)) => {
            eprintln!("minicon-srcmgr: {message}");
            // git exits 129 for a usage error.
            ExitCode::from(129)
        }
        Err(Refusal::Failed(message)) => {
            eprintln!("minicon-srcmgr: {message}");
            ExitCode::FAILURE
        }
    }
}

/// How a run ended. A usage error is the caller's mistake and gets git's own
/// exit code; a failed or refused operation is `1`.
#[derive(Debug)]
enum Refusal {
    Usage(String),
    Failed(String),
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Refusal::Failed(message)
    }
}

impl From<Usage> for Refusal {
    fn from(usage: Usage) -> Self {
        Refusal::Usage(usage.0)
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), Refusal> {
    let parsed = args::parse(arguments)?;
    let options = Options { profile: parsed.profile, git: parsed.git, directory: parsed.directory };
    match parsed.command {
        Command::Version => {
            println!("minicon-srcmgr {}", env!("CARGO_PKG_VERSION"));
        }
        Command::Status { porcelain, null, json } => status(&options, porcelain, null, json)?,
        Command::Trust { action } => trust(&options, action)?,
        Command::Stage { paths } => change_paths(&options, Change::Stage, &paths)?,
        Command::Unstage { paths } => change_paths(&options, Change::Unstage, &paths)?,
        Command::Commit { message } => commit_command(&options, message.as_deref())?,
        Command::Draft { action, message } => draft_command(&options, action, message.as_deref())?,
        Command::Gui => gui(&options)?,
        Command::Diff { staged, path } => diff_command(&options, &path, staged)?,
        Command::Branch { action } => branch_command(&options, action)?,
        Command::Remote { action } => remote_command(&options, action)?,
        Command::Fetch { remote, json } => fetch_command(&options, &remote, json)?,
        Command::Clone { url, directory, branch, depth } => clone_command(&options, &url, directory, branch, depth)?,
        Command::Pull => pull_command(&options)?,
        Command::Push { remote } => push_command(&options, &remote)?,
        Command::Hunk { action } => hunk_command(&options, action)?,
        Command::Log { skip, limit } => log_command(&options, skip, limit)?,
        Command::CheckIgnore { path } => check_ignore_command(&options, &path)?,
        Command::Config { action } => config_command(&options, action)?,
    }
    Ok(())
}

/// Opens the workspace window. The first snapshot goes through the trust gate,
/// so the window never shows a repository that is not granted; every later
/// action passes the gate again on the worker.
fn gui(options: &Options) -> Result<(), String> {
    let initial = options.directory.as_ref().map(|path| NativePath::new(path.as_os_str().to_owned()));
    let timing = if env::var_os("SRCMGR_MEASURE_UI").as_deref() == Some(std::ffi::OsStr::new("1")) {
        drop(open_profile_store(options)?);
        let profile = std::fs::canonicalize(profile_root(options)?).map_err(|e| e.to_string())?;
        if let Some(path) = &initial {
            let repository = canonical_worktree(Path::new(path.as_os_str()))?;
            if profile.starts_with(repository) {
                return Err("UI qualification receipts require a profile outside the repository".into());
            }
        }
        Some(std::fs::OpenOptions::new().write(true).create_new(true)
            .open(profile.join(format!("timing-{}.json", std::process::id())))
            .map_err(|e| format!("cannot create exclusive UI timing receipt: {e}"))?)
    } else { None };
    srcmgr_ui::show_entry(initial, Box::new(entry::ApplicationBackend::welcome(options.clone())), timing)
        .map_err(|error| format!("the workspace window could not be shown: {error}"))
}

/// `branch [--list] | -m [<old>] <new> | create <name> | switch <name>`.
fn branch_command(options: &Options, action: BranchAction) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    match action {
        BranchAction::List => {
            for branch in srcmgr_git::local_branches(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?
            {
                println!("{} {}", if branch.current { '*' } else { ' ' }, branch.label);
            }
        }
        BranchAction::Rename { old, new } => {
            let current = || {
                srcmgr_git::local_branches(&session.installation, &session.identity)
                    .map_err(|error| error.to_string())
                    .and_then(|branches| {
                        branches.into_iter().find(|branch| branch.current).map(|branch| branch.label).ok_or_else(|| "no current branch to rename; give the old name".to_owned())
                    })
            };
            let old = match old {
                Some(old) => old,
                None => current()?,
            };
            srcmgr_git::rename_branch(&session.installation, &session.identity, &old, &new)
                .map_err(|error| error.to_string())?;
            println!("renamed {old} to {new}");
        }
        BranchAction::Create(name) => {
            srcmgr_git::create_branch(&session.installation, &session.identity, name.as_ref())
                .map_err(|error| error.to_string())?;
            println!("created and switched to {name}");
        }
        BranchAction::Switch(name) => {
            srcmgr_git::switch_branch(&session.installation, &session.identity, name.as_ref())
                .map_err(|error| error.to_string())?;
            println!("switched to {name}");
        }
    }
    Ok(())
}

/// `pull <worktree>`: fetch facts are assumed current; the merge is fast-forward only.
fn pull_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    let cancel = AtomicBool::new(false);
    let sink: srcmgr_git::ProgressSink = Arc::new(|chunk: &str| {
        if let Some(line) = srcmgr_git::progress_line(chunk) {
            eprintln!("{line}");
        }
    });
    let report = srcmgr_git::pull_ff(&session.installation, &session.identity, &cancel, Some(&sink))
        .map_err(|error| error.to_string())?;
    if report.updated {
        println!("fast-forwarded {}..{}", short_object_id(&report.before), short_object_id(&report.after));
    } else {
        println!("already up to date");
    }
    Ok(())
}

/// `push <worktree> <remote>`: one explicit branch ref, per-ref state reported.
fn push_command(options: &Options, remote: &str) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    let cancel = AtomicBool::new(false);
    let sink: srcmgr_git::ProgressSink = Arc::new(|chunk: &str| {
        if let Some(line) = srcmgr_git::progress_line(chunk) {
            eprintln!("{line}");
        }
    });
    let report = srcmgr_git::push_current(&session.installation, &session.identity, remote, &cancel, Some(&sink))
        .map_err(|error| error.to_string())?;
    let state = match report.state {
        srcmgr_git::PushState::Updated => "updated".to_owned(),
        srcmgr_git::PushState::UpToDate => "up to date".to_owned(),
        srcmgr_git::PushState::Rejected => "rejected by the remote; fetch and review before retrying".to_owned(),
        srcmgr_git::PushState::Unknown { reconciled: true } => "unknown result; the remote already has this commit".to_owned(),
        srcmgr_git::PushState::Unknown { reconciled: false } => "unknown result; observe the remote before retrying".to_owned(),
    };
    println!("{} {}: {state}", report.branch, report.reference);
    if let Some(observed) = &report.observed {
        println!("remote {observed}");
    }
    Ok(())
}

/// The reviewed change, mirroring `srcmgr_git::RemoteChange`.
fn remote_change(change: RemoteChange) -> srcmgr_git::RemoteChange {
    match change {
        RemoteChange::Add { name, url, push_url } => srcmgr_git::RemoteChange::Add { name, url, push_url },
        RemoteChange::SetUrl { name, url, push } => srcmgr_git::RemoteChange::SetUrl { name, url, push },
        RemoteChange::Rename { from, to } => srcmgr_git::RemoteChange::Rename { from, to },
        RemoteChange::Remove { name } => srcmgr_git::RemoteChange::Remove { name },
    }
}

/// `clone [--branch <b>] [--depth <n>] <url> <dir>`: reviewed destination and
/// transport, no checkout. A transfer failure keeps whatever was written so it
/// can be inspected.
fn clone_command(options: &Options, url: &str, destination: PathBuf, branch: Option<String>, depth: Option<u32>) -> Result<(), String> {
    let installation = GitInstallation::probe(selected_git(options)?).map_err(|error| error.to_string())?;
    let request = srcmgr_git::CloneRequest { url: url.to_owned(), destination, branch, depth };
    let preview = srcmgr_git::preview_clone(&installation, &request).map_err(|error| error.to_string())?;
    for line in &preview.lines {
        println!("{line}");
    }
    let cancel = AtomicBool::new(false);
    let sink: srcmgr_git::ProgressSink = Arc::new(|chunk: &str| {
        if let Some(line) = srcmgr_git::progress_line(chunk) {
            eprintln!("{line}");
        }
    });
    srcmgr_git::clone_repository(&installation, &request, &cancel, Some(&sink)).map_err(|error| {
        format!("{error}; inspect {} for a partial clone before retrying", preview.destination.display())
    })?;
    println!("cloned into {}", preview.destination.display());
    println!("checkout is deferred: review and trust the repository, then approve the filter review");
    Ok(())
}

/// Emits one JSON document on stdout. Used by the read-only remote commands so
/// their shape matches `status --json`.
fn print_json(value: JsonValue) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    let mut bytes = json::to_vec_pretty(&value);
    bytes.push(b'\n');
    stdout.write_all(&bytes).map_err(|error| error.to_string())
}

/// `remote list|tracking [--json]`, or a reviewed configuration change that
/// is applied only with `--yes`.
fn remote_command(options: &Options, action: RemoteAction) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    match action {
        RemoteAction::Change { change, yes } => {
            let change = remote_change(change);
            let preview = srcmgr_git::preview_remote_change(&session.installation, &session.identity, &change)
                .map_err(|error| error.to_string())?;
            for line in &preview.lines {
                println!("{line}");
            }
            if yes {
                srcmgr_git::apply_remote_change(&session.installation, &session.identity, &change)
                    .map_err(|error| error.to_string())?;
                println!("applied; the repository trust fingerprint changed - review and grant trust again before the next action");
            } else {
                println!("preview only; add --yes to apply");
            }
            Ok(())
        }
        RemoteAction::List { json } => {
            let remotes = srcmgr_git::list_remotes(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?;
            if json {
                let rows = remotes
                    .iter()
                    .map(|remote| {
                        json::object(vec![
                            ("name", remote.name.as_str().into()),
                            ("fetch_url", remote.fetch_url.as_str().into()),
                            ("push_url", json::nullable(remote.push_url.clone())),
                            (
                                "refspecs",
                                JsonValue::Array(remote.fetch_refspecs.iter().map(|refspec| refspec.as_str().into()).collect()),
                            ),
                        ])
                    })
                    .collect();
                print_json(JsonValue::Array(rows))
            } else {
                for remote in remotes {
                    match &remote.push_url {
                        Some(push) => println!("{}\t{}\tpush {push}", remote.name, remote.fetch_url),
                        None => println!("{}\t{}", remote.name, remote.fetch_url),
                    }
                }
                Ok(())
            }
        }
        RemoteAction::Tracking { json } => {
            let state = srcmgr_git::tracking(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?;
            if json {
                print_json(json::object(vec![
                    ("upstream", json::nullable(state.upstream.clone())),
                    ("ahead", JsonValue::Unsigned(u64::from(state.ahead))),
                    ("behind", JsonValue::Unsigned(u64::from(state.behind))),
                ]))
            } else {
                match &state.upstream {
                    Some(upstream) => println!("{upstream}  ahead {}  behind {}", state.ahead, state.behind),
                    None => println!("no upstream"),
                }
                Ok(())
            }
        }
    }
}

/// `fetch [--json] <remote>`: one explicit remote, with transfer progress on
/// stderr and the actual refs Git reported on stdout.
fn fetch_command(options: &Options, remote: &str, json_output: bool) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    let cancel = AtomicBool::new(false);
    let sink: srcmgr_git::ProgressSink = Arc::new(|chunk: &str| {
        if let Some(line) = srcmgr_git::progress_line(chunk) {
            eprintln!("{line}");
        }
    });
    let report = srcmgr_git::fetch(&session.installation, &session.identity, remote, &cancel, Some(&sink))
        .map_err(|error| error.to_string())?;
    if json_output {
        let updates = report
            .updates
            .iter()
            .map(|update| {
                json::object(vec![
                    ("old", update.old.as_str().into()),
                    ("new", update.new.as_str().into()),
                    ("ref", update.reference.as_str().into()),
                ])
            })
            .collect();
        print_json(json::object(vec![("remote", report.remote.as_str().into()), ("updates", JsonValue::Array(updates))]))
    } else {
        if report.updates.is_empty() {
            println!("{}: up to date", report.remote);
        }
        for update in &report.updates {
            println!("{}..{} {}", short_object_id(&update.old), short_object_id(&update.new), update.reference);
        }
        Ok(())
    }
}

fn short_object_id(object_id: &str) -> &str {
    object_id.get(..7).unwrap_or(object_id)
}

/// `history <worktree> [--limit N]`: a newest-first commit list from HEAD.
/// Merges are marked `M`. This is a list, not a lane graph.
const HISTORY_LIMIT_KEY: &str = "history.limit";
const DEFAULT_HISTORY_LIMIT: usize = 20;
const MAX_HISTORY_LIMIT: usize = 200;
/// The interface language. `system` follows the environment once at startup;
/// `en` and `zh-CN` are explicit and persist across restarts.
const UI_LOCALE_KEY: &str = "ui.locale";

/// The environment's own language preference. Windows has no `LANG` unless the
/// user set one, so `system` resolves to English there and an explicit choice is
/// the only way to Chinese; that is stated in the release notes rather than
/// guessed from a registry value this build does not read.
pub(crate) fn chinese_environment() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| env::var(name).ok())
        .any(|value| value.to_ascii_lowercase().trim_start().starts_with("zh"))
}

/// Resolves the stored language against the environment. Reading the setting is
/// the only profile access here, so a language change needs no repository.
pub(crate) fn interface_locale(store: &srcmgr_state::ProfileStore) -> Result<srcmgr_ui::Locale, String> {
    let saved = store.get_setting(UI_LOCALE_KEY).map_err(|error| error.to_string())?;
    Ok(srcmgr_ui::Locale::resolve(saved.as_deref(), chinese_environment()))
}

/// Accepts only the three values the window can render. Anything else is a
/// refusal rather than a silent fallback, so a typo cannot persist.
fn parse_locale(text: &str) -> Result<&'static str, String> {
    match text {
        "system" => Ok("system"),
        "en" => Ok("en"),
        "zh-CN" | "zh_CN" | "zh" => Ok("zh-CN"),
        _ => Err(format!("{UI_LOCALE_KEY} must be one of: system, en, zh-CN")),
    }
}

/// `config get|set <key> [value]`: SrcMgr preferences kept in the profile
/// store, in the `git config` spelling. These are the product's own keys, not
/// Git configuration. Only the keys listed here are accepted, and each value is
/// validated before it is written, so a typo cannot silently persist.
fn config_command(options: &Options, action: ConfigAction) -> Result<(), String> {
    let key = match &action {
        ConfigAction::Get { key } | ConfigAction::Set { key, .. } => key.as_str(),
    };
    if key != HISTORY_LIMIT_KEY && key != UI_LOCALE_KEY {
        return Err(format!("unknown setting '{key}'; known keys: {HISTORY_LIMIT_KEY}, {UI_LOCALE_KEY}"));
    }
    let mut store = open_profile_store(options)?;
    match action {
        ConfigAction::Get { key } => match store.get_setting(&key).map_err(|error| error.to_string())? {
            Some(value) => println!("{value}"),
            None => println!("unset"),
        },
        ConfigAction::Set { key, value } => {
            let value = value.to_str().ok_or("the value must be valid UTF-8")?;
            if key == UI_LOCALE_KEY {
                let locale = parse_locale(value)?;
                store.put_setting(&key, locale).map_err(|error| error.to_string())?;
                println!("{key} = {locale}");
            } else {
                let limit = parse_history_limit(value)?;
                store.put_setting(&key, &limit.to_string()).map_err(|error| error.to_string())?;
                println!("{key} = {limit}");
            }
        }
    }
    Ok(())
}

fn parse_history_limit(text: &str) -> Result<usize, String> {
    match text.parse::<usize>() {
        Ok(limit) if (1..=MAX_HISTORY_LIMIT).contains(&limit) => Ok(limit),
        _ => Err(format!("{HISTORY_LIMIT_KEY} must be a number from 1 to {MAX_HISTORY_LIMIT}")),
    }
}

/// `log [--skip <n>] [--limit <n>]`: a newest-first commit list from HEAD,
/// in the `git log` spelling. Merges are marked `M`; this is a list, not a
/// lane graph.
fn log_command(options: &Options, skip: usize, limit: Option<usize>) -> Result<(), String> {
    // The store is opened only when no --limit is given, so an explicit limit
    // never waits on the profile lock.
    let limit = match limit {
        Some(limit) => limit,
        None => match open_profile_store(options)?.get_setting(HISTORY_LIMIT_KEY).map_err(|error| error.to_string())? {
            Some(saved) => parse_history_limit(&saved)?,
            None => DEFAULT_HISTORY_LIMIT,
        },
    };
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
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

/// `check-ignore [--] <path>`: names the rule that ignores the path, or says
/// that none does. Git decides the match; this only reports it. `git
/// check-ignore` exits 1 when nothing matches; so does this.
fn check_ignore_command(options: &Options, path: &OsStr) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    match srcmgr_git::ignore_match(&session.installation, &session.identity, path)
        .map_err(|error| error.to_string())?
    {
        Some(rule) => println!("ignored by {rule}"),
        None => {
            println!("not ignored");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `hunk list [--staged] [--] <path>`, or `hunk stage|unstage [--] <path> <n>`.
/// Hunk numbers start at 1. Staging applies to the index only; unstaging
/// reverses a staged hunk.
fn hunk_command(options: &Options, action: HunkAction) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    match action {
        HunkAction::List { staged, path } => {
            let hunks = srcmgr_git::hunks(&session.installation, &session.identity, &path, staged)
                .map_err(|error| error.to_string())?;
            for (index, hunk) in hunks.iter().enumerate() {
                println!("hunk {}", index + 1);
                print!("{hunk}");
            }
        }
        HunkAction::Apply { path, number, unstage } => {
            srcmgr_git::apply_hunk(&session.installation, &session.identity, &path, number, unstage)
                .map_err(|error| error.to_string())?;
            println!("{} hunk {}", if unstage { "unstaged" } else { "staged" }, number + 1);
        }
    }
    Ok(())
}

/// Prints the textual diff of one path, unstaged by default and staged with
/// `--staged`. The same trust gate as `status` applies.
fn diff_command(options: &Options, path: &OsStr, staged: bool) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    let text = srcmgr_git::diff(&session.installation, &session.identity, path, staged)
        .map_err(|error| error.to_string())?;
    print!("{text}");
    Ok(())
}

fn profile_root(options: &Options) -> Result<PathBuf, String> {
    options
        .profile
        .clone()
        .or_else(|| env::var_os("MINICON_SRCMGR_PROFILE").map(PathBuf::from))
        .or_else(default_root)
        .ok_or_else(|| "no profile directory: pass --profile".to_owned())
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

/// The repository a verb acts on, from `-C <path>` or the current directory.
///
/// Discovery walks up from the current directory and stops at the first
/// ancestor whose `.git` is a plain directory. It reads filesystem metadata
/// only: nothing here runs Git, and what it finds is a candidate that still
/// goes through the trust gate in [`open_trusted`]. An ancestor is never
/// *followed into* in the sense of trusting it; it is only where the candidate
/// is looked for, which is where `git` would look.
fn worktree_argument(options: &Options) -> Result<PathBuf, String> {
    match &options.directory {
        // git accepts `-C ""` as "do not change directory".
        Some(path) if !path.as_os_str().is_empty() => return Ok(path.clone()),
        _ => {}
    }
    let start = env::current_dir().map_err(|error| format!("cannot read the current directory: {error}"))?;
    let mut candidate = Some(start.as_path());
    while let Some(directory) = candidate {
        if let Ok(metadata) = std::fs::symlink_metadata(directory.join(".git"))
            && metadata.is_dir()
            && !metadata.file_type().is_symlink()
        {
            return Ok(directory.to_path_buf());
        }
        candidate = directory.parent();
    }
    Err("not a Git repository (or any parent): no plain .git directory was found from the current directory; pass -C <path>".to_owned())
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
        .map_err(|error| format!("{error}; run `minicon-srcmgr trust grant -C <path>` after review"))?;

    let executable = refuse_repository_local_executable(&selected_git(options)?, &worktree)?;
    let installation = GitInstallation::probe(executable).map_err(|e| e.to_string())?;
    let backend = GitBackend::new(installation.clone());
    if let CapabilityState::Unavailable { reason } | CapabilityState::Unsupported { reason } =
        backend.capability(Capability::ReadStatus)
    {
        return Err(format!("status is unavailable: {reason}"));
    }
    let identity = locate(&installation, &worktree).map_err(|error| error.to_string())?;
    if identity.common_dir != git_dir || identity.worktree_root != worktree {
        return Err("discovery resolved an unapproved repository/worktree location; no mutation was requested".to_owned());
    }
    Ok(Session { installation, identity })
}

fn selected_git(options: &Options) -> Result<PathBuf, String> {
    if let Some(path) = &options.git { return Ok(path.clone()); }
    if let Some(path) = open_profile_store(options)?.git_executable().map_err(|e| e.to_string())? {
        return Ok(PathBuf::from(path.as_os_str()));
    }
    find_git_on_path().ok_or("No installed Git was found. Open Git settings to select Git 2.43 or newer, or pass --git".into())
}

/// `status [--porcelain[=v1|v2]] [-z] [--json]`.
///
/// `--porcelain` is Git's own output, byte for byte: the product already reads
/// `status --porcelain=v2 -z --branch` for its own snapshot, so that command's
/// bytes are passed through rather than rebuilt from the coarser snapshot.
/// Without `--porcelain` the text is the product's own listing and `--json` is
/// its document.
fn status(options: &Options, porcelain: Option<Porcelain>, null: bool, json_output: bool) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    if let Some(version) = porcelain {
        let mut arguments = vec!["status".to_owned(), format!("--porcelain={}", match version { Porcelain::V1 => "v1", Porcelain::V2 => "v2" })];
        if null {
            arguments.push("-z".to_owned());
        }
        arguments.push("--untracked-files=normal".to_owned());
        if version == Porcelain::V2 {
            arguments.push("--branch".to_owned());
        }
        let references: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = srcmgr_git::run_raw(&session.installation, &session.identity, &references).map_err(|error| error.to_string())?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(&output).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;
    let mut stdout = io::stdout().lock();
    if json_output {
        let mut bytes = json::to_vec_pretty(&status_json(&snapshot));
        bytes.push(b'\n');
        stdout.write_all(&bytes).map_err(|error| error.to_string())?;
    } else {
        stdout.write_all(status_text(&snapshot).as_bytes()).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// `trust grant|revoke`: metadata only. No Git runs here.
pub(crate) fn trust(options: &Options, action: TrustAction) -> Result<(), String> {
    let worktree = canonical_worktree(&worktree_argument(options)?)?;
    let git_dir = git_dir_of(&worktree)?;
    let (path, mut store) = trust_store(options)?;
    match action {
        TrustAction::Grant => {
            let digest = fingerprint(&git_dir, &worktree).map_err(|error| error.to_string())?;
            store.grant(git_dir.clone(), digest.clone());
            store.save().map_err(|error| error.to_string())?;
            println!("trusted {} (fingerprint {})", display_path(&git_dir), &digest[..12]);
            println!("hooks, filters and helpers configured there may now run as you; re-grant after any change");
        }
        TrustAction::Revoke => {
            let removed = store.revoke(&git_dir);
            store.save().map_err(|error| error.to_string())?;
            if removed {
                println!("revoked trust for {}", display_path(&git_dir));
            } else {
                println!("no grant existed for {} (store: {})", display_path(&git_dir), path.display());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Change {
    Stage,
    Unstage,
}

fn change_paths(options: &Options, change: Change, paths: &[OsString]) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let session = open_trusted(options, &worktree)?;
    let snapshot = read_status(&session.installation, &session.identity, Generation(1))
        .map_err(|error| error.to_string())?;

    // Only paths the status read reports as changed may be targeted; the
    // paths are literal, never patterns.
    for path in paths {
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
        Change::Stage => stage(&session.installation, &session.identity, paths),
        Change::Unstage => unstage(&session.installation, &session.identity, paths),
    }
    .map_err(|error| error.to_string())?;
    println!("{} {} path(s)", match change { Change::Stage => "staged", Change::Unstage => "unstaged" }, paths.len());
    Ok(())
}

/// Commits the index. The message comes from `-m` or, when that is absent, from
/// the saved draft for this repository. A successful commit clears the draft.
/// A refused commit leaves the draft in place.
pub(crate) fn commit_command(options: &Options, message: Option<&OsStr>) -> Result<(), String> {
    // Resolve the passive preference before taking the writer used for the
    // draft/journal; installed-Git lookup must not recursively open it.
    let mut selected = options.clone();
    selected.git = Some(selected_git(options)?);
    let options = &selected;
    let worktree = worktree_argument(options)?;
    let key = repository_key(&worktree)?;
    let mut store = open_profile_store(options)?;
    let message = match message {
        Some(given) => given.to_str().ok_or("the commit message must be valid UTF-8")?.to_owned(),
        None => store.get_draft(BACKEND_GIT, &key).map_err(|e| e.to_string())?.ok_or("commit needs -m <message> or a saved draft")?,
    };
    if message.trim().is_empty() { return Err("the commit message is empty".into()); }
    let session = open_trusted(options, &worktree)?;
    let review = srcmgr_git::StagedReview::read(&session.installation, &session.identity).map_err(|e| e.to_string())?;
    if let Some(reason) = &review.blocked { return Err(reason.clone()); }
    store.begin_git_commit(&key, &message, review.binding.head.as_deref()).map_err(|e| e.to_string())?;
    let result = review.commit(&session.installation, &session.identity, &message);
    drop(store);
    match result {
        Ok(srcmgr_git::ReviewedCommit::Observed { oid, warning }) => {
            println!("committed {}", short(&oid));
            if let Some(warning) = &warning { eprintln!("Observed commit warning: {warning}; submitted draft retained"); }
            let cleanup = (|| {
                let mut store = open_profile_store(options)?;
                store.finish_git_commit(&key).map_err(|e| e.to_string())?;
                if warning.is_none() { store.delete_draft(BACKEND_GIT, &key).map_err(|e| e.to_string())?; }
                Ok::<(), String>(())
            })();
            if let Err(error) = cleanup { eprintln!("Commit {oid} was observed; draft/attempt cleanup unavailable: {error}. Do not resubmit."); }
            Ok(())
        }
        Ok(srcmgr_git::ReviewedCommit::Unknown(reason)) => Err(format!("{reason} Submitted draft and pending attempt were saved. Open the GUI and Refresh before retrying.")),
        Err(error) => {
            let cleanup = open_profile_store(options).and_then(|mut store| store.finish_git_commit(&key).map_err(|e| e.to_string()));
            Err(format!("{error}; submitted draft retained; hooks may have changed index/worktree{}", cleanup.err().map_or_else(String::new, |error| format!("; reconciliation pending: {error}"))))
        }
    }
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

/// `draft save -m <message> | show | clear`. Drafts store text only and never
/// run Git, so they need no trust grant.
fn draft_command(options: &Options, action: DraftAction, message: Option<&OsStr>) -> Result<(), String> {
    let worktree = worktree_argument(options)?;
    let key = repository_key(&worktree)?;
    let mut store = open_profile_store(options)?;
    match action {
        DraftAction::Save => {
            let message = message.ok_or("draft save needs -m <message>")?.to_str().ok_or("the draft must be valid UTF-8")?;
            store.put_draft(BACKEND_GIT, &key, message).map_err(|error| error.to_string())?;
            println!("draft saved");
        }
        DraftAction::Show => match store.get_draft(BACKEND_GIT, &key).map_err(|error| error.to_string())? {
            Some(message) => print!("{message}"),
            None => println!("no draft"),
        },
        DraftAction::Clear => {
            store.delete_draft(BACKEND_GIT, &key).map_err(|error| error.to_string())?;
            println!("draft cleared");
        }
    }
    Ok(())
}

fn refuse_repository_local_executable(path: &Path, worktree: &Path) -> Result<PathBuf, String> {
    let executable = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve the git executable: {error}"))?;
    if executable.starts_with(worktree) {
        return Err("the selected git executable lies inside the repository being inspected; choose an installed Git".to_owned());
    }
    Ok(executable)
}

/// Finds the first `git` on PATH. Only the process environment is consulted;
/// the current directory and any repository are never searched.
fn find_git_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["git.exe"] } else { &["git"] };
    let path = env::var_os("PATH")?;
    env::split_paths(&path).filter(|directory| directory.is_absolute()).find_map(|directory| {
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
