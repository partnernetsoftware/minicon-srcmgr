//! Optional user-owned tool handoff, separate from supervised Git operations.
use std::ffi::OsString;
use std::path::{Path, PathBuf, Component};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use srcmgr_domain::{ExternalTool, NativePath, ToolKind};
use srcmgr_ui::Reply;
use crate::{Options, open_profile_store, trust_store, git_dir_of};

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp { canonical: PathBuf, length: u64, modified: SystemTime }

fn stamp(path: &Path) -> Result<FileStamp, String> {
    let canonical = std::fs::canonicalize(path).map_err(|_| "Tool or target is unavailable; review its path again")?;
    let metadata = std::fs::metadata(&canonical).map_err(|_| "Tool or target metadata is unavailable")?;
    if !metadata.is_file() { return Err("Choose an existing regular file".into()); }
    Ok(FileStamp { canonical, length: metadata.len(), modified: metadata.modified().map_err(|_| "Tool or target modification time is unavailable")? })
}

struct Prepared {
    ticket: String,
    kind: ToolKind,
    definition: ExternalTool,
    repository: PathBuf,
    working_directory: PathBuf,
    git_directory: PathBuf,
    trust: String,
    executable: FileStamp,
    file: Option<(NativePath, FileStamp)>,
    arguments: Vec<OsString>,
    expires: Instant,
}

#[derive(Default)]
pub struct Tools { pending: Option<Prepared> }

impl Tools {
    pub fn cancel(&mut self) { self.pending = None; }

    pub fn load(&mut self, options: &Options, kind: ToolKind) -> Result<Reply, String> {
        self.cancel();
        let definition = open_profile_store(options)?.external_tool(kind).map_err(|e| e.to_string())?;
        Ok(Reply::ToolSettings { kind, definition, detail: "Application preference. Saving runs nothing. Review launch uses the saved definition and current target. Each argument is separate; {repo} and {file} expand only as whole arguments.".into() })
    }

    pub fn save(&mut self, options: &Options, repository: Option<&Path>, kind: ToolKind, mut definition: Option<ExternalTool>) -> Result<Reply, String> {
        self.cancel();
        if let Some(value) = definition.as_mut() {
            value.validate(kind)?;
            let executable = checked_executable(value, repository)?;
            value.executable = NativePath::new(executable.canonical.into_os_string());
        }
        open_profile_store(options)?.set_external_tool(kind, definition.as_ref()).map_err(|e| e.to_string())?;
        Ok(Reply::ToolSettings { kind, definition, detail: "Saved. No program was started. Review launch explicitly to inspect the destination and arguments.".into() })
    }

