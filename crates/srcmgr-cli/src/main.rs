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

mod window_backend;
mod confirmation;
mod entry;
mod tools;

const USAGE: &str = "usage:
  minicon-srcmgr [--profile DIR] [--git EXE] status <worktree> [--json]
  minicon-srcmgr [--profile DIR] trust grant|revoke <worktree>
  minicon-srcmgr [--profile DIR] [--git EXE] stage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] unstage <worktree> <path>...
  minicon-srcmgr [--profile DIR] [--git EXE] commit <worktree> [-m <message>]
  minicon-srcmgr [--profile DIR] draft <worktree> save -m <message> | show | clear
  minicon-srcmgr [--profile DIR] [--git EXE] gui [worktree]
  minicon-srcmgr [--profile DIR] [--git EXE] diff <worktree> <path> [--staged]
  minicon-srcmgr [--profile DIR] [--git EXE] branch <worktree> list|create|switch <name>
  minicon-srcmgr [--profile DIR] [--git EXE] remote <worktree> list [--json]
  minicon-srcmgr [--profile DIR] [--git EXE] remote <worktree> tracking [--json]
  minicon-srcmgr [--profile DIR] [--git EXE] fetch <worktree> <remote> [--json]
  minicon-srcmgr [--git EXE] clone <url> <destination> [--branch N] [--depth N] [--json]
  minicon-srcmgr [--profile DIR] [--git EXE] remote <worktree> add|set-url|rename|remove ... [--yes]
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
    if options.positional.is_empty() { return gui(&options); }
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
        "remote" => remote_command(&options),
        "fetch" => fetch_command(&options),
        "clone" => clone_command(&options),
        "upstream" => upstream_command(&options),
        "pull" => pull_command(&options),
        "push" => push_command(&options),
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
    let initial = options.positional.get(1).map(|path| NativePath::new(path.clone()));
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

/// `upstream <worktree> set <remote>/<branch> | clear`.
fn upstream_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let action = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    match action {
        "set" => {
            let upstream = options.positional.get(3).and_then(|word| word.to_str()).ok_or(USAGE)?;
            srcmgr_git::set_upstream(&session.installation, &session.identity, upstream)
                .map_err(|error| error.to_string())?;
            println!("upstream set to {upstream}");
        }
        "clear" => {
            srcmgr_git::clear_upstream(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?;
            println!("upstream cleared");
        }
        _ => return Err(USAGE.to_owned()),
    }
    Ok(())
}

/// `pull <worktree>`: fetch facts are assumed current; the merge is fast-forward only.
fn pull_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let session = open_trusted(options, worktree)?;
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
fn push_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let remote = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
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

/// The value after `flag` when present.
fn flag_value<'a>(options: &'a Options, flag: &str) -> Option<&'a str> {
    let position = options.positional.iter().position(|word| word == flag)?;
    options.positional.get(position + 1).and_then(|word| word.to_str())
}

fn argument(options: &Options, index: usize) -> Result<&str, String> {
    options.positional.get(index).and_then(|word| word.to_str()).ok_or_else(|| USAGE.to_owned())
}

/// Builds the reviewed change from the command words. Nothing is applied here.
fn remote_change(options: &Options, action: &str) -> Result<srcmgr_git::RemoteChange, String> {
    Ok(match action {
        "add" => srcmgr_git::RemoteChange::Add {
            name: argument(options, 3)?.to_owned(),
            url: argument(options, 4)?.to_owned(),
            push_url: flag_value(options, "--push-url").map(str::to_owned),
        },
        "set-url" => srcmgr_git::RemoteChange::SetUrl {
            name: argument(options, 3)?.to_owned(),
            url: argument(options, 4)?.to_owned(),
            push: options.positional.iter().any(|word| word == "--push"),
        },
        "rename" => srcmgr_git::RemoteChange::Rename {
            from: argument(options, 3)?.to_owned(),
            to: argument(options, 4)?.to_owned(),
        },
        "remove" => srcmgr_git::RemoteChange::Remove { name: argument(options, 3)?.to_owned() },
        _ => return Err(USAGE.to_owned()),
    })
}

