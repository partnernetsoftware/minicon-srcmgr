//! Bounded local confirmation records; never a general backup framework.
use std::fs;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use srcmgr_domain::NativePath;
use srcmgr_ui::{Job, Reply};

use crate::{Options, Session, open_trusted, profile_root};

const MAX_COPY: u64 = 16 * 1024 * 1024;
const RECOVERY_BUDGET: u64 = 1024 * 1024 * 1024;

fn recovery_usage(root: &Path) -> Result<u64, String> {
    fn visit(path: &Path, depth: usize, entries: &mut usize, used: &mut u64) -> Result<(), String> {
        *entries += 1;
        if depth > 32 || *entries > 100_000 {
            return Err("recovery storage accounting exceeds its bound; inspect storage before discarding".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("recovery storage contains a link; inspect storage before discarding".into());
        }
        if metadata.is_file() {
            *used = used.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                visit(&entry.map_err(|e| e.to_string())?.path(), depth + 1, entries, used)?;
                if *used > RECOVERY_BUDGET { break; }
            }
        } else {
            return Err("unsupported recovery storage entry; inspect storage before discarding".into());
        }
        Ok(())
    }
    let (mut entries, mut used) = (0, 0);
    visit(root, 0, &mut entries, &mut used)?;
    Ok(used)
}

pub struct Prepared {
    ignore: Option<srcmgr_git::IgnoreEdit>,
    pub ticket: String,
    pub action: Job,
    expires: Instant,
    state: Vec<u8>,
    file: Option<(NativePath, Option<Vec<u8>>)>,
    retained_oid: Option<String>,
    guard_worktree: bool,
}

