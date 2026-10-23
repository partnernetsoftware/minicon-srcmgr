//! Scoped identity inspection and one-publication local config edits. Git
//! interprets/updates configuration syntax; SrcMgr binds the reviewed bytes.
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use minicon_platform::filesystem::{self, FileIdentity, metadata_is_link_like, path_identity};
use crate::runner::{self, Limits};
use crate::{GitError, GitInstallation, RepositoryIdentity};

const MAX_CONFIG: u64 = 1024 * 1024;

#[derive(Debug)]
pub struct IdentityConfig {
    path: PathBuf,
    original: Vec<u8>,
    identity: FileIdentity,
    pub name: Option<IdentityValue>,
    pub email: Option<IdentityValue>,
    pub author: Option<String>,
    pub committer: Option<String>,
}

#[derive(Debug)]
pub struct IdentityValue {
    pub value: String,
    pub scope: String,
    pub origin: String,
}

impl IdentityConfig {
    /// Caller must have verified repository execution trust first.
    pub fn read(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<Self, GitError> {
        git.require_supported()?;
        let path = repo.common_dir.join("config");
        let original = plain_config(&path)?;
        let identity = path_identity(&path).map_err(GitError::Io)?;
        let name = config_value(git, repo, "user.name")?;
        let email = config_value(git, repo, "user.email")?;
        let author = resolved_ident(git, repo, "GIT_AUTHOR_IDENT")?;
        let committer = resolved_ident(git, repo, "GIT_COMMITTER_IDENT")?;
        if plain_config(&path)? != original || !identity.same_object(path_identity(&path).map_err(GitError::Io)?) { return Err(GitError::IdentityConfigChanged); }
        Ok(Self { path, original, identity, name, email, author, committer })
    }

    /// Returns true when published but parent durability could not be
    /// acknowledged. Such a result is a completed write, never a refusal.
    pub fn save_local(self, git: &GitInstallation, repo: &RepositoryIdentity, name: &str, email: &str) -> Result<bool, GitError> {
        git.require_supported()?;
        validate_value(name, false)?;
        validate_value(email, true)?;
        if self.path != repo.common_dir.join("config") || plain_config(&self.path)? != self.original || !self.identity.same_object(path_identity(&self.path).map_err(GitError::Io)?) { return Err(GitError::IdentityConfigChanged); }
        let lock_path = repo.common_dir.join("config.lock");
        let mut file = filesystem::private_create_new_options().open(&lock_path).map_err(GitError::Io)?;
        let mut lock = ConfigLock { path: lock_path, identity: filesystem::file_identity(&file).map_err(GitError::Io)?, armed: true };
        file.write_all(&self.original).map_err(GitError::Io)?;
        file.sync_all().map_err(GitError::Io)?;
        drop(file);
        // The public config remains unchanged while both values are edited in
        // the owned lock. Cooperative Git config writers refuse config.lock.
        for (key, value) in [("user.name", name), ("user.email", email)] {
            // Git for Windows config locking rejects some verbatim absolute
            // paths. Bind cwd to the already-identified metadata directory and
            // use the fixed literal lock name, never a reconstructed label.
            let arguments = ["config", "--file", "config.lock", "--no-includes", "--replace-all", key, value];
            runner::run(git.executable(), Some(&repo.common_dir), &arguments, None, Limits::METADATA)?;
            lock.identity = path_identity(&lock.path).map_err(GitError::Io)?;
        }
        if plain_config(&self.path)? != self.original || !self.identity.same_object(path_identity(&self.path).map_err(GitError::Io)?) { return Err(GitError::IdentityConfigChanged); }
        fs::OpenOptions::new().read(true).write(true).open(&lock.path).and_then(|file| file.sync_all()).map_err(GitError::Io)?;
        filesystem::replace_file(&lock.path, &self.path).map_err(GitError::Io)?;
        lock.armed = false;
        Ok(filesystem::sync_parent(&repo.common_dir).is_err())
    }
}

struct ConfigLock { path: PathBuf, identity: FileIdentity, armed: bool }
impl Drop for ConfigLock {
    fn drop(&mut self) {
        if self.armed && path_identity(&self.path).is_ok_and(|identity| identity.same_object(self.identity)) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn validate_value(value: &str, email: bool) -> Result<(), GitError> {
    if value.is_empty() || value.len() > 1024 || value.trim() != value || value.chars().any(|ch| ch.is_control() || matches!(ch, '<' | '>') || (email && ch.is_whitespace())) {
        return Err(GitError::InvalidIdentity);
    }
    Ok(())
}

fn plain_config(path: &Path) -> Result<Vec<u8>, GitError> {
    let metadata = fs::symlink_metadata(path).map_err(GitError::Io)?;
    if !metadata.is_file() || metadata_is_link_like(&metadata) || metadata.len() > MAX_CONFIG { return Err(GitError::IdentityConfigChanged); }
    let mut bytes = Vec::new();
    File::open(path).map_err(GitError::Io)?.take(MAX_CONFIG + 1).read_to_end(&mut bytes).map_err(GitError::Io)?;
    if bytes.len() as u64 > MAX_CONFIG { return Err(GitError::IdentityConfigChanged); }
    Ok(bytes)
}

fn config_value(git: &GitInstallation, repo: &RepositoryIdentity, key: &str) -> Result<Option<IdentityValue>, GitError> {
    let output = match runner::run(git.executable(), Some(&repo.worktree_root), &["config", "--null", "--show-scope", "--show-origin", "--get", key], None, Limits::METADATA) {
        Ok(output) => output,
        Err(GitError::Exit { code: 1, .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let fields: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    if fields.len() != 4 || !fields[3].is_empty() { return Err(GitError::MalformedOutput { record: 0 }); }
    let text = |bytes: &[u8]| std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| GitError::UnrepresentableName);
    Ok(Some(IdentityValue { scope: text(fields[0])?, origin: text(fields[1])?, value: text(fields[2])? }))
}

fn resolved_ident(git: &GitInstallation, repo: &RepositoryIdentity, key: &str) -> Result<Option<String>, GitError> {
    let output = match runner::run(git.executable(), Some(&repo.worktree_root), &["var", key], None, Limits::METADATA) {
        Ok(output) => output,
        Err(GitError::Exit { code: 128, .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let text = std::str::from_utf8(&output.stdout).map_err(|_| GitError::UnrepresentableName)?;
    let end = text.rfind('>').ok_or(GitError::MalformedOutput { record: 0 })?;
    Ok(Some(text[..=end].to_owned()))
}