/// `clone <url> <destination>`: reviewed destination and transport, no checkout.
/// A transfer failure keeps whatever was written so it can be inspected.
fn clone_command(options: &Options) -> Result<(), String> {
    let url = argument(options, 1)?.to_owned();
    let destination = PathBuf::from(argument(options, 2)?);
    let depth = match flag_value(options, "--depth") {
        Some(value) => Some(value.parse::<u32>().map_err(|_| "--depth needs a number".to_owned())?),
        None => None,
    };
    let installation = GitInstallation::probe(selected_git(options)?).map_err(|error| error.to_string())?;
    let request = srcmgr_git::CloneRequest {
        url,
        destination,
        branch: flag_value(options, "--branch").map(str::to_owned),
        depth,
    };
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
        format!(
            "{error}; inspect {} for a partial clone before retrying",
            preview.destination.display()
        )
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

/// `remote <worktree> list|tracking`: configured remotes with redacted URLs,
/// and the current branch's distance from the fetched tracking ref.
fn remote_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let action = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    match action {
        "add" | "set-url" | "rename" | "remove" => {
            let change = remote_change(options, action)?;
            let preview = srcmgr_git::preview_remote_change(&session.installation, &session.identity, &change)
                .map_err(|error| error.to_string())?;
            for line in &preview.lines {
                println!("{line}");
            }
            if options.positional.iter().any(|word| word == "--yes") {
                srcmgr_git::apply_remote_change(&session.installation, &session.identity, &change)
                    .map_err(|error| error.to_string())?;
                println!("applied; the repository trust fingerprint changed - review and grant trust again before the next action");
            } else {
                println!("preview only; add --yes to apply");
            }
            Ok(())
        }
        "list" => {
            let remotes = srcmgr_git::list_remotes(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?;
            if options.json {
                let rows = remotes
                    .iter()
                    .map(|remote| {
                        json::object(vec![
                            ("name", remote.name.as_str().into()),
                            ("fetch_url", remote.fetch_url.as_str().into()),
                            ("push_url", json::nullable(remote.push_url.clone())),
                            (
                                "refspecs",
                                JsonValue::Array(
                                    remote
                                        .fetch_refspecs
                                        .iter()
                                        .map(|refspec| refspec.as_str().into())
                                        .collect(),
                                ),
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
        "tracking" => {
            let state = srcmgr_git::tracking(&session.installation, &session.identity)
                .map_err(|error| error.to_string())?;
            if options.json {
                print_json(json::object(vec![
                    ("upstream", json::nullable(state.upstream.clone())),
                    ("ahead", JsonValue::Unsigned(u64::from(state.ahead))),
                    ("behind", JsonValue::Unsigned(u64::from(state.behind))),
                ]))
            } else {
                match &state.upstream {
                    Some(upstream) => {
                        println!("{upstream}  ahead {}  behind {}", state.ahead, state.behind);
                    }
                    None => println!("no upstream"),
                }
                Ok(())
            }
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// `fetch <worktree> <remote>`: one explicit remote, with transfer progress on
/// stderr and the actual refs Git reported on stdout.
fn fetch_command(options: &Options) -> Result<(), String> {
    let worktree = worktree_argument(options, 1)?;
    let remote = options.positional.get(2).and_then(|word| word.to_str()).ok_or(USAGE)?;
    let session = open_trusted(options, worktree)?;
    let cancel = AtomicBool::new(false);
    let sink: srcmgr_git::ProgressSink = Arc::new(|chunk: &str| {
        if let Some(line) = srcmgr_git::progress_line(chunk) {
            eprintln!("{line}");
        }
    });
    let report = srcmgr_git::fetch(
        &session.installation,
        &session.identity,
        remote,
        &cancel,
        Some(&sink),
    )
    .map_err(|error| error.to_string())?;
    if options.json {
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
        print_json(json::object(vec![
            ("remote", report.remote.as_str().into()),
            ("updates", JsonValue::Array(updates)),
        ]))
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
    // Resolve the passive preference before taking the writer used for the
    // draft/journal; installed-Git lookup must not recursively open it.
    let mut selected = options.clone();
    selected.git = Some(selected_git(options)?);
    let options = &selected;
    let worktree = worktree_argument(options, 1)?;
    let key = repository_key(worktree)?;
    let mut store = open_profile_store(options)?;
    let message = match &options.message {
        Some(given) => given.to_str().ok_or("the commit message must be valid UTF-8")?.to_owned(),
        None => store.get_draft(BACKEND_GIT, &key).map_err(|e| e.to_string())?.ok_or("commit needs -m <message> or a saved draft")?,
    };
    if message.trim().is_empty() { return Err("the commit message is empty".into()); }
    let session = open_trusted(options, worktree)?;
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
