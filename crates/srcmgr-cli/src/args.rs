//! Git-compatible command-line parsing for `minicon-srcmgr` ([PRD 01]).
//!
//! `git` puts the subcommand first and accepts its global options only before
//! it; `-C <path>` runs as if started in that directory, and without it the
//! repository is discovered from the current directory. This module parses one
//! invocation into a [`Parsed`] without touching the filesystem, so every rule
//! about `-C`, `--` and option order is a pure unit case.
//!
//! Options are per verb. A verb rejects an option it does not know by name
//! rather than silently ignoring it, because an ignored `--force` is worse than
//! a usage error.
//!
//! Behaviour that was measured against `git` 2.47 before being written here,
//! not assumed:
//!
//! - `-C` is accepted before the subcommand only; `git status -C .` is an
//!   unknown switch.
//! - `-C ""` is accepted and changes nothing.
//! - `--` ends options; a word that begins with `-` after it is a path.
//! - An unknown option is an error with the verb's usage and a non-zero exit.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// One parsed invocation: the global options, the verb and what it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    /// `--profile DIR`, or `MINICON_SRCMGR_PROFILE` when absent.
    pub profile: Option<PathBuf>,
    /// `--git EXE`.
    pub git: Option<PathBuf>,
    /// `-C <path>`: run as if started there. The last occurrence wins.
    pub directory: Option<PathBuf>,
    /// The verb with its own arguments.
    pub command: Command,
}

