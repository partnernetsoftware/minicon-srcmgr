//! Repository-independent entry. Metadata browsing never invokes repository
//! Git; an explicit, one-use trust review precedes any untrusted repo command.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use srcmgr_domain::NativePath;
use srcmgr_state::{RecentRepository, TrustError, fingerprint};
use srcmgr_ui::{Backend, Job, PromptAction, RecentRow, Reply, TrustReview};

use crate::window_backend::WindowBackend;
use crate::{Options, canonical_worktree, git_dir_of, open_profile_store, trust_store};

fn recent_row(entry: RecentRepository) -> RecentRow {
    let label = entry.native_path().map_or_else(|_| format!("{} location; unavailable on this platform", entry.path.tag()), |path| path.display_label());
    RecentRow { backend: entry.backend, path: entry.path, label }
}

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

struct PendingNavigation {
    ticket: String,
    worktree: PathBuf,
    expected: String,
    expires: Instant,
}

struct PendingLocate {
    ticket: String,
    source: RecentRepository,
    target: PathBuf,
    git_directory: PathBuf,
    fingerprint: String,
    expires: Instant,
}

pub struct ApplicationBackend {
    locate: Option<PendingLocate>,
    tools: crate::tools::Tools,
    navigation: Option<PendingNavigation>,
    options: Options,
    active: Option<WindowBackend>,
    trust: Option<PendingTrust>,
    create: Option<PendingCreate>,
    identity: Option<PendingIdentity>,
}

