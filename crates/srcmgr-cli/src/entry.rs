//! Repository-independent entry. Metadata browsing never invokes repository
//! Git; an explicit, one-use trust review precedes any untrusted repo command.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use srcmgr_domain::NativePath;
use srcmgr_state::{RecentRepository, TrustError, fingerprint};
use srcmgr_ui::{Backend, Job, PromptAction, RecentRow, Reply, TrustReview};

use crate::window_backend::WindowBackend;
use crate::{Options, canonical_worktree, git_dir_of, open_profile_store, trust_store};

struct PendingTrust {
    ticket: String,
    worktree: PathBuf,
    git_directory: PathBuf,
    fingerprint: String,
    expires: Instant,
}

pub struct ApplicationBackend {
    options: Options,
    active: Option<WindowBackend>,
    trust: Option<PendingTrust>,
}

impl ApplicationBackend {
    pub fn welcome(options: Options) -> Self {
        Self {
            options,
            active: None,
            trust: None,
        }
    }

    fn recent(&self) -> Result<Reply, String> {
        let entries = open_profile_store(&self.options)?
            .recent_repositories()
            .map_err(|e| e.to_string())?;
        Ok(Reply::Welcome(
            entries
                .into_iter()
                .map(|entry| {
                    let label = entry.native_path().map_or_else(
                        |_| {
                            format!(
                                "{} location; unavailable on this platform",
                                entry.path.tag()
                            )
                        },
                        |path| path.display_label(),
                    );
                    RecentRow {
                        backend: entry.backend,
                        path: entry.path,
                        label,
                    }
                })
                .collect(),
        ))
    }

