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

struct PendingCreate {
    ticket: String,
    target: srcmgr_git::InitializationTarget,
    branch: String,
    expires: Instant,
}

struct PendingIdentity {
    ticket: String,
    worktree: PathBuf,
    config: srcmgr_git::IdentityConfig,
    expires: Instant,
}

pub struct ApplicationBackend {
    options: Options,
    active: Option<WindowBackend>,
    trust: Option<PendingTrust>,
    create: Option<PendingCreate>,
    identity: Option<PendingIdentity>,
}

impl ApplicationBackend {
    pub fn welcome(options: Options) -> Self {
        Self {
            options,
            active: None,
            trust: None,
            create: None,
            identity: None,
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
        self.open_mode(path, false)
    }

    fn open_mode(&mut self, path: NativePath, force_review: bool) -> Result<Reply, String> {
        self.identity = None;
        self.create = None;
        self.trust = None;
        let worktree = canonical_worktree(std::path::Path::new(path.as_os_str()))?;
        let git_directory = git_dir_of(&worktree)?;
        let (_, grants) = trust_store(&self.options)?;
        match grants.verify(&git_directory, &worktree) {
            Ok(()) if !force_review => self.activate(worktree),
            Ok(()) | Err(TrustError::Untrusted | TrustError::Changed) => {
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

    fn prepare_create(&mut self, path: NativePath, branch: String) -> Result<Reply, String> {
        self.identity = None;
        self.trust = None;
        self.create = None;
        let target = srcmgr_git::InitializationTarget::prepare(std::path::Path::new(path.as_os_str())).map_err(|e| e.to_string())?;
        if branch.is_empty() || branch.starts_with('-') || branch == "HEAD" || branch.len() > 255 {
            return Err("Choose a valid initial branch name (up to 255 UTF-8 bytes)".into());
        }
        let bytes = minicon_platform::entropy::secure_random_array::<16>().map_err(|e| format!("cannot prepare creation review: {e:?}"))?;
        let ticket: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let reply = Reply::CreateReview {
            ticket: ticket.clone(), path: NativePath::new(target.path().as_os_str()), branch: branch.clone(),
            new_directory: target.is_new_directory(),
            entries: target.entries().iter().map(|name| NativePath::new(name).display_label()).collect(),
            more_entries: target.has_more_entries(),
        };
        self.create = Some(PendingCreate { ticket, target, branch, expires: Instant::now() + Duration::from_secs(60) });
        Ok(reply)
    }

    fn create(&mut self, ticket: &str) -> Result<Reply, String> {
        let pending = self.create.take().ok_or("Creation review is missing or already consumed; review again")?;
        if pending.ticket != ticket || Instant::now() >= pending.expires {
            return Err("Creation review expired or changed; review again".into());
        }
        let executable = std::fs::canonicalize(crate::selected_git(&self.options)?).map_err(|_| "Selected Git is unavailable; choose an installed Git")?;
        if executable.starts_with(pending.target.path()) {
            return Err("Select an installed Git outside the initialization target".into());
        }
        let git = srcmgr_git::GitInstallation::probe(executable).map_err(|error| error.to_string())?;
        let path = pending.target.execute(&git, &pending.branch).map_err(|error| format!("Create did not complete: {error}. Inspect the target before retrying; partial metadata may remain. Existing files were not staged or committed."))?;
        self.open_mode(NativePath::new(path.into_os_string()), true).map_err(|error| format!("Repository was created; opening is unavailable: {error}"))
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

    fn git_settings(&self) -> Result<Reply, String> {
        let saved = open_profile_store(&self.options)?.git_executable().map_err(|e| e.to_string())?;
        let selected = self.options.git.as_ref().map(|path| NativePath::new(path.as_os_str())).or_else(|| saved.clone());
        let source = if self.options.git.is_some() { "Current session selection (initial --git overrides the saved preference)." } else if saved.is_some() { "Saved application preference." } else { "Automatic PATH discovery; no saved executable." };
        Ok(Reply::GitSettings { path: selected, detail: format!("{source}\nNo executable was run to display these settings.\nSelect an installed Git 2.43 or newer. Leave blank to use PATH.\nInstall through your trusted OS package manager or git-scm.com/downloads.\nTest and save runs the selected program's --version, then saves only on success.\nSaving applies to this session and future launches; it does not install Git.\nThis changes an application preference, not repository or global Git config.") })
    }

    fn load_identity(&mut self) -> Result<Reply, String> {
        self.identity = None;
        let active = self.active.as_ref().ok_or("Open and trust a repository before inspecting its identity")?;
        let session = crate::open_trusted(&active.options, &active.worktree)?;
        let config = srcmgr_git::IdentityConfig::read(&session.installation, &session.identity).map_err(|e| e.to_string())?;
        let bytes = minicon_platform::entropy::secure_random_array::<16>().map_err(|e| format!("Cannot prepare identity settings: {e:?}"))?;
        let ticket: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let label = |text: &str| NativePath::new(text).display_label();
        let source = |value: &Option<srcmgr_git::IdentityValue>| value.as_ref().map_or_else(|| "not configured".into(), |value| format!("{}: {}", label(&value.scope), label(&value.origin)));
        let detail = format!("Repository: {}\nSave writes user.name and user.email to this repository's .git/config only.\nName source: {}\nEmail source: {}\nResolved author: {}\nResolved committer: {}\nEnvironment overrides can differ from configuration. Global/system identity is inherited, never changed here.\nSave preserves other settings/comments. Changed config or an existing lock refuses the write.\nAfter saving, review the changed repository configuration before further Git work.\nPreview expires after 60 seconds.", NativePath::new(active.worktree.as_os_str()).display_label(), source(&config.name), source(&config.email), config.author.as_deref().map_or_else(|| "unavailable: configure identity".into(), &label), config.committer.as_deref().map_or_else(|| "unavailable: configure identity".into(), &label));
        let reply = Reply::IdentitySettings { ticket: ticket.clone(), name: config.name.as_ref().map_or_else(String::new, |value| value.value.clone()), email: config.email.as_ref().map_or_else(String::new, |value| value.value.clone()), detail };
        self.identity = Some(PendingIdentity { ticket, worktree: active.worktree.clone(), config, expires: Instant::now() + Duration::from_secs(60) });
        Ok(reply)
    }

    fn save_identity(&mut self, ticket: &str, name: &str, email: &str) -> Result<Reply, String> {
        let pending = self.identity.take().ok_or("Identity preview is missing or consumed; reload before saving")?;
        if ticket != pending.ticket || Instant::now() >= pending.expires { return Err("Identity preview expired or changed; reload before saving".into()); }
        let active = self.active.as_ref().ok_or("Repository is no longer open")?;
        if active.worktree != pending.worktree { return Err("Active repository changed; identity was not saved".into()); }
        let session = crate::open_trusted(&active.options, &active.worktree)?;
        let durability_warning = pending.config.save_local(&session.installation, &session.identity, name, email).map_err(|e| format!("Identity was not published: {e}; reload before retrying"))?;
        let path = NativePath::new(pending.worktree.into_os_string());
        let mut detail = "Identity saved to this repository's config. Existing files, index and refs were not changed. Review configuration trust before continuing.".to_owned();
        if durability_warning { detail.push_str(" Parent-directory durability could not be acknowledged; inspect before assuming crash persistence."); }
        let review = match self.open_mode(path.clone(), true) {
            Ok(Reply::TrustReview(review)) => Some(review),
            Ok(_) => unreachable!("forced metadata review never activates directly"),
            Err(error) => { detail.push_str(&format!(" Opening is unavailable: {error}")); None }
        };
        Ok(Reply::IdentitySaved { path, review, detail })
    }

    fn configure_git(&mut self, path: Option<NativePath>) -> Result<Reply, String> {
        let executable = match &path {
            Some(path) => PathBuf::from(path.as_os_str()),
            None => crate::find_git_on_path().ok_or("No Git was found on PATH; select an installed executable")?,
        };
        let executable = std::fs::canonicalize(executable).map_err(|_| "Selected executable is unavailable; previous Git preference was preserved")?;
        if !executable.is_file() { return Err("Select a Git executable file".into()); }
        // Explicit selection is execution authority, but cannot admit a known
        // active or pending repository's executable before its trust boundary.
        let mut roots = Vec::new();
        if let Some(active) = &self.active { roots.push(active.worktree.as_path()); }
        if let Some(trust) = &self.trust { roots.push(trust.worktree.as_path()); }
        if let Some(create) = &self.create { roots.push(create.target.path()); }
        if roots.iter().any(|root| executable.starts_with(root)) {
            return Err("Choose an installed Git outside the selected repositories".into());
        }
        let installation = srcmgr_git::GitInstallation::probe(&executable).map_err(|e| e.to_string())?;
        installation.require_supported().map_err(|e| e.to_string())?;
        let saved = path.as_ref().map(|_| NativePath::new(executable.as_os_str()));
        open_profile_store(&self.options)
            .and_then(|mut store| store.set_git_executable(saved.as_ref()).map_err(|e| e.to_string()))
            .map_err(|e| format!("Git was tested but not saved or applied: {e}"))?;
        self.options.git = Some(executable.clone());
        if let Some(active) = self.active.as_mut() {
            active.options.git = Some(executable.clone());
            active.prepared = None;
            active.commit_review = None;
        }
        self.trust = None;
        self.create = None;
        self.identity = None;
        Ok(Reply::GitSettings { path: saved, detail: format!("Saved and applied. Git {}.{}.{}\nEffective executable: {}\n{}\nNo repository/global Git config was changed.", installation.version().major, installation.version().minor, installation.version().patch, NativePath::new(executable.into_os_string()).display_label(), if path.is_none() { "Future launches discover Git on PATH." } else { "Future launches use this saved executable unless --git overrides it." }) })
    }
}

impl Backend for ApplicationBackend {
    fn run(&mut self, job: Job) -> Reply {
        match job {
            Job::LoadIdentity => self.load_identity().unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::IdentitySettings, text }),
            Job::SaveIdentity { ticket, name, email } => self.save_identity(&ticket, &name, &email).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::IdentitySettings, text }),
            Job::LoadGitSettings => self.git_settings().unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::GitSettings, text }),
            Job::ConfigureGit(path) => self.configure_git(path).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::GitSettings, text }),
            Job::PrepareCreate { path, branch } => self.prepare_create(path, branch).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::CreateRepository, text }),
            Job::ExecuteCreate(ticket) => self.create(&ticket).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::CreateRepository, text }),
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
                self.identity = None;
                self.trust = None;
                self.create = None;
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

    #[test]
    fn entry_repository_local_git_is_refused_before_even_the_version_probe() {
        let root = Scratch::new("entry-local-git-no-probe");
        let repo = repository(&root.0, "repo");
        let mut app = trusted_app(&root.0, &repo);
        let marker = root.0.join("git-was-executed");
        let source = root.0.join("version_canary.rs");
        let executable = repo.join(if cfg!(windows) { "git-canary.exe" } else { "git-canary" });
        std::fs::write(&source, format!("fn main() {{ std::fs::write({:?}, b\"ran\").unwrap(); println!(\"git version 2.56.0\"); }}", marker.as_os_str())).unwrap();
        let compiled = std::process::Command::new("rustc").arg(&source).arg("-o").arg(&executable).output().unwrap();
        assert!(compiled.status.success(), "compile execution canary: {}", String::from_utf8_lossy(&compiled.stderr));
        assert!(std::process::Command::new(&executable).arg("--version").status().unwrap().success());
        assert_eq!(std::fs::read(&marker).unwrap(), b"ran", "positive control must detect execution");
        std::fs::remove_file(&marker).unwrap();
        let options = Options { git: Some(executable.clone()), ..app.options.clone() };
        assert!(crate::open_trusted(&options, &repo).is_err());
        assert!(!marker.exists(), "repository-local program ran before rejection");
        assert!(matches!(app.run(Job::ConfigureGit(Some(NativePath::new(executable.into_os_string())))), Reply::PromptFailed { action: PromptAction::GitSettings, .. }));
        assert!(!marker.exists(), "settings validation executed a repository-local program");
    }

    fn trusted_app(root: &std::path::Path, repo: &std::path::Path) -> ApplicationBackend {
        let mut app = app(root);
        app.options.git = Some(git_executable());
        let approved = review(&mut app, repo);
        assert!(matches!(app.run(Job::GrantRepositoryTrust(approved.ticket)), Reply::RepositoryOpened(_)));
        app
    }

    #[test]
    fn entry_identity_save_changes_only_local_config_then_requires_fresh_trust() {
        let root = Scratch::new("entry-identity-local");
        let repo = repository(&root.0, "repo");
        let config_path = repo.join(".git/config");
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&config_path).unwrap().write_all(b"\n# keep this comment\n[custom]\n\tsetting = untouched\n").unwrap();
        let mut app = trusted_app(&root.0, &repo);
        let head = std::fs::read(repo.join(".git/HEAD")).unwrap();
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        let Reply::IdentitySettings { ticket, name, email, detail } = app.run(Job::LoadIdentity) else { panic!("expected scoped identity settings"); };
        assert_eq!(name, "Fixture");
        assert_eq!(email, "fixture@example.invalid");
        assert!(detail.contains("Name source: local:"));
        let result = app.run(Job::SaveIdentity { ticket: ticket.clone(), name: "New Identity 中文".into(), email: "new@example.invalid".into() });
        let Reply::IdentitySaved { review: Some(review), .. } = result else { panic!("known config publication must be acknowledged with a fresh trust review: {result:?}"); };
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("New Identity 中文"));
        assert!(config.contains("new@example.invalid"));
        assert!(config.contains("# keep this comment"));
        assert!(config.contains("setting = untouched"));
        assert!(!repo.join(".git/config.lock").exists());
        assert_eq!(std::fs::read(repo.join(".git/HEAD")).unwrap(), head);
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"two\n");
        assert!(matches!(app.run(Job::SaveIdentity { ticket, name: "Replay".into(), email: "replay@example.invalid".into() }), Reply::PromptFailed { action: PromptAction::IdentitySettings, .. }));
        assert!(matches!(app.run(Job::Reload), Reply::Notice(text) if text.contains("changed since trust")));
        assert!(matches!(app.run(Job::GrantRepositoryTrust(review.ticket)), Reply::RepositoryOpened(_)));
        let Reply::IdentitySettings { name, email, detail, .. } = app.run(Job::LoadIdentity) else { panic!("newly trusted identity should be readable"); };
        assert_eq!(name, "New Identity 中文");
        assert_eq!(email, "new@example.invalid");
        assert!(detail.contains("Resolved author:"));
    }

    #[test]
    fn entry_identity_config_refuses_stale_locked_and_failed_edits_without_partial_publication() {
        let root = Scratch::new("entry-identity-stale");
        let repo = repository(&root.0, "repo");
        let app = trusted_app(&root.0, &repo);
        let session = crate::open_trusted(&app.options, &repo).unwrap();
        let preview = srcmgr_git::IdentityConfig::read(&session.installation, &session.identity).unwrap();
        fixture_git(&repo, &["config", "custom.external", "keep external change"]);
        let external = std::fs::read(repo.join(".git/config")).unwrap();
        assert!(preview.save_local(&session.installation, &session.identity, "New", "new@example.invalid").is_err());
        assert_eq!(std::fs::read(repo.join(".git/config")).unwrap(), external);
        for lock_name in ["config.lock", "config.lock.lock"] {
            let preview = srcmgr_git::IdentityConfig::read(&session.installation, &session.identity).unwrap();
            let lock = repo.join(".git").join(lock_name);
            std::fs::write(&lock, b"external writer").unwrap();
            assert!(preview.save_local(&session.installation, &session.identity, "New", "new@example.invalid").is_err());
            assert_eq!(std::fs::read(repo.join(".git/config")).unwrap(), external);
            assert_eq!(std::fs::read(&lock).unwrap(), b"external writer");
            if lock_name != "config.lock" { assert!(!repo.join(".git/config.lock").exists()); }
            std::fs::remove_file(lock).unwrap();
        }
        let preview = srcmgr_git::IdentityConfig::read(&session.installation, &session.identity).unwrap();
        assert!(preview.save_local(&session.installation, &session.identity, "New", "bad\nemail").is_err());
        assert_eq!(std::fs::read(repo.join(".git/config")).unwrap(), external);
        assert!(!repo.join(".git/config.lock").exists());
    }

    #[test]
    fn entry_git_settings_are_passive_and_saved_choice_survives_a_new_session() {
        let root = Scratch::new("entry-git-settings");
        let mut app = app(&root.0);
        assert!(matches!(app.run(Job::LoadGitSettings), Reply::GitSettings { .. }), "showing a nonexistent configured Git must not probe it");
        let executable = std::fs::canonicalize(git_executable()).unwrap();
        assert!(matches!(app.run(Job::ConfigureGit(Some(NativePath::new(executable.as_os_str())))), Reply::GitSettings { detail, .. } if detail.starts_with("Saved and applied.")));
        let restarted = ApplicationBackend::welcome(Options { profile: app.options.profile.clone(), ..Options::default() });
        assert_eq!(crate::selected_git(&restarted.options).unwrap(), executable);
        assert!(matches!(app.run(Job::ConfigureGit(Some(NativePath::new(root.0.join("missing.exe").into_os_string())))), Reply::PromptFailed { action: PromptAction::GitSettings, .. }));
        assert_eq!(crate::selected_git(&restarted.options).unwrap(), executable);
        assert_eq!(app.options.git.as_deref(), Some(executable.as_path()));
        let override_options = Options { git: Some(root.0.join("launch-override")), profile: app.options.profile.clone(), ..Options::default() };
        assert_eq!(crate::selected_git(&override_options).unwrap(), root.0.join("launch-override"));
    }

    #[test]
    fn entry_git_settings_failed_persistence_cannot_apply_an_unsaved_selection() {
        let root = Scratch::new("entry-git-settings-lock");
        let mut app = app(&root.0);
        let previous = app.options.git.clone();
        let store = open_profile_store(&app.options).unwrap();
        let reply = app.run(Job::ConfigureGit(Some(NativePath::new(git_executable().into_os_string()))));
        assert!(matches!(reply, Reply::PromptFailed { action: PromptAction::GitSettings, text } if text.contains("not saved or applied")));
        assert_eq!(app.options.git, previous);
        assert!(store.git_executable().unwrap().is_none());
        drop(store);
    }

    fn creation(app: &mut ApplicationBackend, path: &std::path::Path, branch: &str) -> String {
        match app.run(Job::PrepareCreate { path: NativePath::new(path.as_os_str()), branch: branch.into() }) {
            Reply::CreateReview { ticket, .. } => ticket,
            other => panic!("expected filesystem-only creation preview: {other:?}"),
        }
    }

    #[test]
    fn entry_create_preview_cancel_expiry_and_invalid_branch_write_nothing() {
        let root = Scratch::new("entry-create-no-write");
        let target = root.0.join("new repo");
        let mut app = app(&root.0);
        let cancelled = creation(&mut app, &target, "main");
        assert!(!target.exists(), "preview must not create directories or run missing Git");
        app.run(Job::CancelEntry);
        assert!(matches!(app.run(Job::ExecuteCreate(cancelled)), Reply::PromptFailed { action: PromptAction::CreateRepository, .. }));
        let expired = creation(&mut app, &target, "main");
        app.create.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(matches!(app.run(Job::ExecuteCreate(expired)), Reply::PromptFailed { .. }));
        let missing_git = creation(&mut app, &target, "main");
        assert!(matches!(app.run(Job::ExecuteCreate(missing_git)), Reply::PromptFailed { .. }));
        app.options.git = Some(git_executable());
        for branch in ["bad..branch", "@{-1}", "refs/../escape"] {
            let ticket = creation(&mut app, &target, branch);
            assert!(matches!(app.run(Job::ExecuteCreate(ticket)), Reply::PromptFailed { .. }));
            assert!(!target.exists(), "invalid branch must be refused before writes");
        }
        assert!(!target.exists());
        assert!(!root.0.join("profile/trust.v1").exists());
    }

    #[test]
    fn entry_create_preserves_existing_files_and_requires_separate_trust() {
        let root = Scratch::new("entry-create-preserve");
        let mut app = app(&root.0);
        app.options.git = Some(git_executable());
        for (name, existing) in [("new repo 中文", false), ("existing project", true)] {
            let target = root.0.join(name);
            if existing {
                std::fs::create_dir(&target).unwrap();
                std::fs::write(target.join("keep.bin"), [0, 255, 13, 10]).unwrap();
                std::fs::create_dir(target.join("nested")).unwrap();
                std::fs::write(target.join("nested/keep.txt"), b"untouched").unwrap();
            }
            let ticket = creation(&mut app, &target, "feature/start");
            let Reply::TrustReview(review) = app.run(Job::ExecuteCreate(ticket.clone())) else { panic!("creation must end at separate trust review"); };
            assert_eq!(std::fs::read(target.join(".git/HEAD")).unwrap(), b"ref: refs/heads/feature/start\n");
            assert!(!target.join(".git/index").exists());
            assert!(!target.join(".git/hooks").exists(), "empty template must not copy hooks");
            assert!(!target.join(".git/refs/heads/feature/start").exists(), "no automatic commit");
            if existing {
                assert_eq!(std::fs::read(target.join("keep.bin")).unwrap(), [0, 255, 13, 10]);
                assert_eq!(std::fs::read(target.join("nested/keep.txt")).unwrap(), b"untouched");
            }
            assert!(matches!(app.run(Job::ExecuteCreate(ticket)), Reply::PromptFailed { .. }), "creation authority is one-use");
            let Reply::RepositoryOpened(snapshot) = app.run(Job::GrantRepositoryTrust(review.ticket)) else { panic!("explicit trust should open unborn repository"); };
            assert!(snapshot.commits.is_empty());
            assert_eq!(snapshot.files.len(), if existing { 2 } else { 0 });
        }
    }

    #[test]
    fn entry_create_refuses_existing_or_racing_metadata_and_replaced_directory() {
        let root = Scratch::new("entry-create-stale");
        let mut app = app(&root.0);
        app.options.git = Some(git_executable());
        let repo = repository(&root.0, "already-repo");
        let config = std::fs::read(repo.join(".git/config")).unwrap();
        assert!(matches!(app.run(Job::PrepareCreate { path: NativePath::new(repo.as_os_str()), branch: "main".into() }), Reply::PromptFailed { .. }));
        assert_eq!(std::fs::read(repo.join(".git/config")).unwrap(), config);
        let target = root.0.join("existing");
        std::fs::create_dir(&target).unwrap();
        let ticket = creation(&mut app, &target, "main");
        std::fs::write(target.join(".git"), b"external marker").unwrap();
        assert!(matches!(app.run(Job::ExecuteCreate(ticket)), Reply::PromptFailed { .. }));
        assert_eq!(std::fs::read(target.join(".git")).unwrap(), b"external marker");
        let raced = root.0.join("racing-init");
        std::fs::create_dir(&raced).unwrap();
        let ticket = creation(&mut app, &raced, "main");
        fixture_git(&raced, &["init", "-q", "-b", "external"]);
        let head = std::fs::read(raced.join(".git/HEAD")).unwrap();
        let config = std::fs::read(raced.join(".git/config")).unwrap();
        assert!(matches!(app.run(Job::ExecuteCreate(ticket)), Reply::PromptFailed { .. }), "must refuse concurrent initialization, never reinitialize");
        assert_eq!(std::fs::read(raced.join(".git/HEAD")).unwrap(), head);
        assert_eq!(std::fs::read(raced.join(".git/config")).unwrap(), config);
        let swapped = root.0.join("swapped");
        std::fs::create_dir(&swapped).unwrap();
        let ticket = creation(&mut app, &swapped, "main");
        std::fs::rename(&swapped, root.0.join("original")).unwrap();
        std::fs::create_dir(&swapped).unwrap();
        assert!(matches!(app.run(Job::ExecuteCreate(ticket)), Reply::PromptFailed { .. }));
        assert!(!swapped.join(".git").exists());
        assert!(app.active.is_none());
    }

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