impl ApplicationBackend {
    pub fn welcome(options: Options) -> Self {
        Self {
            locate: None,
            tools: Default::default(),
            navigation: None,
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
                .map(recent_row)
                .collect(),
        ))
    }

    fn prepare_locate(&mut self, source: RecentRepository, target: NativePath) -> Result<Reply, String> {
        if source.backend != "git" { return Err("This backend is unavailable; the recent entry was preserved".into()); }
        if !open_profile_store(&self.options)?.recent_repositories().map_err(|e| e.to_string())?.contains(&source) { return Err("Recent entry changed; reload the list before locating again".into()); }
        let input = std::path::Path::new(target.as_os_str());
        if !input.is_absolute() { return Err("Enter an absolute local repository path".into()); }
        // Filesystem metadata only. Neither reviewing nor saving a bookmark
        // starts Git, grants trust or publishes a new active worker target.
        let target = canonical_worktree(input)?;
        let git_directory = git_dir_of(&target)?;
        let digest = fingerprint(&git_directory, &target).map_err(|e| e.to_string())?;
        let random = minicon_platform::entropy::secure_random_array::<16>().map_err(|_| "Cannot prepare location review")?;
        let ticket: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let old = source.native_path().map_or_else(|_| format!("{} location; unavailable on this platform", source.path.tag()), |p| p.display_label());
        let lines = vec![format!("Previous recent location: {old}"), format!("New location: {}", NativePath::new(target.as_os_str()).display_label()), "Replace only this navigation record. An existing bookmark for the new location is deduplicated.".into(), "No files, Git metadata, drafts, saved views or trust grants are moved, merged or deleted.".into(), "This review does not prove that the two locations are the same repository. Old-path drafts remain retained separately.".into(), "Save returns to Recent repositories. Open explicitly afterward to check current Git state and review trust for the new location.".into(), "Changed or expired reviews require a new review. Enter does not approve; Cancel preserves the old record.".into()];
        self.locate = Some(PendingLocate { ticket: ticket.clone(), source, target, git_directory, fingerprint: digest, expires: Instant::now()+Duration::from_secs(60) });
        Ok(Reply::LocateReview { ticket, lines })
    }

    fn save_located(&mut self, ticket: &str) -> Result<Reply, String> {
        let pending = self.locate.take().ok_or("Location review is missing or consumed; review again")?;
        if pending.ticket != ticket || Instant::now() >= pending.expires { return Err("Location review expired or changed; review again".into()); }
        let canonical = canonical_worktree(&pending.target)?;
        let git_directory = git_dir_of(&canonical)?;
        if canonical != pending.target || git_directory != pending.git_directory || fingerprint(&git_directory, &canonical).map_err(|e| e.to_string())? != pending.fingerprint { return Err("Repository metadata changed; review the location again".into()); }
        let mut store = open_profile_store(&self.options)?;
        let entries = store.relocate_repository(&pending.source, &canonical).map_err(|e| format!("Recent entry changed or could not be saved; reload and review again: {e}"))?;
        let target = NativePath::new(canonical.into_os_string()).encoded();
        // Return the known metadata publication directly. A later read must
        // not turn an already saved location into an apparent failed write.
        let entries = entries.into_iter().map(recent_row).collect();
        Ok(Reply::RecentLocated { entries, target, detail: "Location saved. Open explicitly to review trust and load the repository. Old-path drafts, saved views and trust remain separate; files were not moved.".into() })
    }

    fn open(&mut self, path: NativePath) -> Result<Reply, String> {
        self.open_mode(path, false)
    }

    fn open_mode(&mut self, path: NativePath, force_review: bool) -> Result<Reply, String> {
        self.tools.cancel();
        self.navigation = None;
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
        let path = NativePath::new(worktree.as_os_str());
        let needs_repair = match open_profile_store(&self.options)?.navigation(&path) {
            Ok(Some(record)) => srcmgr_ui::Navigation::decode(&record).err().map(str::to_owned),
            Ok(None) => None,
            Err(srcmgr_state::StoreError::InvalidMetadata) => Some("Saved navigation is larger than its supported limit".into()),
            Err(error) => return Err(error.to_string()),
        };
        if let Some(reason) = needs_repair {
            let expected = open_profile_store(&self.options)?.navigation_for_repair(&path).map_err(|e| format!("Saved view cannot be safely reviewed (recovery limit 1 MiB). Preserve the profile database for manual recovery: {e}"))?.ok_or("Saved view changed; retry opening")?;
            if srcmgr_ui::Navigation::decode(&expected).is_ok() { return Err("Saved view changed; retry opening to load the current record".into()); }
            let random = minicon_platform::entropy::secure_random_array::<16>().map_err(|e| format!("Cannot prepare navigation recovery: {e:?}"))?;
            let ticket: String = random.iter().map(|b| format!("{b:02x}")).collect();
            self.navigation = Some(PendingNavigation { ticket: ticket.clone(), worktree, expected, expires: Instant::now() + Duration::from_secs(60) });
            return Ok(Reply::NavigationIssue(srcmgr_ui::NavigationIssue { ticket, path, reason, submitted: false }));
        }
        let (candidate, snapshot) = WindowBackend::open_target(&self.options, &worktree)?;
        self.active = Some(candidate);
        Ok(Reply::RepositoryOpened(snapshot))
    }

    fn reset_navigation(&mut self, ticket: &str) -> Result<Reply, String> {
        let pending = self.navigation.take().ok_or("Navigation recovery is missing or consumed; open again to review")?;
        if pending.ticket != ticket || Instant::now() >= pending.expires { return Err("Navigation recovery changed or expired; review again".into()); }
        let path = NativePath::new(pending.worktree.into_os_string());
        let archive = open_profile_store(&self.options)?.archive_navigation(&path, &pending.expected, &pending.ticket).map_err(|e| format!("Saved view changed or could not be archived; no reset was published. Reopen to review: {e}"))?;
        let detail = format!("Saved view archived as {archive}. Defaults were selected; drafts and Git data were retained.");
        let next = self.open(path).unwrap_or_else(|e| Reply::PromptFailed { action: PromptAction::OpenRepository, text: format!("Saved view archived. Repository was not opened: {e}. The archive remains in this profile; retry Open explicitly.") });
        Ok(Reply::NavigationReset { archive, detail, next: Box::new(next) })
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
        if !matches!(&job, Job::SaveLocated(_) | Job::CancelLocate) { self.locate = None; }
        match job {
            Job::PrepareLocate { backend, source, target } => self.prepare_locate(RecentRepository { backend, path: source }, target).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::LocateRepository, text }),
            Job::SaveLocated(ticket) => self.save_located(&ticket).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::LocateRepository, text }),
            Job::CancelLocate => { self.locate = None; Reply::NoChange }
            Job::LoadToolSettings(kind) => self.tools.load(&self.options, kind).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::ExternalTool, text }),
            Job::SaveToolSettings { kind, definition } => self.tools.save(&self.options, self.active.as_ref().map(|a| a.worktree.as_path()), kind, definition).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::ExternalTool, text }),
            Job::PrepareTool { kind, file } => self.active.as_ref().ok_or_else(|| "Open a repository before tool handoff".into()).and_then(|a| self.tools.prepare(&self.options, &a.worktree, kind, file)).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::ExternalTool, text }),
            Job::LaunchTool(ticket) => self.active.as_ref().ok_or_else(|| "Open a repository before tool handoff".into()).and_then(|a| self.tools.launch(&self.options, &a.worktree, &ticket)).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::ExternalTool, text }),
            Job::CancelTool => { self.tools.cancel(); Reply::NoChange }
            Job::ResetNavigation(ticket) => self.reset_navigation(&ticket).unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::OpenRepository, text }),
            Job::EnterRepository { path, current } => (|| {
                if let Some((source, navigation)) = current {
                    let active = self.active.as_ref().ok_or("No active repository for navigation save")?;
                    if std::path::Path::new(source.as_os_str()) != active.worktree { return Err("Navigation belongs to another repository; target unchanged".into()); }
                    open_profile_store(&self.options)?.put_navigation(&source, &navigation.encode()).map_err(|e| e.to_string())?;
                } else if self.active.is_some() { return Err("Active repository context is required before entry".into()); }
                if let Some(path) = path { self.open(path) }
                else {
                    let Reply::Welcome(recents) = self.recent()? else { return Err("Recent list unavailable".into()); };
                    self.active = None;
                    self.tools.cancel();
                    self.navigation = None;
                    self.trust = None;
                    self.create = None;
                    self.identity = None;
                    Ok(Reply::RepositoryClosed(recents))
                }
            })().unwrap_or_else(|text| Reply::PromptFailed { action: PromptAction::OpenRepository, text }),
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
                self.navigation = None;
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
    fn entry_locate_repairs_only_the_reviewed_bookmark_and_requires_separate_open_trust() {
        let root = Scratch::new("locate-recent");
        let old = repository(&root.0,"old");
        let stable = repository(&root.0,"stable");
        let _ = trusted_app(&root.0,&old);
        let mut app = trusted_app(&root.0,&stable);
        let options = app.options.clone();
        let old_native = NativePath::new(std::fs::canonicalize(&old).unwrap().into_os_string());
        let old_key = crate::repository_key(&old).unwrap();
        let source = open_profile_store(&options).unwrap().recent_repositories().unwrap().into_iter().find(|e| e.path == old_native.encoded()).unwrap();
        let moved = root.0.join("moved 中文");
        let canonical_root = std::fs::canonicalize(&root.0).unwrap();
        assert!(std::fs::canonicalize(&old).unwrap().starts_with(&canonical_root));
        assert_eq!(moved.parent(),Some(root.0.as_path())); assert!(!moved.exists());
        std::fs::rename(&old,&moved).unwrap();
        let target = NativePath::new(std::fs::canonicalize(&moved).unwrap().into_os_string());
        let new_key = crate::repository_key(&moved).unwrap();
        {
            let mut store = open_profile_store(&options).unwrap();
            store.put_draft("git",&old_key,"old retained draft").unwrap();
            store.put_draft("git",&new_key,"independent new draft").unwrap();
            store.put_navigation(&old_native,"old view retained").unwrap();
        }
        let (trust_path,_) = trust_store(&options).unwrap();
        let trust_before = std::fs::read(&trust_path).unwrap();
        let data = |repo: &std::path::Path| [std::fs::read(repo.join(".git/HEAD")).unwrap(),std::fs::read(repo.join(".git/index")).unwrap(),std::fs::read(repo.join("a.txt")).unwrap(),std::fs::read(repo.join("u.txt")).unwrap()];
        let before = [data(&stable),data(&moved)];
        let recents = || open_profile_store(&options).unwrap().get_setting("workspace.recents.v1").unwrap();
        let original_recents = recents();
        // Missing Git is a discriminating prerequisite: metadata review/save
        // must still work, and must never retarget the active backend.
        app.options.git = Some(root.0.join("not-installed-git"));
        let prepare = |app: &mut ApplicationBackend| {
            let Reply::LocateReview { ticket,.. } = app.run(Job::PrepareLocate { backend: source.backend.clone(), source: source.path.clone(), target: target.clone() }) else { panic!("metadata-only review unavailable") }; ticket
        };
        let cancelled = prepare(&mut app); app.run(Job::CancelLocate);
        assert!(matches!(app.run(Job::SaveLocated(cancelled)),Reply::PromptFailed {..}));
        assert_eq!(recents(),original_recents);
        let expired = prepare(&mut app); app.locate.as_mut().unwrap().expires = Instant::now()-Duration::from_secs(1);
        assert!(matches!(app.run(Job::SaveLocated(expired)),Reply::PromptFailed {..})); assert_eq!(recents(),original_recents);
        let changed = prepare(&mut app);
        let config_path = moved.join(".git/config"); let config = std::fs::read(&config_path).unwrap();
        let mut modified = config.clone(); modified.extend_from_slice(b"\n# external edit\n"); std::fs::write(&config_path,modified).unwrap();
        let refusal = app.run(Job::SaveLocated(changed));
        assert_eq!(recents(),original_recents,"changed repository metadata must not publish a new bookmark");
        assert!(matches!(refusal,Reply::PromptFailed {..})); std::fs::write(&config_path,config).unwrap();
        let stale = prepare(&mut app); open_profile_store(&options).unwrap().forget_repository(&source).unwrap();
        let removed = recents(); let refusal = app.run(Job::SaveLocated(stale)); assert_eq!(recents(),removed); assert!(matches!(refusal,Reply::PromptFailed {..}));
        open_profile_store(&options).unwrap().remember_repository("git",std::path::Path::new(old_native.as_os_str())).unwrap();
        let locked = prepare(&mut app);
        {
            let store = open_profile_store(&options).unwrap();
            assert!(matches!(app.run(Job::SaveLocated(locked)),Reply::PromptFailed {..}));
            assert!(store.recent_repositories().unwrap().contains(&source));
        }
        let ticket = prepare(&mut app);
        let Reply::RecentLocated { entries,target: returned,.. } = app.run(Job::SaveLocated(ticket.clone())) else { panic!("location was not saved") };
        assert_eq!(returned,target.encoded()); assert!(entries.iter().any(|e| e.path==returned)); assert!(!entries.iter().any(|e| e.path==source.path));
        assert_eq!(app.active.as_ref().unwrap().worktree,std::fs::canonicalize(&stable).unwrap());
        assert_eq!(std::fs::read(&trust_path).unwrap(),trust_before);
        assert_eq!([data(&stable),data(&moved)],before);
        assert!(matches!(app.run(Job::SaveLocated(ticket)),Reply::PromptFailed {..}));
        let Reply::TrustReview(review) = app.run(Job::OpenRepository(target.clone())) else { panic!("new location must not inherit old trust") };
        app.run(Job::CancelEntry);
        assert_eq!(std::fs::read(&trust_path).unwrap(),trust_before);
        assert!(matches!(app.run(Job::GrantRepositoryTrust(review.ticket)),Reply::PromptFailed {..}));
        app.options.git = options.git.clone();
        let Reply::TrustReview(review) = app.run(Job::OpenRepository(target.clone())) else { panic!() };
        let Reply::RepositoryOpened(snapshot) = app.run(Job::GrantRepositoryTrust(review.ticket)) else { panic!() };
        assert_eq!(snapshot.draft.as_deref(),Some("independent new draft"));
        let store = open_profile_store(&options).unwrap();
        assert_eq!(store.get_draft("git",&old_key).unwrap().as_deref(),Some("old retained draft"));
        assert_eq!(store.navigation(&old_native).unwrap().as_deref(),Some("old view retained"));
        assert_eq!([data(&stable),data(&moved)],before);
    }

    #[test]
    fn entry_navigation_recovery_cancel_stale_approval_and_known_archive_outcomes() {
        use srcmgr_ui::Workspace;
        for fail_after_archive in [false, true] {
            let root = Scratch::new(if fail_after_archive { "navigation-after-archive" } else { "navigation-archive" });
            let left = repository(&root.0, "left");
            let right = repository(&root.0, "right 中文");
            let mut app = trusted_app(&root.0, &left);
            let _ = trusted_app(&root.0, &right);
            let path = NativePath::new(std::fs::canonicalize(&right).unwrap().into_os_string());
            let options = app.options.clone();
            let navigation = Workspace::new(app.active.as_ref().unwrap().snapshot().unwrap()).navigation();
            let source = NativePath::new(app.active.as_ref().unwrap().worktree.as_os_str());
            let open = || Job::EnterRepository { path: Some(path.clone()), current: Some((source.clone(), Box::new(navigation.clone()))) };
            let before = std::fs::read(right.join(".git/HEAD")).unwrap();
            let index = std::fs::read(right.join(".git/index")).unwrap();
            {
                let mut store = crate::open_profile_store(&options).unwrap();
                store.put_navigation(&path, "unknown-view-1").unwrap();
                store.put_draft("git", &crate::repository_key(&right).unwrap(), "retained right draft").unwrap();
            }
            let Reply::NavigationIssue(issue) = app.run(open()) else { panic!("invalid view must offer bounded recovery"); };
            app.run(Job::CancelEntry);
            assert!(matches!(app.run(Job::ResetNavigation(issue.ticket)), Reply::PromptFailed { .. }));
            assert_eq!(crate::open_profile_store(&options).unwrap().navigation(&path).unwrap().as_deref(), Some("unknown-view-1"));
            let Reply::NavigationIssue(issue) = app.run(open()) else { panic!() };
            crate::open_profile_store(&options).unwrap().put_navigation(&path, "unknown-view-2").unwrap();
            let stale = app.run(Job::ResetNavigation(issue.ticket));
            assert_eq!(crate::open_profile_store(&options).unwrap().navigation(&path).unwrap().as_deref(), Some("unknown-view-2"));
            assert!(matches!(stale, Reply::PromptFailed { .. }));
            assert_eq!(app.active.as_ref().unwrap().worktree, std::fs::canonicalize(&left).unwrap());
            let Reply::NavigationIssue(issue) = app.run(open()) else { panic!() };
            if fail_after_archive { app.options.git = Some(root.0.join("missing-git")); }
            let Reply::NavigationReset { archive, next, .. } = app.run(Job::ResetNavigation(issue.ticket.clone())) else { panic!("known metadata publication must be acknowledged"); };
            if fail_after_archive {
                assert!(matches!(*next, Reply::PromptFailed { action: PromptAction::OpenRepository, ref text } if text.contains("Saved view archived") && text.contains("not opened")));
                assert_eq!(app.active.as_ref().unwrap().worktree, std::fs::canonicalize(&left).unwrap());
            } else {
                let Reply::RepositoryOpened(snapshot) = *next else { panic!("default view must open"); };
                assert!(snapshot.navigation.is_none());
                assert_eq!(snapshot.draft.as_deref(), Some("retained right draft"));
            }
            let store = crate::open_profile_store(&options).unwrap();
            assert_eq!(store.get_setting(&archive).unwrap().as_deref(), Some("unknown-view-2"));
            assert!(store.navigation(&path).unwrap().is_none());
            assert_eq!(store.get_draft("git", &crate::repository_key(&right).unwrap()).unwrap().as_deref(), Some("retained right draft"));
            assert_eq!(std::fs::read(right.join(".git/HEAD")).unwrap(), before);
            assert_eq!(std::fs::read(right.join(".git/index")).unwrap(), index);
            assert_eq!(std::fs::read(right.join("a.txt")).unwrap(), b"two\n");
            drop(store);
            assert!(matches!(app.run(Job::ResetNavigation(issue.ticket)), Reply::PromptFailed { .. }), "one-use approval cannot archive a later record");
        }
    }

    #[test]
    fn entry_history_return_survives_repository_departure_and_a_new_backend() {
        use srcmgr_ui::{Workspace, Pane, Key, Effect};
        fn drive(ui: &mut Workspace, app: &mut ApplicationBackend, mut effect: Effect) {
            for _ in 0..16 { match effect { Effect::Nothing => return, Effect::Send(job) => effect = ui.reply(app.run(job)), _ => panic!("unexpected effect") } }
            panic!("work did not settle");
        }
        fn git(repo: &std::path::Path, args: &[&str]) -> Vec<u8> {
            let output = std::process::Command::new(git_executable()).current_dir(repo).args(args).output().unwrap();
            assert!(output.status.success()); output.stdout
        }
        let root = Scratch::new("history-return");
        let left = repository(&root.0, "left");
        let right = repository(&root.0, "right");
        for i in 0..20 { std::fs::write(left.join(format!("f{i:02} 中文.txt")), (0..80).map(|n| format!("line {n}\n")).collect::<String>()).unwrap(); }
        fixture_git(&left, &["add", "."]);
        let message = format!("history target\n\n{}", (0..80).map(|n| format!("body {n}\n")).collect::<String>());
        fixture_git(&left, &["commit", "-qm", &message]);
        std::fs::write(left.join("a.txt"), b"uncommitted survives\n").unwrap();
        let mut app = trusted_app(&root.0, &left);
        let _ = trusted_app(&root.0, &right);
        let options = app.options.clone();
        let before = [git(&left, &["rev-parse", "HEAD"]), git(&left, &["write-tree"]), git(&right, &["rev-parse", "HEAD"]), git(&right, &["write-tree"])];
        let snapshot = app.active.as_ref().unwrap().snapshot().unwrap();
        let left_path = snapshot.repository.clone().unwrap();
        let mut ui = Workspace::new(snapshot);
        ui.set_view_rows(6);
        ui.request_filter(Pane::Commits); ui.key(Key::Text("history target".into()));
        let effect = ui.key(Key::Enter); drive(&mut ui, &mut app, effect);
        ui.click_main(Some(35)); ui.key(Key::Escape);
        let summary_position = (ui.cursor(), ui.scroll());
        let effect = ui.key(Key::Enter); drive(&mut ui, &mut app, effect);
        let selected = ui.main().committed.as_ref().unwrap().files.iter().position(|f| f.path.as_os_str() == "f19 中文.txt").unwrap() + 2;
        ui.click_main(Some(selected));
        let file_position = (ui.cursor(), ui.scroll());
        let effect = ui.key(Key::Enter); drive(&mut ui, &mut app, effect);
        ui.click_main(Some(30));
        let navigation = ui.navigation();
        assert_eq!(navigation.back.len(), 2);
        let Reply::RepositoryOpened(snapshot) = app.run(Job::EnterRepository { path: Some(NativePath::new(right.as_os_str())), current: Some((left_path.clone(), Box::new(navigation))) }) else { panic!() };
        let other = Workspace::new(snapshot).navigation();
        app.run(Job::EnterRepository { path: None, current: Some((NativePath::new(std::fs::canonicalize(&right).unwrap().into_os_string()), Box::new(other))) });
        let mut restarted = ApplicationBackend::welcome(options);
        let Reply::RepositoryOpened(snapshot) = restarted.run(Job::OpenRepository(left_path)) else { panic!() };
        let mut resumed = Workspace::new(snapshot); resumed.set_view_rows(6);
        let effect = resumed.open(); drive(&mut resumed, &mut restarted, effect);
        assert!(resumed.main().file.is_none(), "committed Diff remains read-only after restart");
        let effect = resumed.key(Key::Escape); drive(&mut resumed, &mut restarted, effect);
        assert_eq!((resumed.cursor(), resumed.scroll()), file_position);
        assert_eq!(resumed.main().committed.as_ref().unwrap().files[resumed.cursor()-2].path.as_os_str(), "f19 中文.txt");
        let effect = resumed.key(Key::Escape); drive(&mut resumed, &mut restarted, effect);
        assert_eq!((resumed.cursor(), resumed.scroll()), summary_position);
        assert_eq!(resumed.filter_query(Pane::Commits), "history target");
        assert_eq!(before, [git(&left, &["rev-parse", "HEAD"]), git(&left, &["write-tree"]), git(&right, &["rev-parse", "HEAD"]), git(&right, &["write-tree"])]);
        assert_eq!(std::fs::read(left.join("a.txt")).unwrap(), b"uncommitted survives\n");
    }

    #[test]
    fn entry_repository_tabs_persist_context_and_never_retarget_a_write_on_refusal() {
        use srcmgr_ui::{Workspace, Pane, Key};
        let root = Scratch::new("entry-tabs");
        let left = repository(&root.0, "left");
        let right = repository(&root.0, "right 中文");
        let mut app = trusted_app(&root.0, &left);
        let _ = trusted_app(&root.0, &right);
        let left_path = NativePath::new(std::fs::canonicalize(&left).unwrap().into_os_string());
        let right_path = NativePath::new(std::fs::canonicalize(&right).unwrap().into_os_string());
        let snapshot = app.active.as_ref().unwrap().snapshot().unwrap();
        let mut workspace = Workspace::new(snapshot);
        workspace.click_row(Pane::Files, 1);
        let navigation = workspace.navigation();
        let options = app.options.clone();
        crate::open_profile_store(&options).unwrap().put_draft("git", &crate::repository_key(&left).unwrap(), "left draft").unwrap();
        let Reply::RepositoryOpened(opened) = app.run(Job::EnterRepository { path: Some(right_path.clone()), current: Some((left_path.clone(), Box::new(navigation.clone()))) }) else { panic!("right must open"); };
        assert_eq!(opened.repository, Some(right_path.clone()));
        assert!(matches!(app.run(Job::SetFile { path: NativePath::new("a.txt"), stage: true }), Reply::Full(_)));
        let show = |p: &std::path::Path| std::process::Command::new(git_executable()).current_dir(p).args(["show", ":a.txt"]).output().unwrap().stdout;
        assert_eq!(show(&left), b"one\n");
        assert_eq!(show(&right), b"two\n");
        assert!(matches!(app.run(Job::EnterRepository { path: Some(left_path.clone()), current: Some((left_path.clone(), Box::new(navigation.clone()))) }), Reply::PromptFailed { .. }), "forged old source cannot save or activate");
        assert_eq!(app.active.as_ref().unwrap().worktree, std::fs::canonicalize(&right).unwrap());
        let mut other = Workspace::new(opened);
        other.key(Key::Char('3'));
        let other_navigation = other.navigation();
        assert!(matches!(app.run(Job::EnterRepository { path: Some(NativePath::new(root.0.join("missing").into_os_string())), current: Some((right_path.clone(), Box::new(other_navigation.clone()))) }), Reply::PromptFailed { .. }));
        assert_eq!(app.active.as_ref().unwrap().worktree, std::fs::canonicalize(&right).unwrap());
        assert!(matches!(app.run(Job::EnterRepository { path: None, current: Some((right_path.clone(), Box::new(other_navigation.clone()))) }), Reply::RepositoryClosed(_)));
        assert!(app.active.is_none());
        let mut restarted = ApplicationBackend::welcome(options.clone());
        let Reply::RepositoryOpened(restored) = restarted.run(Job::EnterRepository { path: Some(left_path), current: None }) else { panic!("left must reopen"); };
        assert_eq!(restored.navigation, Some(navigation.clone()));
        assert_eq!(restored.draft.as_deref(), Some("left draft"));
        assert_eq!(crate::open_profile_store(&options).unwrap().navigation(&right_path).unwrap(), Some(other_navigation.encode()));
        // A storage refusal preserves the active backend, not only the label.
        let lock = root.0.join("profile/profile.lock");
        std::fs::write(&lock, std::process::id().to_string()).unwrap();
        let result = restarted.run(Job::EnterRepository { path: Some(right_path), current: restored.repository.map(|p| (p, Box::new(navigation))) });
        std::fs::remove_file(lock).unwrap();
        assert!(matches!(result, Reply::PromptFailed { .. }));
        assert_eq!(show(&left), b"one\n");
        assert_eq!(restarted.active.as_ref().unwrap().worktree, std::fs::canonicalize(&left).unwrap());
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