    pub fn prepare(&mut self, options: &Options, repository: &Path, kind: ToolKind, file: Option<NativePath>) -> Result<Reply, String> {
        self.cancel();
        let definition = open_profile_store(options)?.external_tool(kind).map_err(|e| e.to_string())?.ok_or("Configure and save this tool first")?;
        definition.validate(kind)?;
        let repository = std::fs::canonicalize(repository).map_err(|_| "Repository is unavailable")?;
        let git_directory = git_dir_of(&repository)?;
        let (_, grants) = trust_store(options)?;
        grants.verify(&git_directory, &repository).map_err(|e| e.to_string())?;
        let trust = srcmgr_state::fingerprint(&git_directory, &repository).map_err(|e| e.to_string())?;
        let executable = checked_executable(&definition, Some(&repository))?;
        let file = if kind == ToolKind::Editor {
            let raw = file.ok_or("Select a working-tree file before opening the editor; committed Diff content is not a working-tree target")?;
            let path = Path::new(raw.as_os_str());
            if path.components().next().is_none() || path.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) { return Err("Selected file must be relative to the active worktree".into()); }
            let observed = stamp(&repository.join(path))?;
            Some((raw, observed))
        } else { None };
        let working_directory = handoff_path(&repository);
        let arguments = definition.arguments.iter().map(|arg| match arg.as_str() {
            "{repo}" => working_directory.as_os_str().to_os_string(),
            "{file}" => handoff_path(&file.as_ref().unwrap().1.canonical).into_os_string(),
            _ => OsString::from(arg),
        }).collect::<Vec<_>>();
        let bytes = minicon_platform::entropy::secure_random_array::<16>().map_err(|_| "Cannot prepare a tool review")?;
        let ticket: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let label = |p: &Path| NativePath::new(p.as_os_str()).display_label();
        let mut lines = vec![format!("Open {}", kind.name()), format!("Repository identity: {}", label(&repository)), format!("Working directory: {}", label(&working_directory)), format!("Executable: {}", label(&executable.canonical))];
        if native_path_limit(&working_directory) { lines.push("Warning: the native long/UNC path is retained. The selected application may not support this path or may choose another directory; inspect its actual destination before editing or running commands.".into()); }
        if let Some((raw, observed)) = &file {
            lines.push(format!("Selected file: {}", raw.display_label()));
            lines.push(format!("Resolved file: {}", label(&observed.canonical)));
            if repository.join(raw.as_os_str()) != observed.canonical { lines.push("Warning: this path resolves through an alias or symbolic link; review the resolved destination.".into()); }
            if !observed.canonical.starts_with(&repository) { lines.push("Warning: the resolved file is outside this worktree.".into()); }
            if executable_content(&observed.canonical) { lines.push("Warning: selected content may be executable. It is passed to the chosen tool, whose behavior you authorize.".into()); }
        }
        for (index, argument) in arguments.iter().enumerate() { lines.push(format!("Argument {}: {}", index + 1, NativePath::new(argument.clone()).display_label())); }
        lines.extend(["No shell interpolation. The tool may read/change files or run its own plugins; it is not confined by SrcMgr.".into(), "Launch starts a user-owned tool; it does not establish editing, conflict resolution or a successful Git operation.".into(), "Review expires in 60 seconds. Enter does not approve; Cancel keeps the workspace.".into()]);
        self.pending = Some(Prepared { ticket: ticket.clone(), kind, definition, repository, working_directory, git_directory, trust, executable, file, arguments, expires: Instant::now() + Duration::from_secs(60) });
        Ok(Reply::ToolReview { ticket, lines })
    }

    pub fn launch(&mut self, options: &Options, current: &Path, ticket: &str) -> Result<Reply, String> {
        let p = self.pending.take().ok_or("Tool review is missing or consumed; review again")?;
        if p.ticket != ticket || Instant::now() >= p.expires || current != p.repository { return Err("Tool review expired or belongs to another repository; review again".into()); }
        // Hold the cooperative profile writer through launch, so a second
        // cooperating instance cannot replace the reviewed definition midway.
        let store = open_profile_store(options)?;
        if store.external_tool(p.kind).map_err(|e| e.to_string())?.as_ref() != Some(&p.definition) { return Err("Tool settings changed; review again".into()); }
        let (_, grants) = trust_store(options)?;
        grants.verify(&p.git_directory, &p.repository).map_err(|e| e.to_string())?;
        if git_dir_of(&p.repository)? != p.git_directory || srcmgr_state::fingerprint(&p.git_directory, &p.repository).map_err(|e| e.to_string())? != p.trust { return Err("Repository execution configuration changed; review again".into()); }
        if checked_executable(&p.definition, Some(&p.repository))? != p.executable { return Err("Tool executable changed; review again".into()); }
        if let Some((raw, expected)) = &p.file && stamp(&p.repository.join(raw.as_os_str()))? != *expected { return Err("Selected file changed or was redirected; review again".into()); }
        let mut command = Command::new(&p.executable.canonical);
        if std::fs::canonicalize(&p.working_directory).map_err(|_| "Working directory is unavailable")? != p.repository { return Err("Working directory changed; review again".into()); }
        command.args(&p.arguments).current_dir(&p.working_directory).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        for (key, _) in std::env::vars_os() {
            let label = key.to_string_lossy().to_ascii_uppercase();
            if label.starts_with("GIT_") || matches!(label.as_str(), "BASH_ENV" | "ENV" | "CDPATH") { command.env_remove(key); }
        }
        let (mut child, mode) = minicon_platform::process_spawn::spawn_breakaway_visible_child(&mut command).map_err(|e| format!("Tool was not started: {e}; correct the setting and review again"))?;
        let pid = child.id();
        let reaper = std::thread::Builder::new().name("srcmgr-tool-reaper".into()).spawn(move || { let _ = child.wait(); });
        let ownership = match mode { minicon_platform::process_spawn::DetachedSpawnMode::Independent => "independent user tool", minicon_platform::process_spawn::DetachedSpawnMode::CallerJobFallback => "host denied independence; tool remains in the caller job", _ => "host-specific lifetime; independence is unqualified" };
        let warning = if reaper.is_err() { "; startup succeeded but exit observation could not be scheduled" } else { "" };
        Ok(Reply::ToolLaunched(format!("{} started (process {pid}; {ownership}){warning}. Return here to continue; refresh repository state after external edits. Tool exit is not Git success.", p.kind.name())))
    }
}

