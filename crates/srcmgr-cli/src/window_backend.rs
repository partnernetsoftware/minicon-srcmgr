//! The status window's Git side. Every job re-runs the trust gate and reads
//! fresh state, so a grant revoked or a config changed while the window is
//! open stops the next action. Results are plain data for the window.

use std::path::PathBuf;

use srcmgr_domain::{ChangeKind, Generation, NativePath};
use srcmgr_ui::{Backend, BranchRow, CommitRow, FileTarget, Job, MainRequest, MainView, Reply, Snapshot};

use crate::{
    BACKEND_GIT, DEFAULT_HISTORY_LIMIT, HISTORY_LIMIT_KEY, Options, open_profile_store, open_trusted,
    parse_history_limit, repository_key,
};

pub struct WindowBackend {
    pub options: Options,
    pub worktree: PathBuf,
}

impl Backend for WindowBackend {
    fn run(&mut self, job: Job) -> Reply {
        self.perform(job).unwrap_or_else(Reply::Notice)
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
        Ok(Snapshot::new(format!("MiniCon SrcMgr - {name}"), &status, branches, commits, draft))
    }

    fn perform(&mut self, job: Job) -> Result<Reply, String> {
        Ok(match job {
            Job::Reload => Reply::Full(self.snapshot()?),
            Job::Main { generation, request } => Reply::Main { generation, view: self.main(request)? },
            Job::SetFile { path, stage } => {
                self.set_file(&path, stage)?;
                Reply::Full(self.snapshot()?)
            }
            Job::SetHunk { path, staged_view, number } => {
                let index = number.checked_sub(1).ok_or("hunk numbers start at 1")?;
                let session = open_trusted(&self.options, &self.worktree)?;
                srcmgr_git::apply_hunk(&session.installation, &session.identity, path.as_os_str(), index, staged_view)
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
                Reply::Full(self.snapshot()?)
            }
            Job::Commit(message) => {
                self.commit(&message)?;
                Reply::Full(self.snapshot()?)
            }
            Job::SaveDraft(text) => {
                self.save_draft(&text)?;
                Reply::Full(self.snapshot()?)
            }
        })
    }

    fn main(&self, request: MainRequest) -> Result<MainView, String> {
        let session = open_trusted(&self.options, &self.worktree)?;
        match request {
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
    fn commit(&self, message: &str) -> Result<(), String> {
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
        srcmgr_git::commit(&session.installation, &session.identity, message).map_err(|e| e.to_string())?;
        open_profile_store(&self.options)?.delete_draft(BACKEND_GIT, &key).map_err(|e| e.to_string())
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
        }
    }

    fn full(reply: Reply) -> Snapshot {
        match reply {
            Reply::Full(snapshot) => snapshot,
            other => panic!("expected a full snapshot, got {other:?}"),
        }
    }

    fn notice(reply: Reply) -> String {
        match reply {
            Reply::Notice(text) => text,
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
        let index = Command::new(git_executable())
            .current_dir(&repo)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("git");
        assert_eq!(String::from_utf8_lossy(&index.stdout).trim(), "a.txt");
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
        let saved = full(backend.run(Job::SaveDraft("first line\nsecond".to_owned())));
        assert_eq!(saved.draft.as_deref(), Some("first line\nsecond"));
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
    fn a_blank_draft_clears_the_saved_one() {
        let scratch = Scratch::new("blank");
        let repo = repository(&scratch.0, "repo");
        let mut backend = backend(&scratch.0, &repo, true);
        backend.run(Job::SaveDraft("keep me".to_owned()));
        let snapshot = full(backend.run(Job::SaveDraft("   ".to_owned())));
        assert!(snapshot.draft.is_none());
    }
}