pub fn is_risky(job: &Job) -> bool {
    matches!(
        job,
        Job::Discard { .. }
            | Job::Ignore { .. }
            | Job::DiscardHunk { .. }
            | Job::DeleteBranch(_)
            | Job::Detach(_)
            | Job::StashAction { .. }
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A conservative freshness guard. A harmless index rewrite may invalidate a
/// preview; it must never cause a broader action to run.
fn state(session: &Session, guard_worktree: bool) -> Result<Vec<u8>, String> {
    let snapshot = srcmgr_git::read_status(
        &session.installation,
        &session.identity,
        srcmgr_domain::Generation(1),
    )
    .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(format!("{:?}", snapshot.head));
    hash.update(format!("{:?}", snapshot.entries));
    if guard_worktree {
        let mut bytes_read = 0_usize;
        for entry in &snapshot.entries {
            if matches!(
                entry.unstaged,
                srcmgr_domain::ChangeKind::Untracked | srcmgr_domain::ChangeKind::Ignored
            ) {
                continue;
            }
            if let Some(bytes) = content(&session.identity.worktree_root, &entry.path)? {
                bytes_read += bytes.len();
                if bytes_read > 64 * 1024 * 1024 {
                    return Err("working-copy guard exceeds 64 MiB; action refused".to_owned());
                }
                hash.update(bytes);
            } else {
                hash.update(b"deleted");
            }
        }
    }
    hash.update(session.identity.id.value().as_bytes());
    let index = session.identity.common_dir.join("index");
    match fs::metadata(&index) {
        Ok(meta) if meta.len() <= MAX_COPY => {
            hash.update(fs::read(index).map_err(|e| e.to_string())?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update(b"no-index"),
        _ => return Err("index is unavailable or exceeds the confirmation bound".to_owned()),
    }
    Ok(hash.finalize().to_vec())
}

fn content(root: &Path, path: &NativePath) -> Result<Option<Vec<u8>>, String> {
    let relative = Path::new(path.as_os_str());
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || relative.as_os_str().is_empty()
    {
        return Err("unsafe recovery path".to_owned());
    }
    let mut target = root.to_path_buf();
    for component in relative.components() {
        target.push(component);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("link recovery is not available in this slice".to_owned());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && target == root.join(relative) => {
                return Ok(None);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let canonical = fs::canonicalize(&target).map_err(|e| e.to_string())?;
    let meta = fs::metadata(&target).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) || !meta.is_file() || meta.len() > MAX_COPY {
        return Err("recovery requires a regular in-repository file of at most 16 MiB".to_owned());
    }
    fs::read(&target).map(Some).map_err(|e| e.to_string())
}

impl Prepared {
    pub fn create(
        options: &Options,
        worktree: &Path,
        generation: u64,
        action: Job,
    ) -> Result<(Self, Reply), String> {
        if !is_risky(&action) {
            return Err("this action does not use a destructive confirmation".to_owned());
        }
        let session = open_trusted(options, worktree)?;
        let ignore=if let Job::Ignore {path,local}=&action {Some(srcmgr_git::IgnoreEdit::prepare(&session.installation,&session.identity,path,*local).map_err(|e|e.to_string())?)}else{None};
        if let Job::StashAction { id, expected, .. } = &action {
            srcmgr_git::check_stash_selection(
                &session.installation,
                &session.identity,
                id,
                expected,
            )
            .map_err(|e| e.to_string())?;
        }
        let ticket = hex(&minicon_platform::entropy::secure_random_array::<16>()
            .map_err(|e| format!("cannot create confirmation: {e:?}"))?);
        let file = match &action {
            Job::Discard {
                path,
                untracked: true,
            } => {
                // No reliable trash facade has been qualified yet. Refuse
                // instead of silently selecting permanent deletion.
                return Err(format!(
                    "{}: trash is unavailable; untracked deletion is disabled",
                    path.display_label()
                ));
            }
            Job::Discard { path, .. } | Job::DiscardHunk { path, .. } => Some((
                path.clone(),
                content(&session.identity.worktree_root, path)?,
            )),
            _ => None,
        };
        let retained_oid = match &action {
            Job::DeleteBranch(name) => Some(
                srcmgr_git::reference_target(
                    &session.installation,
                    &session.identity,
                    &format!("refs/heads/{name}"),
                )
                .map_err(|e| e.to_string())?,
            ),
            Job::StashAction { id, operation, .. } if operation == "drop" || operation == "pop" => {
                Some(id.clone())
            }
            _ => None,
        };
        let recovery = if ignore.is_some() {"Ignore edits only the named rules file; it does not untrack, stage, commit or delete the selected content.".into()} else if let Some((path, bytes)) = &file {
            format!(
                "Recovery: safety copy required for {} ({} bytes); created only after y. A save failure blocks discard. Restore manually from the named local record; index is kept.",
                path.display_label(),
                bytes.as_ref().map_or(0, Vec::len)
            )
        } else if matches!(action, Job::DeleteBranch(_)) {
            "Recovery: old branch OID will be retained in a local recovery ref before deletion; no worktree backup.".to_owned()
        } else if let Job::StashAction { operation, .. } = &action {
            if operation == "drop" {
                "Recovery ref required before removing the selected stash. It retains the stash commit; no automatic undo. Drop leaves worktree and index unchanged. Partial ref/reflog publication requires manual inspection of the retained recovery transaction.".to_owned()
            } else if operation == "pop" {
                "Recovery ref required before removing the selected stash. It retains the stash commit, not preexisting worktree edits; no automatic undo. Apply may leave conflicts.".to_owned()
            } else {
                "Recovery with limits: source stash is kept on conflict/unknown outcome; existing worktree edits are not backed up. Apply can leave conflicts.".to_owned()
            }
        } else {
            "Recovery with limits: existing branches/commits remain; no worktree edits are backed up by checkout.".to_owned()
        };
        let guard_worktree =
            matches!(&action, Job::StashAction { operation, .. } if operation != "drop");
        let mut lines=ignore.as_ref().map_or_else(Vec::new,|edit|edit.lines.clone());
        lines.push(recovery);
        lines.push("y performs this reviewed action; n / Escape cancels. Enter does not approve. Preview expires in 60 seconds.".into());
        let prepared = Self {
            ignore,
            ticket: ticket.clone(),
            action,
            expires: Instant::now() + Duration::from_secs(60),
            state: state(&session, guard_worktree)?,
            file,
            retained_oid,
            guard_worktree,
        };
        Ok((
            prepared,
            Reply::Prepared {
                generation,
                ticket,
                lines,
            },
        ))
    }

    /// Consumed before calling: unknown outcomes can never replay this token.
    pub fn authorize(
        self,
        options: &Options,
        worktree: &Path,
        ticket: &str,
    ) -> Result<(Job, String), String> {
        if ticket != self.ticket || Instant::now() >= self.expires {
            return Err("confirmation expired or was already consumed; preview again".to_owned());
        }
        let session = open_trusted(options, worktree)?;
        if state(&session, self.guard_worktree)? != self.state {
            return Err("repository changed since preview; nothing was run".to_owned());
        }
        if let Some(edit)=self.ignore {
            let note=edit.publish(&session.installation,&session.identity).map_err(|e|e.to_string())?;
            return Ok((self.action,note));
        }
        let mut note = String::new();
        if let Some((path, bytes)) = &self.file {
            if content(&session.identity.worktree_root, path)? != *bytes {
                return Err("file changed since preview; nothing was discarded".to_owned());
            }
            let profile = fs::canonicalize(profile_root(options)?).map_err(|e| e.to_string())?;
            if profile.starts_with(&session.identity.worktree_root) {
                return Err(
                    "recovery profile must be outside the repository; discard blocked".to_owned(),
                );
            }
            let root = profile.join("recovery");
            fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let used = recovery_usage(&root)?;
            let manifest = format!(
                "worktree state: {}\npath encoding: {:?}\nindex unchanged by discard\n",
                if bytes.is_some() { "file" } else { "deleted" },
                path.encoded()
            );
            let needed = bytes.as_ref().map_or(0, |v| v.len() as u64).saturating_add(manifest.len() as u64);
            if used.saturating_add(needed) > RECOVERY_BUDGET {
                return Err(
                    "recovery storage budget reached; no record was evicted and discard is blocked"
                        .to_owned(),
                );
            }
            let recovery = root.join(&self.ticket);
            fs::create_dir(&recovery)
                .map_err(|e| format!("recovery unavailable; discard blocked: {e}"))?;
            minicon_platform::filesystem::protect_private_directory(&recovery)
                .map_err(|e| format!("private recovery storage unavailable; discard blocked: {e}"))?;
            if let Some(bytes) = bytes {
                let file = recovery.join("worktree.bin");
                srcmgr_state::write_atomic(&file, bytes)
                    .map_err(|e| format!("safety copy failed; discard blocked: {e}"))?;
                if fs::read(file).map_err(|e| e.to_string())? != *bytes {
                    return Err("safety copy verification failed; discard blocked".to_owned());
                }
            }
            srcmgr_state::write_atomic(&recovery.join("record.txt"), manifest.as_bytes())
                .map_err(|e| format!("recovery record failed; discard blocked: {e}"))?;
            note = format!(
                "{}Worktree safety copy: recovery/{} in this profile; restore manually; index kept",
                if used.saturating_add(needed) >= RECOVERY_BUDGET * 4 / 5 { "Recovery storage >=80% of budget. " } else { "" },
                self.ticket
            );
        }
        if let (Job::DeleteBranch(name), Some(oid)) = (&self.action, &self.retained_oid) {
            let current = srcmgr_git::reference_target(
                &session.installation,
                &session.identity,
                &format!("refs/heads/{name}"),
            )
            .map_err(|e| e.to_string())?;
            if &current != oid {
                return Err("branch changed since preview; nothing was deleted".to_owned());
            }
        }
        if let Some(oid) = &self.retained_oid {
            srcmgr_git::retain_recovery_ref(
                &session.installation,
                &session.identity,
                &self.ticket,
                oid,
            )
            .map_err(|e| format!("recovery ref failed; removal blocked: {e}"))?;
            note = format!("Recovery ref: refs/srcmgr/recovery/{}", self.ticket);
        }
        Ok((self.action, note))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> std::path::PathBuf {
        let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = parent.join(format!("srcmgr-recovery-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        root
    }

    fn remove(root: &Path) {
        let root = fs::canonicalize(root).unwrap();
        assert_eq!(root.parent(), Some(fs::canonicalize(std::env::temp_dir()).unwrap().as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_accounting_includes_nested_transaction_payloads() {
        let root = root("nested");
        fs::create_dir_all(root.join("record/git/logs/refs")).unwrap();
        fs::write(root.join("record/worktree.bin"), b"payload").unwrap();
        fs::write(root.join("record/git/logs/refs/stash"), b"reflog").unwrap();
        assert_eq!(recovery_usage(&root).unwrap(), 13);
        assert_eq!(fs::read(root.join("record/worktree.bin")).unwrap(), b"payload");
        remove(&root);
    }

    #[test]
    fn storage_accounting_refuses_excessive_depth_without_deleting_records() {
        let root = root("depth");
        let mut deep = root.clone();
        for _ in 0..34 { deep.push("n"); fs::create_dir(&deep).unwrap(); }
        fs::write(deep.join("keep"), b"retained").unwrap();
        assert!(recovery_usage(&root).unwrap_err().contains("exceeds its bound"));
        assert_eq!(fs::read(deep.join("keep")).unwrap(), b"retained");
        remove(&root);
    }
}