// Keep repository identity canonical, but ordinary Windows desktop programs
// (including cmd hosted by Windows Terminal) do not uniformly accept verbatim
// paths as cwd/arguments. Offer a lossless DOS spelling only for ordinary short
// disk paths and after proving it still resolves to the same existing target.
fn handoff_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let units: Vec<_> = path.as_os_str().encode_wide().collect();
        let ordinary = path.components().all(|component| match component {
            Component::Normal(name) => {
                let units: Vec<_> = name.encode_wide().collect();
                if matches!(units.last(), Some(32 | 46)) || units.iter().any(|u| *u < 32 || [34,60,62,58,124,63,42].contains(u)) { return false; }
                let stem: Vec<_> = units.into_iter().take_while(|u| *u != 46).collect();
                let stem = String::from_utf16(&stem).unwrap_or_default().to_ascii_lowercase();
                !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul" | "conin$" | "conout$" | "com¹" | "com²" | "com³" | "lpt¹" | "lpt²" | "lpt³") && !(stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            }
            _ => true,
        });
        if ordinary && units.len() > 7 && units.len() - 4 < 260 && units[..4] == [92,92,63,92] && units[5] == 58 && units[6] == 92 {
            let candidate = PathBuf::from(OsString::from_wide(&units[4..]));
            if std::fs::canonicalize(&candidate).is_ok_and(|canonical| canonical == path) { return candidate; }
        }
    }
    path.to_owned()
}

fn native_path_limit(path: &Path) -> bool {
    #[cfg(windows)]
    { use std::path::Prefix; matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) | Prefix::UNC(_, _))) }
    #[cfg(not(windows))]
    { let _ = path; false }
}

fn checked_executable(definition: &ExternalTool, repository: Option<&Path>) -> Result<FileStamp, String> {
    let path = Path::new(definition.executable.as_os_str());
    if !path.is_absolute() { return Err("Choose an absolute executable path; PATH lookup and command strings are not used".into()); }
    let value = stamp(path)?;
    if repository.is_some_and(|root| value.canonical.starts_with(root)) { return Err("Choose a tool outside the active repository".into()); }
    let extension = value.canonical.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(extension.as_str(), "cmd" | "bat" | "ps1") { return Err("Choose the native tool executable, not a shell script or command template".into()); }
    let name = value.canonical.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(name.as_str(), "cmd" | "powershell" | "pwsh" | "sh" | "bash" | "zsh" | "fish" | "dash") { return Err("Choose a terminal application executable, such as MiniCon or Windows Terminal, instead of a shell interpreter. This handoff supplies no interactive console or shell command template.".into()); }
    Ok(value)
}