/// The verbs. Repository verbs carry their operands only; the repository
/// itself comes from [`Parsed::directory`] or the current directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `status [--porcelain[=v1|v2]] [-z] [--json]`
    Status { porcelain: Option<Porcelain>, null: bool, json: bool },
    /// `trust grant|revoke`
    Trust { action: TrustAction },
    /// `stage [--] <path>...`
    Stage { paths: Vec<OsString> },
    /// `unstage [--] <path>...`
    Unstage { paths: Vec<OsString> },
    /// `commit [-m <message>]`
    Commit { message: Option<OsString> },
    /// `draft save -m <message> | show | clear`
    Draft { action: DraftAction, message: Option<OsString> },
    /// `gui`
    Gui,
    /// `diff [--staged] [--] <path>`
    Diff { staged: bool, path: OsString },
    /// `branch [--list] | -m [<old>] <new> | create <name> | switch <name>`
    Branch { action: BranchAction },
    /// `remote list|tracking [--json] | add|set-url|rename|remove ... [--yes]`
    Remote { action: RemoteAction },
    /// `fetch [--json] <remote>`
    Fetch { remote: String, json: bool },
    /// `clone [--branch <b>] [--depth <n>] <url> <dir>`
    Clone { url: String, directory: PathBuf, branch: Option<String>, depth: Option<u32> },
    /// `pull`
    Pull,
    /// `push <remote>`
    Push { remote: String },
    /// `hunk list [--staged] [--] <path> | hunk stage|unstage [--] <path> <n>`
    Hunk { action: HunkAction },
    /// `log [--skip <n>] [--limit <n>]`
    Log { skip: usize, limit: Option<usize> },
    /// `check-ignore [--] <path>`
    CheckIgnore { path: OsString },
    /// `config get|set <key> [value]`
    Config { action: ConfigAction },
    /// `--version`, `-V` or `version`
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Porcelain {
    V1,
    V2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustAction {
    Grant,
    Revoke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftAction {
    Save,
    Show,
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchAction {
    List,
    /// `-m [<old>] <new>`
    Rename { old: Option<String>, new: String },
    Create(String),
    Switch(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAction {
    List { json: bool },
    Tracking { json: bool },
    Change { change: RemoteChange, yes: bool },
}

/// The reviewed remote configuration changes, mirroring `srcmgr_git`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteChange {
    Add { name: String, url: String, push_url: Option<String> },
    SetUrl { name: String, url: String, push: bool },
    Rename { from: String, to: String },
    Remove { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HunkAction {
    List { staged: bool, path: OsString },
    Apply { path: OsString, number: usize, unstage: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigAction {
    Get { key: String },
    Set { key: String, value: OsString },
}

/// Why an invocation was refused. The message is what the user reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage(pub String);

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

/// Splits one invocation. `arguments` excludes the program name.
pub fn parse<I, S>(arguments: I) -> Result<Parsed, Usage>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let words: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    let mut parsed = Parsed { profile: None, git: None, directory: None, command: Command::Version };
    let mut index = 0;

    // Global options come before the verb, and only there.
    while let Some(word) = words.get(index).map(|word| word.as_os_str()) {
        match word.to_str() {
            Some("--profile") => parsed.profile = Some(PathBuf::from(value(&words, &mut index, "--profile needs a directory")?)),
            Some("--git") => parsed.git = Some(PathBuf::from(value(&words, &mut index, "--git needs an executable path")?)),
            // git accepts `-C ""` as "do not change directory".
            Some("-C") => parsed.directory = Some(PathBuf::from(value(&words, &mut index, "-C needs a directory")?)),
            Some(word) if word.starts_with("-C") && word.len() > 2 => {
                parsed.directory = Some(PathBuf::from(OsStr::new(&word[2..])));
            }
            Some("-h") | Some("--help") => return Err(Usage(USAGE.to_owned())),
            Some("--version") | Some("-V") if words.len() == 1 => {
                parsed.command = Command::Version;
                return Ok(parsed);
            }
            Some(word) if word.starts_with('-') && word != "--" => {
                return Err(Usage(format!("unknown option '{word}'; global options come before the command, as in git")));
            }
            _ => break,
        }
        index += 1;
    }

    let Some(verb) = words.get(index).and_then(|word| word.to_str()) else {
        return Err(Usage(USAGE.to_owned()));
    };
    let rest = &words[index + 1..];
    parsed.command = match verb {
        "version" if rest.is_empty() => Command::Version,
        "status" => status(rest)?,
        "trust" => trust(rest)?,
        "stage" => Command::Stage { paths: operands(rest, "stage <path>...")? },
        "unstage" => Command::Unstage { paths: operands(rest, "unstage <path>...")? },
        "commit" => commit(rest)?,
        "draft" => draft(rest)?,
        "gui" if rest.is_empty() => Command::Gui,
        "diff" => diff(rest)?,
        "branch" => branch(rest)?,
        "remote" => remote(rest)?,
        "fetch" => fetch(rest)?,
        "clone" => clone(rest)?,
        "pull" if rest.is_empty() => Command::Pull,
        "push" => match rest {
            [remote] => Command::Push { remote: text(remote, "push <remote>")? },
            _ => return Err(Usage("push <remote>".into())),
        },
        "hunk" => hunk(rest)?,
        "log" => log(rest)?,
        "check-ignore" => check_ignore(rest)?,
        "config" => config(rest)?,
        // The removed private spelling names its replacement instead of
        // failing as an unknown word.
        "history" => return Err(Usage(REPLACED_HISTORY.into())),
        "setting" => return Err(Usage(REPLACED_SETTING.into())),
        "ignore" => return Err(Usage(REPLACED_IGNORE.into())),
        other => return Err(Usage(format!("'{other}' is not a minicon-srcmgr command. {USAGE_SHORT}"))),
    };
    Ok(parsed)
}

/// Reads the next word as the value of the option at `index`.
fn value(words: &[OsString], index: &mut usize, missing: &str) -> Result<OsString, Usage> {
    let word = words.get(*index + 1).ok_or_else(|| Usage(missing.to_owned()))?;
    *index += 1;
    Ok(word.clone())
}

fn text(word: &OsStr, missing: &str) -> Result<String, Usage> {
    word.to_str().map(str::to_owned).ok_or_else(|| Usage(missing.to_owned()))
}

fn number<T: std::str::FromStr>(word: &OsStr, flag: &str) -> Result<T, Usage> {
    word.to_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| Usage(format!("{flag} needs a number")))
}

/// Splits a verb's words into options and operands, each in the order written.
/// After `--` every word is an operand, which is the whole point of `--`.
/// Without it a word beginning with `-` is an option wherever it stands and
/// every other word is an operand, so `diff a.txt --staged` and
/// `diff --staged a.txt` parse alike.
fn split(words: &[OsString]) -> (Vec<&OsStr>, Vec<&OsStr>) {
    let mut options = Vec::new();
    let mut operands = Vec::new();
    let mut past_separator = false;
    for word in words {
        let word = word.as_os_str();
        if past_separator {
            operands.push(word);
        } else if word == "--" {
            past_separator = true;
        } else if word.to_str().is_some_and(|text| text.starts_with('-')) {
            options.push(word);
        } else {
            operands.push(word);
        }
    }
    (options, operands)
}

/// The path operands of a verb that takes paths and no options of its own.
fn operands(words: &[OsString], usage: &str) -> Result<Vec<OsString>, Usage> {
    let (options, operands) = split(words);
    if let Some(option) = options.first() {
        return Err(Usage(format!(
            "unknown option '{}'; write -- before a path that begins with -",
            option.to_string_lossy()
        )));
    }
    if operands.is_empty() {
        return Err(Usage(usage.to_owned()));
    }
    Ok(operands.into_iter().map(OsString::from).collect())
}

/// Refuses every option except the known ones. A word that is not UTF-8 is
/// refused too, because it is neither a known option nor a path here.
fn only_option(options: &[&OsStr], known: &[&str], verb: &str) -> Result<(), Usage> {
    for word in options {
        match word.to_str() {
            Some(option) if known.contains(&option) => {}
            Some(other) => return Err(Usage(format!("unknown option '{other}' for {verb}"))),
            None => return Err(Usage(format!("{verb} takes UTF-8 options"))),
        }
    }
    Ok(())
}

fn status(words: &[OsString]) -> Result<Command, Usage> {
    let mut porcelain = None;
    let mut null = false;
    let mut json = false;
    for word in words {
        match word.to_str() {
            Some("--porcelain") | Some("--porcelain=v1") => porcelain = Some(Porcelain::V1),
            Some("--porcelain=v2") => porcelain = Some(Porcelain::V2),
            Some("-z") => null = true,
            Some("--json") => json = true,
            Some(other) => return Err(Usage(format!("unknown option '{other}' for status"))),
            None => return Err(Usage("status [--porcelain[=v2]] [-z] [--json]".into())),
        }
    }
    if json && porcelain.is_some() {
        return Err(Usage("--json and --porcelain are different output formats; choose one".into()));
    }
    Ok(Command::Status { porcelain, null, json })
}

fn trust(words: &[OsString]) -> Result<Command, Usage> {
    match words {
        [action] => match action.to_str() {
            Some("grant") => Ok(Command::Trust { action: TrustAction::Grant }),
            Some("revoke") => Ok(Command::Trust { action: TrustAction::Revoke }),
            Some(other) => Err(Usage(format!("trust grant|revoke, not '{other}'"))),
            None => Err(Usage("trust grant|revoke".into())),
        },
        _ => Err(Usage("trust grant|revoke".into())),
    }
}

fn commit(words: &[OsString]) -> Result<Command, Usage> {
    let mut message = None;
    let mut index = 0;
    while index < words.len() {
        match words[index].to_str() {
            Some("-m") | Some("--message") => message = Some(value(words, &mut index, "-m needs a message")?),
            Some(other) => return Err(Usage(format!("unknown option '{other}' for commit"))),
            None => return Err(Usage("commit [-m <message>]".into())),
        }
        index += 1;
    }
    Ok(Command::Commit { message })
}

fn draft(words: &[OsString]) -> Result<Command, Usage> {
    match words {
        [action] => match action.to_str() {
            Some("show") => Ok(Command::Draft { action: DraftAction::Show, message: None }),
            Some("clear") => Ok(Command::Draft { action: DraftAction::Clear, message: None }),
            Some(other) => Err(Usage(format!("{DRAFT_USAGE}, not '{other}'"))),
            None => Err(Usage(DRAFT_USAGE.into())),
        },
        [action, flag, message] if action == "save" && flag == "-m" => Ok(Command::Draft {
            action: DraftAction::Save,
            message: Some(message.clone()),
        }),
        _ => Err(Usage(DRAFT_USAGE.into())),
    }
}

fn diff(words: &[OsString]) -> Result<Command, Usage> {
    const USAGE_LINE: &str = "diff [--staged] -- <path>";
    let (options, operands) = split(words);
    only_option(&options, &["--staged"], "diff")?;
    match operands.as_slice() {
        [path] => Ok(Command::Diff { staged: options.iter().any(|word| *word == "--staged"), path: (*path).to_owned() }),
        _ => Err(Usage(USAGE_LINE.into())),
    }
}

fn branch(words: &[OsString]) -> Result<Command, Usage> {
    const USAGE_LINE: &str = "branch [--list] | -m [<old>] <new> | create <name> | switch <name>";
    // Every word of this verb is a name or a known option, so non-UTF-8 is a
    // refusal rather than a lossy branch name.
    let words: Vec<&str> = words.iter().map(|word| word.to_str()).collect::<Option<_>>().ok_or_else(|| Usage(USAGE_LINE.into()))?;
    let rename = |old: Option<&str>, new: &str| {
        text(OsStr::new(new), USAGE_LINE).map(|new| Command::Branch {
            action: BranchAction::Rename {
                old: old.map(|old| old.to_owned()),
                new,
            },
        })
    };
    match words.as_slice() {
        // `git branch` and `git branch --list` are the Git spellings; `list`
        // is accepted beside `create` and `switch`, which are SrcMgr's own.
        [] | ["--list"] | ["-l"] | ["list"] => Ok(Command::Branch { action: BranchAction::List }),
        ["-m"] | ["--move"] => Err(Usage("branch -m [<old>] <new>".into())),
        ["-m", new] | ["--move", new] => rename(None, new),
        ["-m", old, new] | ["--move", old, new] => rename(Some(old), new),
        ["create", name] => Ok(Command::Branch { action: BranchAction::Create((*name).to_owned()) }),
        ["switch", name] => Ok(Command::Branch { action: BranchAction::Switch((*name).to_owned()) }),
        [first, ..] if first.starts_with('-') => Err(Usage(format!(
            "unknown option '{first}' for branch; supported: --list, -m [<old>] <new>"
        ))),
        _ => Err(Usage(USAGE_LINE.into())),
    }
}

fn remote(words: &[OsString]) -> Result<Command, Usage> {
    let mut json = false;
    let mut yes = false;
    let mut push = false;
    let mut push_url = None;
    let mut plain = Vec::new();
    let mut index = 0;
    while index < words.len() {
        match words[index].to_str() {
            Some("--json") => json = true,
            Some("--yes") => yes = true,
            Some("--push") => push = true,
            Some("--push-url") => push_url = Some(text(&value(words, &mut index, "--push-url needs a URL")?, "--push-url needs a URL")?),
            Some(other) if other.starts_with('-') => return Err(Usage(format!("unknown option '{other}' for remote"))),
            _ => plain.push(words[index].clone()),
        }
        index += 1;
    }
    let change = |change| Ok(Command::Remote { action: RemoteAction::Change { change, yes } });
    match &plain[..] {
        [action] if action == "list" && !yes => Ok(Command::Remote { action: RemoteAction::List { json } }),
        [action] if action == "tracking" && !yes => Ok(Command::Remote { action: RemoteAction::Tracking { json } }),
        [action, name, url] if action == "add" => change(RemoteChange::Add {
            name: text(name, "remote add <name> <url>")?,
            url: text(url, "remote add <name> <url>")?,
            push_url,
        }),
        [action, name, url] if action == "set-url" => change(RemoteChange::SetUrl {
            name: text(name, "remote set-url <name> <url>")?,
            url: text(url, "remote set-url <name> <url>")?,
            push,
        }),
        [action, from, to] if action == "rename" => change(RemoteChange::Rename {
            from: text(from, "remote rename <old> <new>")?,
            to: text(to, "remote rename <old> <new>")?,
        }),
        [action, name] if action == "remove" => change(RemoteChange::Remove { name: text(name, "remote remove <name>")? }),
        _ => Err(Usage(REMOTE_USAGE.into())),
    }
}

fn fetch(words: &[OsString]) -> Result<Command, Usage> {
    let mut json = false;
    let mut remote = None;
    for word in words {
        match word.to_str() {
            Some("--json") => json = true,
            Some(other) if other.starts_with('-') => return Err(Usage(format!("unknown option '{other}' for fetch"))),
            _ if remote.is_none() => remote = Some(text(word, "fetch <remote>")?),
            _ => return Err(Usage("fetch [--json] <remote>".into())),
        }
    }
    remote
        .map(|remote| Command::Fetch { remote, json })
        .ok_or_else(|| Usage("fetch [--json] <remote>".into()))
}

fn clone(words: &[OsString]) -> Result<Command, Usage> {
    let mut branch = None;
    let mut depth = None;
    let mut plain = Vec::new();
    let mut index = 0;
    while index < words.len() {
        match words[index].to_str() {
            Some("--branch") | Some("-b") => branch = Some(text(&value(words, &mut index, "--branch needs a name")?, "--branch needs a name")?),
            Some("--depth") => depth = Some(number(&value(words, &mut index, "--depth needs a number")?, "--depth")?),
            Some(other) if other.starts_with('-') => return Err(Usage(format!("unknown option '{other}' for clone"))),
            _ => plain.push(words[index].clone()),
        }
        index += 1;
    }
    match &plain[..] {
        [url, directory] => Ok(Command::Clone {
            url: text(url, "clone <url> <dir>")?,
            directory: PathBuf::from(directory),
            branch,
            depth,
        }),
        _ => Err(Usage("clone [--branch <b>] [--depth <n>] <url> <dir>".into())),
    }
}

fn hunk(words: &[OsString]) -> Result<Command, Usage> {
    let (options, operands) = split(words);
    only_option(&options, &["--staged"], "hunk")?;
    let staged = options.iter().any(|word| *word == "--staged");
    // `list` takes --staged; `stage`/`unstage` take a number instead.
    match operands.as_slice() {
        [action, path] if *action == "list" => Ok(Command::Hunk {
            action: HunkAction::List { staged, path: (*path).to_owned() },
        }),
        [action, path, given] if (*action == "stage" || *action == "unstage") && !staged => {
            // The command line numbers hunks from one, like `git add -p`; the
            // Git layer numbers them from zero.
            let shown = number::<usize>(given, "the hunk number")?;
            if shown == 0 {
                return Err(Usage("the hunk number starts at 1".into()));
            }
            Ok(Command::Hunk {
                action: HunkAction::Apply {
                    path: (*path).to_owned(),
                    number: shown - 1,
                    unstage: *action == "unstage",
                },
            })
        }
        _ => Err(Usage(HUNK_USAGE.into())),
    }
}

fn log(words: &[OsString]) -> Result<Command, Usage> {
    let mut skip = 0;
    let mut limit = None;
    let mut index = 0;
    while index < words.len() {
        match words[index].to_str() {
            Some("--skip") => skip = number(&value(words, &mut index, "--skip needs a number")?, "--skip")?,
            Some("--limit") => limit = Some(number(&value(words, &mut index, "--limit needs a number")?, "--limit")?),
            Some(word) if word.starts_with("--skip=") => skip = number(OsStr::new(&word[7..]), "--skip")?,
            Some(word) if word.starts_with("--limit=") => limit = Some(number(OsStr::new(&word[8..]), "--limit")?),
            Some(other) => return Err(Usage(format!("unknown option '{other}' for log"))),
            None => return Err(Usage("log [--skip <n>] [--limit <n>]".into())),
        }
        index += 1;
    }
    if limit == Some(0) {
        return Err(Usage("--limit must be at least 1".into()));
    }
    Ok(Command::Log { skip, limit })
}

fn check_ignore(words: &[OsString]) -> Result<Command, Usage> {
    match operands(words, "check-ignore -- <path>")?.as_slice() {
        [path] => Ok(Command::CheckIgnore { path: path.clone() }),
        _ => Err(Usage("check-ignore -- <path>".into())),
    }
}

fn config(words: &[OsString]) -> Result<Command, Usage> {
    match words {
        [action, key] if action == "get" => Ok(Command::Config {
            action: ConfigAction::Get { key: text(key, "config get <key>")? },
        }),
        [action, key, value] if action == "set" => Ok(Command::Config {
            action: ConfigAction::Set { key: text(key, "config set <key> [value]")?, value: value.clone() },
        }),
        _ => Err(Usage(CONFIG_USAGE.into())),
    }
}

const USAGE: &str = "usage: minicon-srcmgr [--profile DIR] [--git EXE] [-C <path>] <command> [<args>]

Commands (git spelling first; SrcMgr verbs keep their own names):
   status      show the working tree status
   stage       add literal paths to the index
   unstage     remove literal paths from the index
   commit      record the staged index
   branch      list, rename, create or switch branches
   log         list commits
   diff        show one path's changes
   clone       clone a repository (checkout deferred)
   fetch       fetch one remote
   pull        fast-forward pull
   push        push the current branch to one remote
   remote      list remotes or review a remote configuration change
   hunk        list, stage or unstage one hunk
   check-ignore  report which rule ignores a path
   draft       save, show or clear the commit-message draft
   config      get or set a SrcMgr preference
   trust       grant or revoke repository trust
   gui         open the workspace window
   control     the management surface (planned, 0.2.3)

'minicon-srcmgr <command> -h' prints that command's options.";
const USAGE_SHORT: &str = "run 'minicon-srcmgr -h' for the command list.";
const DRAFT_USAGE: &str = "draft save -m <message> | show | clear";
const REMOTE_USAGE: &str =
    "remote list|tracking [--json] | remote add <name> <url> [--push-url <url>] | remote set-url <name> <url> [--push] | remote rename <old> <new> | remote remove <name>   [--yes applies; otherwise preview only]";
const HUNK_USAGE: &str = "hunk list [--staged] -- <path> | hunk stage|unstage -- <path> <number>";
const CONFIG_USAGE: &str = "config get <key> | config set <key> <value>";
const REPLACED_HISTORY: &str = "'history' was removed; use 'log'. Example: minicon-srcmgr log -C <path> --limit 20";
const REPLACED_SETTING: &str = "'setting' was removed; use 'config'. Example: minicon-srcmgr config get history.limit";
const REPLACED_IGNORE: &str = "'ignore' was removed; use 'check-ignore'. Example: minicon-srcmgr check-ignore -C <path> -- <path>";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Parsed, Usage> {
        super::parse(words.iter().copied())
    }

    #[test]
    fn global_options_come_before_the_command_as_in_git() {
        let parsed = parse(&["--profile", "p", "--git", "g", "-C", "repo", "status"]).unwrap();
        assert_eq!(parsed.profile.as_deref(), Some(std::path::Path::new("p")));
        assert_eq!(parsed.git.as_deref(), Some(std::path::Path::new("g")));
        assert_eq!(parsed.directory.as_deref(), Some(std::path::Path::new("repo")));
        assert_eq!(parsed.command, Command::Status { porcelain: None, null: false, json: false });
        // Measured against git 2.47: `-C` after the subcommand is unknown.
        assert!(parse(&["status", "-C", "repo"]).is_err());
    }

    #[test]
    fn an_empty_directory_argument_changes_nothing_as_in_git() {
        let parsed = parse(&["-C", "", "status"]).unwrap();
        assert_eq!(parsed.directory.as_deref(), Some(std::path::Path::new("")));
        assert!(parse(&["-C"]).is_err(), "-C without a value is refused");
    }

    #[test]
    fn the_last_directory_wins_and_the_attached_form_is_accepted() {
        let parsed = parse(&["-C", "one", "-Crepo", "status"]).unwrap();
        assert_eq!(parsed.directory.as_deref(), Some(std::path::Path::new("repo")));
    }

    #[test]
    fn a_dash_path_is_a_path_after_the_separator_and_an_option_before_it() {
        let parsed = parse(&["stage", "--", "-dash.txt", "中文.txt"]).unwrap();
        assert_eq!(parsed.command, Command::Stage { paths: vec!["-dash.txt".into(), "中文.txt".into()] });
        let refused = parse(&["stage", "-dash.txt"]).unwrap_err();
        assert!(refused.0.contains("write -- before a path"), "{refused}");
        // A path that merely contains a dash, not at the start, needs no separator.
        assert_eq!(parse(&["stage", "a-b.txt"]).unwrap().command, Command::Stage { paths: vec!["a-b.txt".into()] });
    }

    #[test]
    fn every_verb_reads_its_own_options_and_rejects_the_rest() {
        assert_eq!(parse(&["status", "--porcelain=v2", "-z"]).unwrap().command, Command::Status { porcelain: Some(Porcelain::V2), null: true, json: false });
        assert_eq!(parse(&["status", "--porcelain"]).unwrap().command, Command::Status { porcelain: Some(Porcelain::V1), null: false, json: false });
        assert!(parse(&["status", "--json", "--porcelain"]).is_err(), "two output formats");
        assert!(parse(&["status", "--force"]).is_err());
        assert_eq!(parse(&["commit", "-m", "one"]).unwrap().command, Command::Commit { message: Some("one".into()) });
        assert_eq!(parse(&["commit"]).unwrap().command, Command::Commit { message: None });
        assert_eq!(parse(&["branch"]).unwrap().command, Command::Branch { action: BranchAction::List });
        assert_eq!(parse(&["branch", "list"]).unwrap().command, Command::Branch { action: BranchAction::List });
        assert_eq!(parse(&["branch", "--list"]).unwrap().command, Command::Branch { action: BranchAction::List });
        assert_eq!(parse(&["branch", "-m", "main", "develop"]).unwrap().command, Command::Branch { action: BranchAction::Rename { old: Some("main".into()), new: "develop".into() } });
        assert_eq!(parse(&["branch", "-m", "develop"]).unwrap().command, Command::Branch { action: BranchAction::Rename { old: None, new: "develop".into() } });
        assert_eq!(parse(&["log", "--skip", "2", "--limit", "5"]).unwrap().command, Command::Log { skip: 2, limit: Some(5) });
        assert_eq!(parse(&["log", "--skip=1"]).unwrap().command, Command::Log { skip: 1, limit: None });
        assert!(parse(&["log", "--limit", "0"]).is_err());
        assert!(parse(&["log", "--skip", "x"]).is_err());
        assert_eq!(parse(&["hunk", "unstage", "--", "a.txt", "3"]).unwrap().command, Command::Hunk { action: HunkAction::Apply { path: "a.txt".into(), number: 2, unstage: true } });
        assert!(parse(&["hunk", "stage", "a.txt", "0"]).is_err(), "hunks are numbered from 1");
        assert_eq!(parse(&["remote", "tracking", "--json"]).unwrap().command, Command::Remote { action: RemoteAction::Tracking { json: true } });
        assert_eq!(parse(&["remote", "rename", "a", "b", "--yes"]).unwrap().command, Command::Remote { action: RemoteAction::Change { change: RemoteChange::Rename { from: "a".into(), to: "b".into() }, yes: true } });
        assert_eq!(parse(&["fetch", "--json", "origin"]).unwrap().command, Command::Fetch { remote: "origin".into(), json: true });
        assert!(parse(&["fetch"]).is_err(), "fetch names one remote");
        assert_eq!(parse(&["clone", "--branch", "main", "--depth", "1", "u", "d"]).unwrap().command, Command::Clone { url: "u".into(), directory: "d".into(), branch: Some("main".into()), depth: Some(1) });
        assert!(parse(&["clone", "only-one"]).is_err());
        assert_eq!(parse(&["config", "set", "history.limit", "5"]).unwrap().command, Command::Config { action: ConfigAction::Set { key: "history.limit".into(), value: "5".into() } });
        assert!(parse(&["config", "set"]).is_err());
    }

    #[test]
    fn the_removed_spelling_names_its_replacement() {
        assert!(parse(&["history", "repo"]).unwrap_err().0.contains("'log'"));
        assert!(parse(&["setting", "get", "history.limit"]).unwrap_err().0.contains("'config'"));
        assert!(parse(&["ignore", "repo", "a.txt"]).unwrap_err().0.contains("'check-ignore'"));
        let unknown = parse(&["frobnicate"]).unwrap_err();
        assert!(unknown.0.contains("not a minicon-srcmgr command"), "{unknown}");
    }

    #[test]
    fn version_is_a_command_and_a_flag() {
        assert_eq!(parse(&["--version"]).unwrap().command, Command::Version);
        assert_eq!(parse(&["-V"]).unwrap().command, Command::Version);
        assert_eq!(parse(&["version"]).unwrap().command, Command::Version);
        assert!(parse(&["--version", "status"]).is_err());
        assert!(parse(&[]).is_err(), "no command is a usage error, not the GUI");
    }

    #[test]
    fn options_may_sit_before_their_operands_only() {
        // `git` reads options anywhere before `--`; SrcMgr does the same for the
        // verbs that have both options and operands.
        assert_eq!(parse(&["diff", "--staged", "--", "a.txt"]).unwrap().command, Command::Diff { staged: true, path: "a.txt".into() });
        assert_eq!(parse(&["diff", "a.txt", "--staged"]).unwrap().command, Command::Diff { staged: true, path: "a.txt".into() });
        assert_eq!(parse(&["hunk", "list", "a.txt", "--staged"]).unwrap().command, Command::Hunk { action: HunkAction::List { staged: true, path: "a.txt".into() } });
    }
}
