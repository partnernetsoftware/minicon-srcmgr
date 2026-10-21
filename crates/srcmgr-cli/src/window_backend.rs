//! The status window's Git side. Every job re-runs the trust gate and reads
//! fresh state, so a grant revoked or a config changed while the window is
//! open stops the next action. Results are plain data for the window.

use std::path::PathBuf;

use srcmgr_domain::{ChangeKind, Generation, NativePath};
use srcmgr_ui::{Backend, BranchRow, CommitRow, FileTarget, Job, MainRequest, MainView, PromptAction, Reply, Snapshot};

use crate::{
    BACKEND_GIT, DEFAULT_HISTORY_LIMIT, HISTORY_LIMIT_KEY, Options, open_profile_store, open_trusted,
    parse_history_limit, repository_key,
};

pub struct WindowBackend {
    pub options: Options,
    pub worktree: PathBuf,
    pub prepared: Option<crate::confirmation::Prepared>,
}

impl Backend for WindowBackend {
    fn run(&mut self, job: Job) -> Reply {
        let result = match job {
            Job::CancelPrepared => { self.prepared = None; Ok(Reply::NoChange) }
            Job::Main { generation, request } => {
                let context = match &request {
                    MainRequest::CommitFiles { id } => Some(format!("Commit {id}\nBase: unavailable")),
                    MainRequest::CommitFile { id, file } => Some(format!("Commit {id}\nBase: unavailable\n{}", file.path.display_label())),
                    _ => None,
                };
                Ok(match self.main(request) {
                Ok(view) => Reply::Main { generation, view },
                Err(error) => Reply::MainFailed { generation, text: context.map_or_else(|| error.clone(), |context| format!("{context}\nDetail unavailable: {error}")) },
                })
            }
            Job::Prepare { generation, action } => {
                self.prepared = None;
                crate::confirmation::Prepared::create(&self.options, &self.worktree, generation, *action)
                    .map(|(prepared, reply)| { self.prepared = Some(prepared); reply })
                    .or_else(|text| Ok(Reply::ConfirmationFailed { generation, text }))
            }
            Job::ExecutePrepared(ticket) => {
                let authorized = self.prepared.take().ok_or_else(|| "confirmation missing or consumed; preview again".to_owned())
                    .and_then(|prepared| prepared.authorize(&self.options, &self.worktree, &ticket));
                match authorized {
                    Err(error) => self.snapshot().map(|mut snapshot| {
                        snapshot.stage_result = Some(format!("Action refused before execution; state refreshed: {error}"));
                        Reply::Full(snapshot)
                    }).map_err(|_| format!("Action refused before execution: {error}; state refresh unavailable")),
                    Ok((action, note)) => (|| {
                        let mut reply = match self.perform(action, Some(&ticket)) {
                            Ok(reply) => reply,
                            Err(error) => {
                                let mut snapshot = self.snapshot().map_err(|refresh| format!("Action may have effects: {error}; refresh failed: {refresh}; {note}"))?;
                                snapshot.stage_result = Some(format!("Action failed; state refreshed: {error}"));
                                Reply::Full(snapshot)
                            }
                        };
                        if let Reply::Full(snapshot) = &mut reply && !note.is_empty() {
                                snapshot.stage_result = Some(snapshot.stage_result.as_ref().map_or_else(|| note.clone(), |result| format!("{result}; {note}")));
                        }
                        Ok(reply)
                    })(),
                }
            }
            action if crate::confirmation::is_risky(&action) => Err("a fresh target-bound confirmation is required".to_owned()),
            action @ (Job::Commit(_) | Job::CreateBranch(_) | Job::SaveDraft(_)) => {
                let kind = match &action {
                    Job::Commit(_) => PromptAction::Commit,
                    Job::CreateBranch(_) => PromptAction::CreateBranch,
                    _ => PromptAction::SaveDraft,
                };
                Ok(self.perform(action, None).unwrap_or_else(|text| Reply::PromptFailed { action: kind, text }))
            }
            action => self.perform(action, None),
        };
        result.unwrap_or_else(Reply::Notice)
    }
}