fn executable_content(path: &Path) -> bool {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    let known = matches!(extension.as_str(), "exe" | "com" | "cmd" | "bat" | "ps1" | "sh" | "py");
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; known || std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) }
    #[cfg(not(unix))]
    { known }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_backend::tests::{Scratch, repository, git_executable};

    fn fixture_root(root: &Scratch) -> PathBuf {
        let canonical = std::fs::canonicalize(&root.0).unwrap();
        // The controlled fixture uses ordinary short names. Resolve any 8.3
        // temp-parent alias first, then express its expected ordinary spelling;
        // do not call the production handoff helper to generate the oracle.
        #[cfg(windows)]
        { PathBuf::from(canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap()) }
        #[cfg(not(windows))]
        { canonical }
    }

    #[cfg(windows)]
    #[test]
    fn tools_windows_handoff_shortens_only_unambiguous_existing_disk_paths() {
        let root = Scratch::new("tool-windows-paths");
        let canonical = std::fs::canonicalize(&root.0).unwrap();
        let ordinary = canonical.join("ordinary 中文");
        std::fs::create_dir(&ordinary).unwrap();
        assert_eq!(handoff_path(&ordinary), fixture_root(&root).join("ordinary 中文"));
        for name in ["trailing.", "trailing ", "COM¹", "NUL.txt"] {
            let path = canonical.join(name);
            std::fs::write(&path, b"native name").unwrap();
            assert_eq!(handoff_path(&path), path, "ambiguous Win32 spellings must retain exact native identity");
            assert_eq!(std::fs::read(&path).unwrap(), b"native name");
        }
    }

    fn wait_report(path: &Path) {
        let until = Instant::now() + Duration::from_secs(5);
        while !path.exists() && Instant::now() < until { std::thread::sleep(Duration::from_millis(10)); }
        assert!(path.exists(), "owned tool did not publish its atomic execution report");
    }

    fn refused(result: Result<Reply, String>, report: &Path) {
        if matches!(result, Ok(Reply::ToolLaunched(_))) { wait_report(report); }
        assert!(!report.exists(), "a refused/stale launch actually executed the tool");
        assert!(result.is_err(), "expected refusal");
    }

    #[test]
    fn tools_actual_launch_binds_saved_definition_target_trust_and_exact_arguments() {
        assert_eq!(std::env::var("GIT_CONFIG_COUNT").as_deref(), Ok("0"), "run this scoped case with harmless GIT_CONFIG_COUNT=0 to discriminate environment sanitization");
        let root = Scratch::new("tool-handoff");
        let left = std::fs::canonicalize(repository(&root.0, "left & 中文")).unwrap();
        let right = std::fs::canonicalize(repository(&root.0, "right")).unwrap();
        let raw = NativePath::new("working & ; $() 中文.txt");
        std::fs::write(left.join(raw.as_os_str()), b"owned file\n").unwrap();
        let options = Options { profile: Some(root.0.join("profile")), git: Some(git_executable()), ..Default::default() };
        drop(open_profile_store(&options).unwrap());
        let (_, mut grants) = trust_store(&options).unwrap();
        for repo in [&left, &right] { let dir = git_dir_of(repo).unwrap(); grants.grant(dir.clone(), srcmgr_state::fingerprint(&dir, repo).unwrap()); }
        grants.save().unwrap();
        let report = root.0.join("executed.txt");
        let executable = root.0.join(if cfg!(windows) { "owned-tool.exe" } else { "owned-tool" });
        let source = root.0.join("owned_tool.rs");
        std::fs::write(&source, r#"fn main() {
            let args: Vec<_> = std::env::args_os().skip(1).collect();
            let report = std::path::Path::new(&args[0]);
            let text = format!("{:?}\n{:?}\n{:?}\n", std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap(), &args[1..], std::env::var_os("GIT_CONFIG_COUNT"));
            let pending = report.with_extension("pending");
            std::fs::write(&pending, text).unwrap(); std::fs::rename(pending, report).unwrap();
        }"#).unwrap();
        let built = Command::new("rustc").arg(&source).arg("-o").arg(&executable).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        let executable = std::fs::canonicalize(executable).unwrap();
        assert!(Command::new(&executable).arg(&report).current_dir(&left).status().unwrap().success());
        assert!(std::fs::read_to_string(&report).unwrap().ends_with("Some(\"0\")\n"), "positive control sees inherited environment and execution");
        std::fs::remove_file(&report).unwrap();
        let definition = ExternalTool { executable: NativePath::new(executable.as_os_str()), arguments: vec![report.to_str().unwrap().into(), "{file}".into(), "{repo}".into(), " literal ; & $() \"quoted\" ".into()] };
        let mut tools = Tools::default();
        tools.save(&options, Some(&left), ToolKind::Editor, Some(definition.clone())).unwrap();
        tools = Tools::default();
        assert!(!report.exists(), "save/restart is passive");
        assert!(tools.save(&options, Some(&left), ToolKind::Editor, Some(ExternalTool { executable: NativePath::new("relative-tool"), ..definition.clone() })).is_err());
        let local_exe = left.join(if cfg!(windows) { "local-tool.exe" } else { "local-tool" });
        std::fs::copy(&executable, &local_exe).unwrap();
        assert!(tools.save(&options, Some(&left), ToolKind::Editor, Some(ExternalTool { executable: NativePath::new(local_exe.as_os_str()), ..definition.clone() })).is_err());
        assert!(!report.exists(), "refusing a repository-local executable must not probe it");
        std::fs::remove_file(local_exe).unwrap();
        {
            let held = open_profile_store(&options).unwrap();
            assert!(tools.save(&options, Some(&left), ToolKind::Editor, None).is_err());
            assert_eq!(held.external_tool(ToolKind::Editor).unwrap(), Some(definition.clone()));
        }
        assert!(tools.prepare(&options, &left, ToolKind::Editor, Some(NativePath::new("../outside"))).is_err());
        assert!(tools.prepare(&options, &left, ToolKind::Editor, Some(NativePath::new("missing-file"))).is_err());
        let review = |tools: &mut Tools| {
            let Reply::ToolReview { ticket, .. } = tools.prepare(&options, &left, ToolKind::Editor, Some(raw.clone())).unwrap() else { panic!() }; ticket
        };
        let cancelled = review(&mut tools); tools.cancel(); refused(tools.launch(&options, &left, &cancelled), &report);
        let stale = review(&mut tools);
        let mut changed = definition.clone(); changed.arguments.push("changed".into());
        open_profile_store(&options).unwrap().set_external_tool(ToolKind::Editor, Some(&changed)).unwrap();
        refused(tools.launch(&options, &left, &stale), &report);
        tools.save(&options, Some(&left), ToolKind::Editor, Some(definition.clone())).unwrap();
        let expired = review(&mut tools); tools.pending.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
        refused(tools.launch(&options, &left, &expired), &report);
        let wrong = review(&mut tools); refused(tools.launch(&options, &right, &wrong), &report);
        let file_changed = review(&mut tools); std::fs::write(left.join(raw.as_os_str()), b"new source stays\n").unwrap();
        refused(tools.launch(&options, &left, &file_changed), &report);
        let exe_changed = review(&mut tools);
        let original_exe = std::fs::read(&executable).unwrap();
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&executable).unwrap().write_all(b"overlay").unwrap();
        refused(tools.launch(&options, &left, &exe_changed), &report);
        std::fs::write(&executable, original_exe).unwrap();
        let untrusted = review(&mut tools);
        let (_, mut grants) = trust_store(&options).unwrap(); grants.revoke(&git_dir_of(&left).unwrap()); grants.save().unwrap();
        refused(tools.launch(&options, &left, &untrusted), &report);
        let dir = git_dir_of(&left).unwrap(); grants.grant(dir.clone(), srcmgr_state::fingerprint(&dir, &left).unwrap()); grants.save().unwrap();
        let state = |repo: &Path| {
            [vec!["rev-parse", "HEAD"], vec!["write-tree"]].into_iter().map(|args| {
                let out = Command::new(git_executable()).current_dir(repo).args(args).output().unwrap(); assert!(out.status.success()); out.stdout
            }).collect::<Vec<_>>()
        };
        let before = [state(&left), state(&right)];
        open_profile_store(&options).unwrap().put_draft("git", "tool-draft", "keep").unwrap();
        let ticket = review(&mut tools);
        assert!(matches!(tools.launch(&options, &left, &ticket), Ok(Reply::ToolLaunched(_))));
        wait_report(&report);
        let argument_repo = fixture_root(&root).join("left & 中文");
        let expected_args = [argument_repo.join(raw.as_os_str()).into_os_string(), argument_repo.into_os_string(), OsString::from(" literal ; & $() \"quoted\" ")];
        assert_eq!(std::fs::read_to_string(&report).unwrap(), format!("{left:?}\n{expected_args:?}\nNone\n"));
        std::fs::remove_file(&report).unwrap();
        refused(tools.launch(&options, &left, &ticket), &report);
        assert_eq!([state(&left), state(&right)], before);
        assert_eq!(std::fs::read(left.join(raw.as_os_str())).unwrap(), b"new source stays\n");
        assert_eq!(open_profile_store(&options).unwrap().get_draft("git", "tool-draft").unwrap().as_deref(), Some("keep"));
        let terminal = ExternalTool { executable: definition.executable.clone(), arguments: vec![report.to_str().unwrap().into(), "{repo}".into()] };
        tools.save(&options, Some(&right), ToolKind::Terminal, Some(terminal)).unwrap();
        let Reply::ToolReview { ticket, .. } = tools.prepare(&options, &right, ToolKind::Terminal, None).unwrap() else { panic!() };
        tools.launch(&options, &right, &ticket).unwrap(); wait_report(&report);
        assert_eq!(std::fs::read_to_string(&report).unwrap(), format!("{right:?}\n{:?}\nNone\n", [fixture_root(&root).join("right").into_os_string()]));
    }
}