    fn open(&mut self, path: NativePath) -> Result<Reply, String> {
        self.trust = None;
        let worktree = canonical_worktree(std::path::Path::new(path.as_os_str()))?;
        let git_directory = git_dir_of(&worktree)?;
        let (_, grants) = trust_store(&self.options)?;
        match grants.verify(&git_directory, &worktree) {
            Ok(()) => self.activate(worktree),
            Err(TrustError::Untrusted | TrustError::Changed) => {
                let digest = fingerprint(&git_directory, &worktree).map_err(|e| e.to_string())?;
                let bytes = minicon_platform::entropy::secure_random_array::<16>()
                    .map_err(|e| format!("cannot prepare trust review: {e:?}"))?;
                let ticket: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                self.trust = Some(PendingTrust {
                    ticket: ticket.clone(),
                    worktree: worktree.clone(),
                    git_directory: git_directory.clone(),
                    fingerprint: digest.clone(),
                    expires: Instant::now() + Duration::from_secs(60),
                });
                Ok(Reply::TrustReview(TrustReview {
                    ticket,
                    worktree: NativePath::new(worktree.into_os_string()),
                    git_directory: NativePath::new(git_directory.into_os_string()),
                    fingerprint: digest,
                    submitted: false,
                }))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn activate(&mut self, worktree: PathBuf) -> Result<Reply, String> {
        let (candidate, snapshot) = WindowBackend::open_target(&self.options, &worktree)?;
        self.active = Some(candidate);
        Ok(Reply::RepositoryOpened(snapshot))
    }

    fn grant(&mut self, ticket: &str) -> Result<Reply, String> {
        let pending = self
            .trust
            .take()
            .ok_or("trust review is missing, expired or already consumed; open again")?;
        if ticket != pending.ticket || Instant::now() >= pending.expires {
            return Err("trust review expired or changed; open again".into());
        }
        if canonical_worktree(&pending.worktree)? != pending.worktree
            || git_dir_of(&pending.worktree)? != pending.git_directory
            || fingerprint(&pending.git_directory, &pending.worktree).map_err(|e| e.to_string())?
                != pending.fingerprint
        {
            return Err("repository metadata changed after review; open and review again".into());
        }
        let (_, mut grants) = trust_store(&self.options)?;
        grants.grant(pending.git_directory, pending.fingerprint);
        grants.save().map_err(|e| e.to_string())?;
        self.activate(pending.worktree)
            .map_err(|error| format!("Trust was recorded; opening is unavailable: {error}"))
    }
}

impl Backend for ApplicationBackend {
    fn run(&mut self, job: Job) -> Reply {
        match job {
            Job::LoadWelcome => self.recent().unwrap_or_else(Reply::Notice),
            Job::ForgetRecent { backend, path } => (|| {
                open_profile_store(&self.options)?
                    .forget_repository(&RecentRepository { backend, path })
                    .map_err(|e| e.to_string())?;
                self.recent()
            })()
            .unwrap_or_else(Reply::Notice),
            Job::OpenRepository(path) => {
                self.open(path).unwrap_or_else(|text| Reply::PromptFailed {
                    action: PromptAction::OpenRepository,
                    text,
                })
            }
            Job::GrantRepositoryTrust(ticket) => {
                self.grant(&ticket)
                    .unwrap_or_else(|text| Reply::PromptFailed {
                        action: PromptAction::OpenRepository,
                        text,
                    })
            }
            Job::CancelEntry => {
                self.trust = None;
                Reply::NoChange
            }
            job => self.active.as_mut().map_or_else(
                || Reply::Notice("Open a repository before requesting repository work".into()),
                |active| active.run(job),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_backend::tests::{Scratch, fixture_git, git_executable, repository};

    fn app(root: &std::path::Path) -> ApplicationBackend {
        ApplicationBackend::welcome(Options {
            profile: Some(root.join("profile")),
            git: Some(root.join("git-that-must-not-run")),
            ..Options::default()
        })
    }
    fn review(app: &mut ApplicationBackend, path: &std::path::Path) -> TrustReview {
        let Reply::TrustReview(review) =
            app.run(Job::OpenRepository(NativePath::new(path.as_os_str())))
        else {
            panic!("expected metadata-only review without installed Git");
        };
        review
    }

    #[test]
    fn entry_cancel_and_changed_metadata_never_create_a_trust_grant() {
        let root = Scratch::new("entry-review-cancel");
        let repo = repository(&root.0, "repo");
        let mut app = app(&root.0);
        assert!(matches!(app.run(Job::LoadWelcome), Reply::Welcome(rows) if rows.is_empty()));
        let first = review(&mut app, &repo);
        assert!(!root.0.join("profile/trust.v1").exists());
        assert!(matches!(app.run(Job::CancelEntry), Reply::NoChange));
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(first.ticket)),
            Reply::PromptFailed { .. }
        ));
        let second = review(&mut app, &repo);
        fixture_git(&repo, &["config", "user.name", "changed after preview"]);
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(second.ticket)),
            Reply::PromptFailed { .. }
        ));
        assert!(!root.0.join("profile/trust.v1").exists());
        assert!(app.active.is_none());
        assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"two\n");
    }

    #[test]
    fn entry_expiry_and_replay_are_rejected_and_explicit_grant_opens_the_target() {
        let root = Scratch::new("entry-one-use");
        let repo = repository(&root.0, "repo");
        let mut app = app(&root.0);
        let expired = review(&mut app, &repo);
        app.trust.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(expired.ticket)),
            Reply::PromptFailed { .. }
        ));
        assert!(!root.0.join("profile/trust.v1").exists());
        let wrong = review(&mut app, &repo);
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust("different ticket".into())),
            Reply::PromptFailed { .. }
        ));
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(wrong.ticket)),
            Reply::PromptFailed { .. }
        ));
        app.options.git = Some(git_executable());
        let approved = review(&mut app, &repo);
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(approved.ticket.clone())),
            Reply::RepositoryOpened(_)
        ));
        let canonical = std::fs::canonicalize(&repo).unwrap();
        assert_eq!(app.active.as_ref().unwrap().worktree, canonical);
        let (_, grants) = trust_store(&app.options).unwrap();
        assert!(
            grants
                .verify(&git_dir_of(&canonical).unwrap(), &canonical)
                .is_ok()
        );
        assert!(matches!(
            app.run(Job::GrantRepositoryTrust(approved.ticket)),
            Reply::PromptFailed { .. }
        ));
        assert_eq!(app.active.as_ref().unwrap().worktree, canonical);
    }

    #[test]
    fn entry_recent_browsing_and_forgetting_are_passive_and_preserve_drafts() {
        let root = Scratch::new("entry-passive-recents");
        let missing = root.0.join("missing worktree");
        let mut app = app(&root.0);
        {
            let mut store = open_profile_store(&app.options).unwrap();
            store.remember_repository("git", &missing).unwrap();
            store
                .put_draft("git", "saved-key", "keep this draft")
                .unwrap();
        }
        let Reply::Welcome(rows) = app.run(Job::LoadWelcome) else {
            panic!("metadata should not need Git");
        };
        assert_eq!(rows.len(), 1);
        assert!(!missing.exists());
        assert!(app.active.is_none());
        assert!(
            matches!(app.run(Job::ForgetRecent { backend: rows[0].backend.clone(), path: rows[0].path.clone() }), Reply::Welcome(rows) if rows.is_empty())
        );
        assert_eq!(
            open_profile_store(&app.options)
                .unwrap()
                .get_draft("git", "saved-key")
                .unwrap()
                .as_deref(),
            Some("keep this draft")
        );
    }
}