impl WindowBackend {
    /// The full list state. Also the trust check the window runs before it opens.
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let session = open_trusted(&self.options, &self.worktree)?;
        let (installation, identity) = (&session.installation, &session.identity);
        let status = srcmgr_git::read_status(installation, identity, Generation(1)).map_err(|e| e.to_string())?;
        let branches = srcmgr_git::local_branches(installation, identity)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|branch| BranchRow { name: branch.name, label: branch.label, current: branch.current })
            .collect();
        let store = open_profile_store(&self.options)?;
        let limit = match store.get_setting(HISTORY_LIMIT_KEY).map_err(|e| e.to_string())? {
            Some(saved) => parse_history_limit(&saved)?,
            None => DEFAULT_HISTORY_LIMIT,
        };
        let commits = srcmgr_git::log(installation, identity, 0, limit)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|commit| CommitRow {
                id: commit.id.token().to_owned(),
                subject: commit.subject,
                merge: commit.parents.len() > 1,
            })
            .collect();
        let key = repository_key(&self.worktree)?;
        let draft = store.get_draft(BACKEND_GIT, &key).map_err(|e| e.to_string())?;
        let name = self
            .worktree
            .file_name()
            .map_or_else(|| "repository".to_owned(), |name| name.to_string_lossy().into_owned());
        let mut snapshot = Snapshot::new(format!("MiniCon SrcMgr - {name}"), &status, branches, commits, draft);
        snapshot.stashes = srcmgr_git::stash_list(installation, identity).map_err(|e| e.to_string())?.into_iter()
            .map(|s| srcmgr_ui::StashRow { id: s.id, subject: s.subject }).collect();
        Ok(snapshot)
    }

    fn perform(&mut self, job: Job, ticket: Option<&str>) -> Result<Reply, String> {
        Ok(match job {
            Job::StashAction { id, expected, operation } => {
                let session = open_trusted(&self.options, &self.worktree)?;
                let ticket = ticket.ok_or_else(|| "stash action requires a consumed confirmation".to_owned())?;
                let transaction = crate::profile_root(&self.options)?.join("recovery").join(ticket);
                let note = srcmgr_git::stash_selected(&session.installation, &session.identity, &id, &expected, &operation, &transaction).map_err(|e| e.to_string())?;
                let mut snapshot = self.snapshot()?;
                snapshot.stage_result = Some(note);
                Reply::Full(snapshot)
            }
            Job::Prepare { .. } | Job::ExecutePrepared(_) | Job::CancelPrepared => return Err("nested confirmation is not allowed".to_owned()),
            Job::Reload => Reply::Full(self.snapshot()?),
            Job::Main { generation, request } => Reply::Main { generation, view: self.main(request)? },
            Job::SetFile { path, stage } => {
                self.set_file(&path, stage)?;
                self.stage_result(Some(&path))?
            }
            Job::SetHunk { path, staged_view, number } => {
                let index = number.checked_sub(1).ok_or("hunk numbers start at 1")?;
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::apply_hunk(&session.installation, &session.identity, path.as_os_str(), index, staged_view)
                    .map_err(|e| e.to_string())?;
                self.stage_result(Some(&path))?
            }
            Job::SetLines { path, staged_view, number, lines } => {
                let index = number.checked_sub(1).ok_or("hunk numbers start at 1")?;
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::apply_lines(
                    &session.installation,
                    &session.identity,
                    path.as_os_str(),
                    index,
                    &lines,
                    staged_view,
                )
                .map_err(|e| e.to_string())?;
                self.stage_result(Some(&path))?
            }
            Job::SetAll { stage } => {
                let session = open_trusted(&self.options, &self.worktree)?;
                let result = if stage {
                    srcmgr_git::stage_all(&session.installation, &session.identity)
                } else {
                    srcmgr_git::unstage_all(&session.installation, &session.identity)
                };
                result.map_err(|e| e.to_string())?;
                self.stage_result(None)?
            }
            Job::DeleteBranch(name) => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::delete_branch(&session.installation, &session.identity, name.as_ref())
                    .map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::Discard { path, untracked } => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::discard_file(&session.installation, &session.identity, path.as_os_str(), untracked)
                    .map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::DiscardHunk { path, index } => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::discard_hunk(&session.installation, &session.identity, path.as_os_str(), index)
                    .map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::Stash => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::stash_push(&session.installation, &session.identity).map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::Detach(id) => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::checkout_detached(&session.installation, &session.identity, &id)
                    .map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::Switch(name) => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::switch_branch(&session.installation, &session.identity, name.as_ref())
                    .map_err(|e| e.to_string())?;
                Reply::Full(self.snapshot()?)
            }
            Job::CreateBranch(name) => {
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::create_branch(&session.installation, &session.identity, name.as_ref())
                    .map_err(|e| e.to_string())?;
                match self.snapshot() {
                    Ok(snapshot) => Reply::BranchCreated(snapshot),
                    Err(error) => Reply::PromptCompleted {
                        action: PromptAction::CreateBranch,
                        text: format!("Branch created.\nName: {}\nState refresh unavailable: {error}\nDisplayed lists may be stale.\nRefresh after storage is available.\nDo not resubmit creation.", NativePath::new(&name).display_label()),
                    },
                }
            }
            Job::Commit(message) => {
                let (id, warning) = self.commit(&message)?;
                match self.snapshot() {
                    Ok(mut snapshot) => {
                        snapshot.stage_result = warning.map(|error| format!("Commit created; saved draft cleanup unavailable: {error}; the saved draft may remain"));
                        Reply::Committed(snapshot)
                    }
                    Err(error) => Reply::PromptCompleted {
                        action: PromptAction::Commit,
                        text: format!("Commit created.\nOID: {id}\n{}State refresh unavailable: {error}\nDisplayed lists may be stale.\nRefresh after storage is available.\nDo not resubmit this commit.", warning.map_or_else(String::new, |error| format!("Saved draft may remain; cleanup unavailable.\nCleanup reason: {error}\n"))),
                    },
                }
            }
            Job::SaveDraft(text) => {
                self.save_draft(&text)?;
                Reply::DraftSaved(if text.trim().is_empty() { None } else { Some(text) })
            }
        })
    }

    fn stage_result(&self, path: Option<&NativePath>) -> Result<Reply, String> {
        let mut snapshot = self.snapshot()?;
        let session = open_trusted(&self.options, &self.worktree)?;
        let summary = srcmgr_git::index_summary(&session.installation, &session.identity, path.map(NativePath::as_os_str))
            .map_err(|e| format!("index action finished; result refresh failed: {e}"))?;
        let scope = path.map_or_else(|| "whole index".to_owned(), NativePath::display_label);
        snapshot.stage_result = Some(summary.label(&scope));
        Ok(Reply::Full(snapshot))
    }

    fn main(&self, request: MainRequest) -> Result<MainView, String> {
        let session = open_trusted(&self.options, &self.worktree)?;
        match request {
            MainRequest::CommitFiles { id } => {
                let detail = srcmgr_git::commit_files(&session.installation, &session.identity, &id).map_err(|e| e.to_string())?;
                let base = detail.parent.as_ref().map_or_else(|| "empty tree (root commit)".to_owned(), |p| format!("{} (first parent{})", p, if detail.merge { "; merge has other parents" } else { "" }));
                let files: Vec<_> = detail.files.into_iter().map(|f| srcmgr_ui::CommittedFile { status: f.status, old_mode: f.old_mode, new_mode: f.new_mode, path: f.path, old_path: f.old_path }).collect();
                let mut text = format!("Commit {id}\nBase: {base}\n");
                for file in &files {
                    let path = file.old_path.as_ref().map_or_else(|| file.path.display_label(), |old| format!("{} -> {}", old.display_label(), file.path.display_label()));
                    text.push_str(&format!("{} {path} [{} -> {}]\n", file.status, file.old_mode, file.new_mode));
                }
                if detail.truncated { text.push_str("(file list truncated at 2000 entries)\n"); }
                let mut view = MainView::text(&text);
                view.committed = Some(srcmgr_ui::CommittedFiles { id, base, files });
                Ok(view)
            }
            MainRequest::CommitFile { id, file } => {
                let git_file = srcmgr_git::CommitFile { status: file.status, old_mode: file.old_mode, new_mode: file.new_mode, path: file.path.clone(), old_path: file.old_path };
                let detail = srcmgr_git::commit_files(&session.installation, &session.identity, &id).map_err(|e| e.to_string())?;
                if !detail.files.contains(&git_file) { return Err("committed file does not belong to the selected comparison".to_owned()); }
                let base = detail.parent.map_or_else(|| "empty tree (root commit)".to_owned(), |parent| format!("{parent} (first parent{})", if detail.merge { "; merge has other parents" } else { "" }));
                let text = srcmgr_git::commit_file_diff(&session.installation, &session.identity, &id, &git_file).map_err(|e| e.to_string())?;
                Ok(MainView::text(&format!("Commit {id}\nBase: {base}\n{} [{} -> {}]\n{text}", file.path.display_label(), git_file.old_mode, git_file.new_mode)))
            }
            MainRequest::Stash { id } => {
                let text = srcmgr_git::stash_diff(&session.installation, &session.identity, &id).map_err(|e| e.to_string())?;
                Ok(MainView::text(&text))
            }
            MainRequest::File { path, staged_view, untracked } => {
                if untracked {
                    return Ok(MainView::note("(untracked file: no diff against the index)"));
                }
                let text = srcmgr_git::diff(&session.installation, &session.identity, path.as_os_str(), staged_view)
                    .map_err(|e| e.to_string())?;
                Ok(MainView::diff(&text, FileTarget { path, staged_view }))
            }
            MainRequest::Commit { id } => {
                let text = srcmgr_git::commit_detail(&session.installation, &session.identity, &id)
                    .map_err(|e| e.to_string())?;
                Ok(MainView::text(&text))
            }
        }
    }

    /// Stages or unstages one changed path. The path must still be in the
    /// current status, as the CLI requires.
    fn set_file(&self, path: &NativePath, stage: bool) -> Result<(), String> {
        let session = open_trusted(&self.options, &self.worktree)?;
        let status = srcmgr_git::read_status(&session.installation, &session.identity, Generation(1))
            .map_err(|e| e.to_string())?;
        let entry = status
            .entries
            .iter()
            .find(|entry| entry.path.as_os_str() == path.as_os_str())
            .ok_or_else(|| format!("not a changed path: {}", path.display_label()))?;
        if !stage && entry.staged == ChangeKind::Unmodified {
            return Err(format!("not staged: {}", path.display_label()));
        }
        let paths = [path.as_os_str().to_os_string()];
        let result = if stage {
            srcmgr_git::stage(&session.installation, &session.identity, &paths)
        } else {
            srcmgr_git::unstage(&session.installation, &session.identity, &paths)
        };
        result.map_err(|e| e.to_string())
    }

    /// Commits the index with `message`. The draft is cleared only after Git
    /// accepts the commit; a refused commit keeps both the index and the draft.
    /// A subsequent cleanup failure is a warning attached to the created OID.
    fn commit(&self, message: &str) -> Result<(String, Option<String>), String> {
        if message.trim().is_empty() {
            return Err("the commit message is empty".to_owned());
        }
        let key = repository_key(&self.worktree)?;
        let session = open_trusted(&self.options, &self.worktree)?;
        let status = srcmgr_git::read_status(&session.installation, &session.identity, Generation(1))
            .map_err(|e| e.to_string())?;
        if !status.entries.iter().any(|entry| entry.staged != ChangeKind::Unmodified) {
            return Err("nothing is staged to commit".to_owned());
        }
        let id = srcmgr_git::commit(&session.installation, &session.identity, message).map_err(|e| e.to_string())?;
        let cleanup = open_profile_store(&self.options)
            .and_then(|mut store| store.delete_draft(BACKEND_GIT, &key).map_err(|e| e.to_string()));
        Ok((id.token().to_owned(), cleanup.err()))
    }

    /// Replaces the saved draft. Blank text clears it.
    fn save_draft(&self, text: &str) -> Result<(), String> {
        let key = repository_key(&self.worktree)?;
        let mut store = open_profile_store(&self.options)?;
        if text.trim().is_empty() {
            store.delete_draft(BACKEND_GIT, &key)
        } else {
            store.put_draft(BACKEND_GIT, &key, text)
        }
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use srcmgr_domain::NativePath;
    use srcmgr_ui::{Backend, Job, MainRequest, Reply, Snapshot};

    use super::WindowBackend;
    use crate::{Options, trust};

    /// A scratch root per test, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = env::temp_dir().join(format!("srcmgr-window-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("create scratch");
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn git_executable() -> PathBuf {
        env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
    }

    fn fixture_git(directory: &Path, arguments: &[&str]) {
        let output = Command::new(git_executable())
            .current_dir(directory)
            .args(arguments)
            .output()
            .expect("run fixture git");
        assert!(output.status.success(), "fixture git {arguments:?}");
    }

    /// One commit with `a.txt` modified and `u.txt` untracked.
    fn repository(root: &Path, name: &str) -> PathBuf {
        let repo = root.join(name);
        fs::create_dir_all(&repo).expect("create repo");
        fixture_git(&repo, &["init", "-q", "-b", "main"]);
        fixture_git(&repo, &["config", "user.name", "Fixture"]);
        fixture_git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        fs::write(repo.join("a.txt"), "one\n").expect("write");
        fixture_git(&repo, &["add", "a.txt"]);
        fixture_git(&repo, &["commit", "-q", "-m", "initial"]);
        fs::write(repo.join("a.txt"), "two\n").expect("modify");
        fs::write(repo.join("u.txt"), "new\n").expect("untracked");
        repo
    }

    fn backend(root: &Path, repo: &Path, grant: bool) -> WindowBackend {
        let options = Options {
            profile: Some(root.join("profile")),
            git: Some(git_executable()),
            ..Options::default()
        };
        if grant {
            let granted = Options {
                positional: vec![
                    OsString::from("trust"),
                    OsString::from("grant"),
                    repo.as_os_str().to_owned(),
                ],
                ..options.clone()
            };
            trust(&granted).expect("grant the fixture");
        }
        WindowBackend {
            options,
            worktree: repo.to_path_buf(),
            prepared: None,
        }
    }

    fn lock_profile_after_git_hook(root: &Path, repo: &Path, name: &str) -> PathBuf {
        let lock = root.join("profile/profile.lock");
        let path = lock.to_str().unwrap().replace('\\', "/").replace('\'', "'\\''");
        let hook = repo.join(".git/hooks").join(name);
        fs::write(&hook, format!("#!/bin/sh\nprintf 'owned post-write lock' > '{path}'\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        lock
    }

    fn confirmed(backend: &mut WindowBackend, action: Job) -> Reply {
        match backend.run(Job::Prepare { generation: 7, action: Box::new(action) }) {
            Reply::Prepared { ticket, .. } => backend.run(Job::ExecutePrepared(ticket)),
            other => other,
        }
    }

    fn git_text(repo: &Path, args: &[&str]) -> String {
        let output = Command::new(git_executable()).current_dir(repo).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn main_view(reply: Reply) -> srcmgr_ui::MainView {
        match reply { Reply::Main { view, .. } => view, other => panic!("expected detail: {other:?}") }
    }

    #[test]
    fn committed_files_use_actual_root_rename_deletion_modes_and_merge_parent() {
        let scratch = Scratch::new("commit-files");
        let repo = repository(&scratch.0, "repo");
        let root = git_text(&repo, &["rev-parse", "HEAD"]);
        fixture_git(&repo, &["restore", "a.txt"]);
        fs::write(repo.join("gone.txt"), "gone\n").unwrap();
        fixture_git(&repo, &["add", "gone.txt"]);
        fixture_git(&repo, &["commit", "-qm", "prepare deletion"]);
        fixture_git(&repo, &["mv", "a.txt", "renamed.txt"]);
        fixture_git(&repo, &["rm", "gone.txt"]);
        fixture_git(&repo, &["commit", "-qm", "rename and delete"]);
        let renamed = git_text(&repo, &["rev-parse", "HEAD"]);
        fixture_git(&repo, &["switch", "-qc", "side"]);
        fs::write(repo.join("side.txt"), "side\n").unwrap();
        fixture_git(&repo, &["add", "side.txt"]);
        fixture_git(&repo, &["commit", "-qm", "side"]);
        fixture_git(&repo, &["switch", "-q", "main"]);
        fixture_git(&repo, &["merge", "--no-ff", "-qm", "merge", "side"]);
        let merge = git_text(&repo, &["rev-parse", "HEAD"]);
        fs::write(repo.join("binary.dat"), [0, 1, 2, 255]).unwrap();
        fixture_git(&repo, &["add", "binary.dat"]);
        fixture_git(&repo, &["update-index", "--add", "--cacheinfo", &format!("160000,{root},module")]);
        fixture_git(&repo, &["commit", "-qm", "binary and gitlink"]);
        let metadata_commit = git_text(&repo, &["rev-parse", "HEAD"]);
        let original_status = git_text(&repo, &["status", "--porcelain"]);
        let mut backend = backend(&scratch.0, &repo, true);
        let root_view = main_view(backend.run(Job::Main { generation: 1, request: MainRequest::CommitFiles { id: root.clone() } }));
        let root_files = root_view.committed.unwrap();
        assert!(root_files.base.contains("empty tree"));
        assert_eq!(root_files.files.len(), 1);
        assert_eq!((&*root_files.files[0].old_mode, &*root_files.files[0].new_mode), ("000000", "100644"));
        let root_diff = main_view(backend.run(Job::Main { generation: 2, request: MainRequest::CommitFile { id: root, file: root_files.files[0].clone() } }));
        assert!(root_diff.lines.iter().any(|line| line.text == "+one"));
        assert!(root_diff.file.is_none(), "historical content never authorizes worktree mutation");
        let view = main_view(backend.run(Job::Main { generation: 3, request: MainRequest::CommitFiles { id: renamed.clone() } }));
        let files = view.committed.unwrap().files;
        let rename = files.iter().find(|f| f.status == "R100").unwrap();
        assert_eq!(rename.old_path, Some(NativePath::new("a.txt")));
        assert_eq!(rename.path, NativePath::new("renamed.txt"));
        let deleted = files.iter().find(|f| f.status == "D").unwrap();
        assert_eq!(deleted.path, NativePath::new("gone.txt"));
        assert_eq!(deleted.new_mode, "000000");
        let deletion = main_view(backend.run(Job::Main { generation: 4, request: MainRequest::CommitFile { id: renamed.clone(), file: deleted.clone() } }));
        assert!(deletion.lines.iter().any(|line| line.text == "-gone"));
        let merge_view = main_view(backend.run(Job::Main { generation: 5, request: MainRequest::CommitFiles { id: merge } }));
        let merged = merge_view.committed.unwrap();
        assert!(merged.base.contains(&renamed) && merged.base.contains("merge has other parents"));
        assert_eq!(merged.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), vec![NativePath::new("side.txt")]);
        let metadata_view = main_view(backend.run(Job::Main { generation: 6, request: MainRequest::CommitFiles { id: metadata_commit.clone() } }));
        let metadata = metadata_view.committed.unwrap();
        let gitlink = metadata.files.iter().find(|f| f.path == NativePath::new("module")).unwrap();
        assert_eq!(gitlink.new_mode, "160000");
        let binary = metadata.files.iter().find(|f| f.path == NativePath::new("binary.dat")).unwrap();
        let binary_diff = main_view(backend.run(Job::Main { generation: 7, request: MainRequest::CommitFile { id: metadata_commit, file: binary.clone() } }));
        assert!(binary_diff.lines.iter().any(|line| line.text.contains("Binary files")));
        assert_eq!(git_text(&repo, &["status", "--porcelain"]), original_status, "historical reads preserve actual worktree/index state");
    }

    #[test]
    fn selected_stash_apply_conflict_and_stale_list_use_real_git_state() {
        let scratch = Scratch::new("selected-stash");
        let repo = repository(&scratch.0, "repo");
        fixture_git(&repo, &["stash", "push", "-qm", "older"]);
        fs::write(repo.join("a.txt"), "three\n").unwrap();
        fixture_git(&repo, &["stash", "push", "-qm", "newer"]);
        let mut backend = backend(&scratch.0, &repo, true);
        let list = full(backend.run(Job::Reload)).stashes;
        assert_eq!(list.len(), 2);
        let ids: Vec<_> = list.iter().map(|s| s.id.clone()).collect();
        let applied = full(confirmed(&mut backend, Job::StashAction { id: ids[1].clone(), expected: ids.clone(), operation: "apply".into() }));
        assert!(applied.stage_result.unwrap().contains("source retained"));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");
        assert_eq!(applied.stashes.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), ids);
        fixture_git(&repo, &["restore", "a.txt"]);
        let prepared = backend.run(Job::Prepare { generation: 6, action: Box::new(Job::StashAction { id: ids[1].clone(), expected: ids.clone(), operation: "drop".into() }) });
        let Reply::Prepared { ticket, .. } = prepared else { panic!("{prepared:?}") };
        fs::write(repo.join("a.txt"), "external\n").unwrap();
        fixture_git(&repo, &["stash", "push", "-qm", "external"]);
        assert!(notice(backend.run(Job::ExecutePrepared(ticket))).contains("changed"));
        assert_eq!(git_text(&repo, &["stash", "list", "--format=%H"]).lines().count(), 3);
        fs::write(repo.join("a.txt"), "conflicting base\n").unwrap();
        fixture_git(&repo, &["add", "a.txt"]);
        fixture_git(&repo, &["commit", "-qm", "different base"]);
        let current = full(backend.run(Job::Reload));
        let expected: Vec<_> = current.stashes.iter().map(|s| s.id.clone()).collect();
        let result = full(confirmed(&mut backend, Job::StashAction { id: ids[1].clone(), expected: expected.clone(), operation: "pop".into() }));
        assert!(result.stage_result.unwrap().contains("failed"));
        assert_eq!(result.stashes.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), expected);
        assert!(git_text(&repo, &["ls-files", "-u"]).contains("a.txt"));
        assert!(fs::read_to_string(repo.join("a.txt")).unwrap().contains("<<<<<<<"));
        assert_eq!(fs::read_to_string(repo.join("u.txt")).unwrap(), "new\n");
    }

    #[test]
    fn selected_stash_pop_drop_last_entry_and_lock_refusal_preserve_other_entries() {
        let scratch = Scratch::new("stash-removal");
        let repo = repository(&scratch.0, "repo");
        fixture_git(&repo, &["stash", "push", "-qm", "older"]);
        fs::write(repo.join("a.txt"), "three\n").unwrap();
        fixture_git(&repo, &["stash", "push", "-qm", "newer"]);
        let mut backend = backend(&scratch.0, &repo, true);
        let list = full(backend.run(Job::Reload));
        let ids: Vec<_> = list.stashes.iter().map(|s| s.id.clone()).collect();
        let lock = repo.join(".git/refs/stash.lock");
        fs::write(&lock, "external lock owner").unwrap();
        let blocked = full(confirmed(&mut backend, Job::StashAction { id: ids[1].clone(), expected: ids.clone(), operation: "drop".into() }));
        assert!(blocked.stage_result.unwrap().contains("failed"));
        assert_eq!(blocked.stashes.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), ids);
        assert_eq!(fs::read_to_string(&lock).unwrap(), "external lock owner");
        fs::remove_file(lock).unwrap();
        let dropped = full(confirmed(&mut backend, Job::StashAction { id: ids[1].clone(), expected: ids.clone(), operation: "drop".into() }));
        assert!(dropped.stage_result.unwrap().contains("Dropped selected"));
        assert_eq!(dropped.stashes.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec![ids[0].clone()]);
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
        let popped = full(confirmed(&mut backend, Job::StashAction { id: ids[0].clone(), expected: vec![ids[0].clone()], operation: "pop".into() }));
        assert!(popped.stage_result.unwrap().contains("removed its source"));
        assert!(popped.stashes.is_empty());
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "three\n");
        assert!(!repo.join(".git/refs/stash").exists());
        assert!(!repo.join(".git/logs/refs/stash").exists());
        assert!(!repo.join(".git/packed-refs.lock").exists());
        let retained = git_text(&repo, &["for-each-ref", "--format=%(objectname)", "refs/srcmgr/recovery"]);
        assert_eq!(retained.lines().collect::<std::collections::BTreeSet<_>>().len(), 2, "both removed stash commits remain reachable, including a prior refused attempt");
        assert!(ids.iter().all(|id| retained.lines().any(|line| line == id)));
    }

    #[test]
    fn external_untracked_stash_is_inspectable_and_duplicate_identity_refuses_drop() {
        let scratch = Scratch::new("stash-external-payload");
        let repo = repository(&scratch.0, "repo");
        fixture_git(&repo, &["stash", "push", "-qu", "-m", "external capture"]);
        let mut backend = backend(&scratch.0, &repo, true);
        let original = full(backend.run(Job::Reload));
        let id = original.stashes[0].id.clone();
        let detail = main_view(backend.run(Job::Main { generation: 1, request: MainRequest::Stash { id: id.clone() } }));
        assert!(detail.lines.iter().any(|line| line.text == format!("Stash {id}")));
        assert!(detail.lines.iter().any(|line| line.text == "+new"), "captured untracked bytes must be visible before applying");
        assert!(!repo.join("u.txt").exists());
        full(confirmed(&mut backend, Job::StashAction { id: id.clone(), expected: vec![id.clone()], operation: "apply".into() }));
        assert_eq!(fs::read_to_string(repo.join("u.txt")).unwrap(), "new\n");
        fixture_git(&repo, &["stash", "push", "-qm", "intermediate entry"]);
        let intermediate = git_text(&repo, &["rev-parse", "refs/stash"]);
        fixture_git(&repo, &["stash", "store", "-m", "duplicate entry", &id]);
        let duplicated = full(backend.run(Job::Reload));
        let expected: Vec<_> = duplicated.stashes.iter().map(|s| s.id.clone()).collect();
        assert_eq!(expected, vec![id.clone(), intermediate, id.clone()]);
        assert!(notice(confirmed(&mut backend, Job::StashAction { id, expected: expected.clone(), operation: "drop".into() })).contains("ambiguous"));
        assert_eq!(git_text(&repo, &["stash", "list", "--format=%H"]).lines().collect::<Vec<_>>(), expected.iter().map(String::as_str).collect::<Vec<_>>());
    }

    fn full(reply: Reply) -> Snapshot {
        match reply {
            Reply::Full(snapshot) | Reply::Committed(snapshot) | Reply::BranchCreated(snapshot) => snapshot,
            other => panic!("expected a full snapshot, got {other:?}"),
        }
    }

    fn notice(reply: Reply) -> String {
        match reply {
            Reply::Notice(text) | Reply::ConfirmationFailed { text, .. } | Reply::PromptFailed { text, .. } => text,
            Reply::Full(snapshot) => snapshot.stage_result.expect("failure summary"),
            other => panic!("expected a notice, got {other:?}"),
        }
    }

    #[test]
    fn a_reload_lists_the_changed_files_branches_and_history() {
        let scratch = Scratch::new("reload");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let snapshot = full(backend.run(Job::Reload));
        assert_eq!(snapshot.files.len(), 2);
        assert!(snapshot.branches.iter().any(|branch| branch.name == "main" && branch.current));
        assert_eq!(snapshot.commits.len(), 1);
        assert_eq!(snapshot.commits[0].subject, "initial");
        assert!(snapshot.draft.is_none());
    }

    #[test]
    fn staging_a_file_through_the_window_updates_the_index_and_the_list() {
        let scratch = Scratch::new("stage");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let snapshot = full(backend.run(Job::SetFile {
            path: NativePath::new("a.txt"),
            stage: true,
        }));
        let row = snapshot
            .files
            .iter()
            .find(|file| file.path == NativePath::new("a.txt"))
            .expect("a.txt");
        assert!(row.has_staged && !row.has_unstaged);
        assert!(snapshot.stage_result.as_deref().unwrap().contains("1 file +1/-1"));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");
        let index = Command::new(git_executable())
            .current_dir(&repo)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&index.stdout).trim(), "a.txt");
    }

    #[test]
    fn stage_results_observe_partial_index_binary_and_unborn_state() {
        let scratch = Scratch::new("stage-summary");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let staged = full(backend.run(Job::SetFile { path: NativePath::new("a.txt"), stage: true }));
        fs::write(repo.join("a.txt"), "three\n").unwrap();
        let session = crate::open_trusted(&backend.options, &repo).unwrap();
        let summary = srcmgr_git::index_summary(&session.installation, &session.identity, Some("a.txt".as_ref())).unwrap();
        assert!(staged.stage_result.unwrap().contains("+1/-1"));
        assert_eq!((summary.added, summary.deleted), (1, 1));
        let unstaged = full(backend.run(Job::SetFile { path: NativePath::new("a.txt"), stage: false }));
        assert!(unstaged.stage_result.unwrap().contains("0 files +0/-0"));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "three\n");

        let unborn = scratch.0.join("unborn");
        fs::create_dir_all(&unborn).unwrap();
        fixture_git(&unborn, &["init", "-q", "-b", "main"]);
        fs::write(unborn.join("image.bin"), [0, 1, 2, 0]).unwrap();
        let mut other = super::tests::backend(&scratch.0.join("second"), &unborn, true);
        let snapshot = full(other.run(Job::SetFile { path: NativePath::new("image.bin"), stage: true }));
        let label = snapshot.stage_result.unwrap();
        assert!(label.contains("1 binary (lines unknown)"));
        let snapshot = full(other.run(Job::SetFile { path: NativePath::new("image.bin"), stage: false }));
        assert!(snapshot.stage_result.unwrap().contains("0 files"));
        assert_eq!(fs::read(unborn.join("image.bin")).unwrap(), [0, 1, 2, 0]);
    }

    #[test]
    fn a_path_that_is_not_changed_is_refused_with_a_notice() {
        let scratch = Scratch::new("refuse");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let text = notice(backend.run(Job::SetFile {
            path: NativePath::new("nothing-here.txt"),
            stage: true,
        }));
        assert!(text.starts_with("not a changed path"), "{text}");
    }

    #[test]
    fn an_ungranted_repository_is_refused_before_git_runs() {
        let scratch = Scratch::new("ungranted");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, false);
        let text = notice(backend.run(Job::Reload));
        assert!(text.contains("trust grant"), "{text}");
    }

    #[test]
    fn a_file_diff_comes_back_with_its_hunks_for_the_same_generation() {
        let scratch = Scratch::new("diff");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let reply = backend.run(Job::Main {
            generation: 7,
            request: MainRequest::File {
                path: NativePath::new("a.txt"),
                staged_view: false,
                untracked: false,
            },
        });
        match reply {
            Reply::Main { generation, view } => {
                assert_eq!(generation, 7);
                assert_eq!(view.hunks.len(), 1);
                assert!(view.file.is_some());
            }
            other => panic!("expected a main view, got {other:?}"),
        }
    }

    #[test]
    fn a_commit_detail_shows_the_subject_of_the_commit() {
        let scratch = Scratch::new("detail");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let snapshot = full(backend.run(Job::Reload));
        let reply = backend.run(Job::Main {
            generation: 1,
            request: MainRequest::Commit {
                id: snapshot.commits[0].id.clone(),
            },
        });
        match reply {
            Reply::Main { view, .. } => assert!(view.lines.iter().any(|line| line.text.contains("initial"))),
            other => panic!("expected a main view, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_commit_keeps_the_draft_and_a_saved_draft_survives_a_reload() {
        let scratch = Scratch::new("draft");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        assert_eq!(backend.run(Job::SaveDraft("first line\nsecond".to_owned())), Reply::DraftSaved(Some("first line\nsecond".to_owned())));
        let text = notice(backend.run(Job::Commit("first line\nsecond".to_owned())));
        assert_eq!(text, "nothing is staged to commit");
        let reloaded = full(backend.run(Job::Reload));
        assert_eq!(reloaded.draft.as_deref(), Some("first line\nsecond"));
    }

    #[test]
    fn a_hunk_action_stages_then_unstages_only_that_hunk() {
        let scratch = Scratch::new("hunk");
        let repo = repository(&scratch.0, "repo");
        // Ten unchanged lines between the two edits keep them in separate hunks.
        let middle = "l\n".repeat(10);
        fs::write(repo.join("a.txt"), format!("top\n{middle}bottom\n")).expect("base");
        fixture_git(&repo, &["add", "a.txt"]);
        fixture_git(&repo, &["commit", "-q", "-m", "lines"]);
        fs::write(repo.join("a.txt"), format!("TOP\n{middle}BOTTOM\n")).expect("two hunks");
        let mut backend = backend(&scratch.0, &repo, true);
        full(backend.run(Job::SetHunk {
            path: NativePath::new("a.txt"),
            staged_view: false,
            number: 1,
        }));
        let staged = Command::new(git_executable())
            .current_dir(&repo)
            .args(["diff", "--cached", "-U0"])
            .output()
            .expect("git");
        let patch = String::from_utf8_lossy(&staged.stdout).into_owned();
        assert!(patch.contains("+TOP") && !patch.contains("+BOTTOM"), "{patch}");
        let worktree = fs::read_to_string(repo.join("a.txt")).expect("read");
        assert_eq!(worktree, format!("TOP\n{middle}BOTTOM\n"), "the working tree is never rewritten");
        full(backend.run(Job::SetHunk {
            path: NativePath::new("a.txt"),
            staged_view: true,
            number: 1,
        }));
        let after = Command::new(git_executable())
            .current_dir(&repo)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("git");
        assert!(String::from_utf8_lossy(&after.stdout).trim().is_empty());
    }

    #[test]
    fn a_hunk_number_below_one_is_refused_without_panicking() {
        let scratch = Scratch::new("hunk-zero");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let text = notice(backend.run(Job::SetHunk {
            path: NativePath::new("a.txt"),
            staged_view: false,
            number: 0,
        }));
        assert_eq!(text, "hunk numbers start at 1");
    }

    #[test]
    fn a_selected_line_stages_only_that_line_and_leaves_the_worktree_alone() {
        let scratch = Scratch::new("lines");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        // Line 2 of the hunk is the added `two`; the removed `one` stays in the index.
        full(backend.run(Job::SetLines {
            path: NativePath::new("a.txt"),
            staged_view: false,
            number: 1,
            lines: vec![2],
        }));
        let index = Command::new(git_executable())
            .current_dir(&repo)
            .args(["show", ":a.txt"])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&index.stdout), "one\ntwo\n");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).expect("read"), "two\n");
    }

    #[test]
    fn deleting_the_checked_out_branch_is_refused_and_a_merged_one_is_deleted() {
        let scratch = Scratch::new("delete");
        let repo = repository(&scratch.0, "repo");
        fixture_git(&repo, &["branch", "old"]);
        let mut backend = backend(&scratch.0, &repo, true);
        assert!(!notice(confirmed(&mut backend, Job::DeleteBranch("main".to_owned()))).is_empty());
        full(confirmed(&mut backend, Job::DeleteBranch("old".to_owned())));
        let branches = Command::new(git_executable())
            .current_dir(&repo)
            .args(["branch", "--list", "old"])
            .output()
            .expect("git");
        assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());
    }

    #[test]
    fn discard_restores_a_tracked_file_and_refuses_untracked_without_trash() {
        let scratch = Scratch::new("discard");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        full(confirmed(&mut backend, Job::Discard {
            path: NativePath::new("a.txt"),
            untracked: false,
        }));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).expect("read"), "one\n");
        assert!(repo.join("u.txt").exists(), "a tracked discard leaves untracked files alone");
        notice(confirmed(&mut backend, Job::Discard {
            path: NativePath::new("u.txt"),
            untracked: true,
        }));
        assert!(repo.join("u.txt").exists(), "no silent permanent-delete fallback");
        let copies = fs::read_dir(backend.options.profile.as_ref().unwrap().join("recovery")).unwrap();
        let saved = copies.into_iter().next().unwrap().unwrap().path().join("worktree.bin");
        assert_eq!(fs::read_to_string(saved).unwrap(), "two\n");
    }

    #[test]
    fn discard_hunk_reverts_only_that_hunk_in_the_working_tree() {
        let scratch = Scratch::new("discard-hunk");
        let repo = repository(&scratch.0, "repo");
        let middle = "l\n".repeat(10);
        fs::write(repo.join("a.txt"), format!("top\n{middle}bottom\n")).expect("base");
        fixture_git(&repo, &["add", "a.txt"]);
        fixture_git(&repo, &["commit", "-q", "-m", "lines"]);
        fs::write(repo.join("a.txt"), format!("TOP\n{middle}BOTTOM\n")).expect("two hunks");
        let mut backend = backend(&scratch.0, &repo, true);
        full(confirmed(&mut backend, Job::DiscardHunk {
            path: NativePath::new("a.txt"),
            index: 0,
        }));
        let worktree = fs::read_to_string(repo.join("a.txt")).expect("read");
        assert_eq!(worktree, format!("top\n{middle}BOTTOM\n"), "only the first hunk is reverted");
    }

    #[test]
    fn confirmation_rejects_changed_content_failed_copy_and_replay() {
        let scratch = Scratch::new("confirm-guards");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let action = Job::Discard { path: NativePath::new("a.txt"), untracked: false };
        let Reply::Prepared { ticket, .. } = backend.run(Job::Prepare { generation: 1, action: Box::new(action.clone()) }) else { panic!("prepared") };
        fs::write(repo.join("a.txt"), "newer content\n").unwrap();
        assert!(notice(backend.run(Job::ExecutePrepared(ticket.clone()))).contains("file changed"));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "newer content\n");
        assert!(notice(backend.run(Job::ExecutePrepared(ticket))).contains("consumed"));

        let root = backend.options.profile.as_ref().unwrap().join("recovery");
        fs::write(&root, "storage blocked").unwrap();
        assert!(notice(confirmed(&mut backend, action)).contains("refused before execution"));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "newer content\n");
        assert!(notice(backend.run(Job::Discard { path: NativePath::new("a.txt"), untracked: false })).contains("confirmation"));
    }

    #[test]
    fn stash_saves_tracked_changes_and_apply_brings_them_back_and_keeps_the_stash() {
        let scratch = Scratch::new("stash");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        full(backend.run(Job::Stash));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).expect("read"), "one\n");
        assert!(repo.join("u.txt").exists(), "untracked files stay where they are");
        let selected = full(backend.run(Job::Reload)).stashes;
        full(confirmed(&mut backend, Job::StashAction { id: selected[0].id.clone(), expected: selected.iter().map(|s| s.id.clone()).collect(), operation: "apply".into() }));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).expect("read"), "two\n");
        let list = Command::new(git_executable()).current_dir(&repo).args(["stash", "list"]).output().expect("git");
        assert!(!list.stdout.is_empty(), "apply keeps the stash");
    }

    #[test]
    fn detach_checks_out_a_commit_and_refuses_a_value_that_is_not_an_id() {
        let scratch = Scratch::new("detach");
        let repo = repository(&scratch.0, "repo");
        fixture_git(&repo, &["commit", "-q", "-am", "second"]);
        let first = Command::new(git_executable())
            .current_dir(&repo)
            .args(["rev-parse", "HEAD~1"])
            .output()
            .expect("git");
        let first = String::from_utf8_lossy(&first.stdout).trim().to_owned();
        let mut backend = backend(&scratch.0, &repo, true);
        assert!(!notice(confirmed(&mut backend, Job::Detach("--orphan".to_owned()))).is_empty());
        full(confirmed(&mut backend, Job::Detach(first)));
        let branch = Command::new(git_executable())
            .current_dir(&repo)
            .args(["branch", "--show-current"])
            .output()
            .expect("git");
        assert!(branch.stdout.is_empty(), "HEAD is detached");
    }

    #[test]
    fn a_switch_that_would_overwrite_a_local_edit_is_refused_and_the_branch_stays() {
        let scratch = Scratch::new("switch");
        let repo = repository(&scratch.0, "repo");
        // Build the feature branch with a clean tree, then bring the local edit back.
        fixture_git(&repo, &["stash", "-q"]);
        fixture_git(&repo, &["checkout", "-q", "-b", "feature"]);
        fs::write(repo.join("a.txt"), "feature\n").expect("branch edit");
        fixture_git(&repo, &["commit", "-q", "-am", "feature edit"]);
        fixture_git(&repo, &["checkout", "-q", "main"]);
        fixture_git(&repo, &["stash", "pop", "-q"]);
        let mut backend = backend(&scratch.0, &repo, true);
        let text = notice(backend.run(Job::Switch("feature".to_owned())));
        assert!(!text.is_empty());
        let head = Command::new(git_executable())
            .current_dir(&repo)
            .args(["branch", "--show-current"])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).expect("read"), "two\n");
    }

    #[test]
    fn a_successful_commit_clears_the_draft_and_adds_a_commit() {
        let scratch = Scratch::new("commit");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        backend.run(Job::SaveDraft("second commit".to_owned()));
        backend.run(Job::SetFile {
            path: NativePath::new("a.txt"),
            stage: true,
        });
        let snapshot = full(backend.run(Job::Commit("second commit".to_owned())));
        assert!(snapshot.draft.is_none());
        assert_eq!(snapshot.commits.len(), 2);
        assert_eq!(snapshot.commits[0].subject, "second commit");
    }

    #[test]
    fn a_created_commit_is_acknowledged_when_post_commit_profile_storage_is_locked() {
        let scratch = Scratch::new("commit-created-profile-lock");
        let repo = repository(&scratch.0, "repo");
        let lock = lock_profile_after_git_hook(&scratch.0, &repo, "post-commit");
        let mut backend = backend(&scratch.0, &repo, true);
        backend.run(Job::SaveDraft("already created".into()));
        full(backend.run(Job::SetFile { path: NativePath::new("a.txt"), stage: true }));
        let before = git_text(&repo, &["rev-parse", "HEAD"]);
        let reply = backend.run(Job::Commit("already created".into()));
        let created = git_text(&repo, &["rev-parse", "HEAD"]);
        assert_ne!(created, before, "Git actually created the commit");
        assert_eq!(git_text(&repo, &["log", "-1", "--format=%s"]), "already created");
        assert!(git_text(&repo, &["diff", "--cached", "--name-only"]).is_empty());
        assert_eq!(fs::read(&lock).unwrap(), b"owned post-write lock", "foreign lock is preserved");
        fs::remove_file(lock).unwrap();
        assert_eq!(full(backend.run(Job::Reload)).draft.as_deref(), Some("already created"), "failed cleanup leaves the durable draft intact");
        assert!(matches!(reply, Reply::PromptCompleted { action: srcmgr_ui::PromptAction::Commit, ref text } if text.contains(&created)), "acknowledgement must name the actually created commit despite unavailable storage: {reply:?}");
    }

    #[test]
    fn a_created_branch_is_acknowledged_when_post_checkout_profile_storage_is_locked() {
        let scratch = Scratch::new("branch-created-profile-lock");
        let repo = repository(&scratch.0, "repo");
        let lock = lock_profile_after_git_hook(&scratch.0, &repo, "post-checkout");
        let mut backend = backend(&scratch.0, &repo, true);
        let reply = backend.run(Job::CreateBranch("created-review".into()));
        assert_eq!(git_text(&repo, &["branch", "--show-current"]), "created-review");
        assert_eq!(git_text(&repo, &["show-ref", "--verify", "--hash", "refs/heads/created-review"]), git_text(&repo, &["rev-parse", "HEAD"]));
        assert_eq!(fs::read(&lock).unwrap(), b"owned post-write lock");
        fs::remove_file(lock).unwrap();
        assert!(matches!(reply, Reply::PromptCompleted { action: srcmgr_ui::PromptAction::CreateBranch, ref text } if text.contains("created-review")), "acknowledge the actual branch even when the following snapshot is unavailable: {reply:?}");
        assert!(full(backend.run(Job::Reload)).branches.iter().any(|branch| branch.name == "created-review" && branch.current));
    }

    #[test]
    fn a_blank_draft_clears_the_saved_one() {
        let scratch = Scratch::new("blank");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        backend.run(Job::SaveDraft("keep me".to_owned()));
        assert_eq!(backend.run(Job::SaveDraft("   ".to_owned())), Reply::DraftSaved(None));
        assert!(full(backend.run(Job::Reload)).draft.is_none());
    }

    #[test]
    fn draft_save_acknowledges_durable_state_and_refuses_a_foreign_profile_lock() {
        let scratch = Scratch::new("draft-ack");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        assert_eq!(backend.run(Job::SaveDraft("original".into())), Reply::DraftSaved(Some("original".into())));
        let lock = backend.options.profile.as_ref().unwrap().join("profile.lock");
        fs::write(&lock, b"foreign profile writer").unwrap();
        assert!(matches!(backend.run(Job::SaveDraft("replacement".into())), Reply::PromptFailed { action: srcmgr_ui::PromptAction::SaveDraft, .. }));
        assert_eq!(fs::read(&lock).unwrap(), b"foreign profile writer");
        fs::remove_file(lock).unwrap();
        assert_eq!(full(backend.run(Job::Reload)).draft.as_deref(), Some("original"));
        assert_eq!(backend.run(Job::SaveDraft("replacement".into())), Reply::DraftSaved(Some("replacement".into())));
        assert_eq!(full(backend.run(Job::Reload)).draft.as_deref(), Some("replacement"));
    }

    #[test]
    fn branch_creation_acknowledges_the_actual_ref_and_tags_its_own_refusal() {
        let scratch = Scratch::new("branch-ack");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        let head = git_text(&repo, &["rev-parse", "HEAD"]);
        let Reply::BranchCreated(snapshot) = backend.run(Job::CreateBranch("review".into())) else { panic!("branch completion") };
        assert!(snapshot.branches.iter().any(|branch| branch.name == "review"));
        assert_eq!(git_text(&repo, &["rev-parse", "refs/heads/review"]), head);
        assert!(matches!(backend.run(Job::CreateBranch("review".into())), Reply::PromptFailed { action: srcmgr_ui::PromptAction::CreateBranch, .. }));
        assert_eq!(git_text(&repo, &["rev-parse", "refs/heads/review"]), head);
    }
}
