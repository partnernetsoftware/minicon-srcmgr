//! Publish an exact stash removal under Git's files-backend locks.
//!
//! Installed Git generates the new reflog/ref in an isolated store. We only
//! publish those bytes while the original ref is locked; no second Git engine
//! or hand-written reflog rewrite is used. A saved transaction permits manual
//! repair if publication is interrupted between its two file replacements.
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::{GitError, GitInstallation, RepositoryIdentity, runner};

struct Lock {
    path: PathBuf,
    file: Option<File>,
    owned: bool,
}

impl Lock {
    fn acquire(path: PathBuf) -> Result<Self, GitError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(GitError::Io)?;
        Ok(Self {
            path,
            file: Some(file),
            owned: true,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), GitError> {
        let file = self.file.as_mut().ok_or(GitError::Supervision)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(GitError::Io)
    }

    fn publish(&mut self, destination: &Path) -> Result<(), GitError> {
        self.file.take();
        minicon_platform::filesystem::replace_file(&self.path, destination)
            .map_err(GitError::Io)?;
        self.owned = false;
        minicon_platform::filesystem::sync_parent(
            destination.parent().ok_or(GitError::NotARepository)?,
        )
        .map_err(GitError::Io)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.file.take();
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn bounded(path: &Path) -> Result<Vec<u8>, GitError> {
    let meta = fs::symlink_metadata(path).map_err(GitError::Io)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 8 * 1024 * 1024 {
        return Err(GitError::UnsupportedStashStorage);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(GitError::Io)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(GitError::Io)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(GitError::OutputLimit);
    }
    Ok(bytes)
}

fn saved(path: &Path, bytes: &[u8]) -> Result<(), GitError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(GitError::Io)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(GitError::Io)
}

fn path_argument(prefix: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(path.as_os_str());
    argument
}

pub(crate) fn remove(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
    expected: &[String],
    transaction: &Path,
) -> Result<(), GitError> {
    remove_locked(git, repo, oid, expected, transaction, |_| Ok(()))
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Locked,
    ReflogPublished,
}

fn remove_locked(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
    expected: &[String],
    transaction: &Path,
    mut checkpoint: impl FnMut(Phase) -> Result<(), GitError>,
) -> Result<(), GitError> {
    // Only ordinary files storage is qualified here. Never follow a custom
    // ref location or replace a symlink/symbolic ref as an ordinary file.
    let refs = repo.common_dir.join("refs");
    let logs = repo.common_dir.join("logs/refs");
    for directory in [&refs, &logs] {
        let meta = fs::symlink_metadata(directory).map_err(GitError::Io)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || !fs::canonicalize(directory)
                .map_err(GitError::Io)?
                .starts_with(&repo.common_dir)
        {
            return Err(GitError::UnsupportedStashStorage);
        }
    }
    let mut ref_lock = Lock::acquire(refs.join("stash.lock"))?;
    let _packed_lock = Lock::acquire(repo.common_dir.join("packed-refs.lock"))?;
    let mut log_lock = Lock::acquire(logs.join("stash.lock"))?;
    let packed = repo.common_dir.join("packed-refs");
    if packed.exists()
        && bounded(&packed)?
            .split(|b| *b == b'\n')
            .any(|line| line.ends_with(b" refs/stash"))
    {
        return Err(GitError::UnsupportedStashStorage);
    }
    let index = crate::stash::check_stash_selection(git, repo, oid, expected)?;
    checkpoint(Phase::Locked)?;
    let original_ref = bounded(&refs.join("stash"))?;
    let original_log = bounded(&logs.join("stash"))?;
    if original_ref.strip_suffix(b"\n") != expected.first().map(|s| s.as_bytes()) {
        return Err(GitError::StaleSelection);
    }
    let parent = transaction.parent().ok_or(GitError::NotARepository)?;
    fs::create_dir_all(parent).map_err(GitError::Io)?;
    let parent = fs::canonicalize(parent).map_err(GitError::Io)?;
    if parent.starts_with(&repo.worktree_root) || parent.starts_with(&repo.common_dir) {
        return Err(GitError::UnsupportedStashStorage);
    }
    fs::create_dir(transaction).map_err(GitError::Io)?;
    minicon_platform::filesystem::protect_private_directory(transaction).map_err(GitError::Io)?;
    let mut publication_started = false;
    let result = (|| {
        saved(&transaction.join("original-ref"), &original_ref)?;
        saved(&transaction.join("original-reflog"), &original_log)?;
        let store = transaction.join("git");
        let work = transaction.join("work");
        fs::create_dir(&work).map_err(GitError::Io)?;
        let format = runner::run(
            git.executable(),
            Some(&repo.worktree_root),
            &["rev-parse", "--show-object-format"],
            None,
            runner::Limits::METADATA,
        )?;
        let format = std::str::from_utf8(&format.stdout)
            .map_err(|_| GitError::InvalidRevision)?
            .trim();
        if !matches!(format, "sha1" | "sha256") {
            return Err(GitError::UnsupportedStashStorage);
        }
        let init: Vec<OsString> = vec![
            "-c".into(),
            "init.defaultRefFormat=files".into(),
            "init".into(),
            "--bare".into(),
            "-q".into(),
            format!("--object-format={format}").into(),
            path_argument("--template=", &work),
            store.as_os_str().to_owned(),
        ];
        runner::run(
            git.executable(),
            Some(&repo.worktree_root),
            &init,
            None,
            runner::Limits::METADATA,
        )?;
        fs::create_dir_all(store.join("logs/refs")).map_err(GitError::Io)?;
        let objects = repo.common_dir.join("objects");
        let objects = objects
            .to_str()
            .ok_or(GitError::UnrepresentableName)?
            .replace('\\', "/");
        let objects = objects.strip_prefix("//?/").unwrap_or(&objects);
        if objects.contains(['\n', '\r']) {
            return Err(GitError::UnsupportedStashStorage);
        }
        saved(
            &store.join("objects/info/alternates"),
            format!("{objects}\n").as_bytes(),
        )?;
        saved(&store.join("refs/stash"), &original_ref)?;
        saved(&store.join("logs/refs/stash"), &original_log)?;
        let temp_run = |args: &[&str]| {
            let mut command: Vec<OsString> = vec![
                path_argument("--git-dir=", &store),
                path_argument("--work-tree=", &work),
            ];
            command.extend(args.iter().map(OsString::from));
            runner::run(
                git.executable(),
                Some(&work),
                &command,
                None,
                runner::Limits::MUTATION,
            )
        };
        temp_run(&["stash", "drop", &format!("stash@{{{index}}}")])?;
        let remaining = temp_run(&["stash", "list", "--format=%H"])?;
        let wanted: Vec<_> = expected
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, id)| id.as_str())
            .collect();
        let actual =
            std::str::from_utf8(&remaining.stdout).map_err(|_| GitError::InvalidRevision)?;
        if actual.lines().collect::<Vec<_>>() != wanted {
            return Err(GitError::StaleSelection);
        }
        let new_log = if wanted.is_empty() {
            Vec::new()
        } else {
            bounded(&store.join("logs/refs/stash"))?
        };
        let new_ref = if wanted.is_empty() {
            None
        } else {
            Some(bounded(&store.join("refs/stash"))?)
        };
        // Recheck even under locks: an unsupported direct metadata writer must
        // not be mistaken for a cooperating Git client.
        if bounded(&refs.join("stash"))? != original_ref
            || bounded(&logs.join("stash"))? != original_log
        {
            return Err(GitError::StaleSelection);
        }
        saved(&transaction.join("publication-started"), b"Original ref/reflog retained here. Publication may be partial; inspect before manual repair. Do not replay apply.\n")?;
        minicon_platform::filesystem::sync_parent(transaction).map_err(GitError::Io)?;
        log_lock.write(&new_log)?;
        if let Some(bytes) = &new_ref {
            ref_lock.write(bytes)?;
        }
        publication_started = true;
        log_lock.publish(&logs.join("stash"))?;
        checkpoint(Phase::ReflogPublished)?;
        if new_ref.is_some() {
            ref_lock.publish(&refs.join("stash"))?;
        } else {
            fs::remove_file(refs.join("stash")).map_err(GitError::Io)?;
            minicon_platform::filesystem::sync_parent(&refs).map_err(GitError::Io)?;
            fs::remove_file(logs.join("stash")).map_err(GitError::Io)?;
            minicon_platform::filesystem::sync_parent(&logs).map_err(GitError::Io)?;
        }
        Ok(())
    })();
    if result.is_ok() || !publication_started {
        // This exclusive-created private child is the only recursive cleanup
        // target, verified outside the repository and under its fixed parent.
        if let Ok(resolved) = fs::canonicalize(transaction)
            && resolved.parent() == Some(parent.as_path())
        {
            let _ = fs::remove_dir_all(resolved);
        }
    }
    if result.is_err() && publication_started {
        Err(GitError::StashPublicationIncomplete)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn external_git_writer_cannot_shift_the_entry_during_locked_removal() {
        let executable = std::env::var_os("SRCMGR_TEST_GIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "git".into());
        let root = std::env::temp_dir().join(format!("srcmgr-stash-lock-{}", std::process::id()));
        // Exclusive creation prevents cleanup of an unrelated existing path.
        fs::create_dir(&root).unwrap();
        let repo_path = root.join("repo");
        fs::create_dir(&repo_path).unwrap();
        let run = |args: &[&str]| {
            let output = Command::new(&executable)
                .current_dir(&repo_path)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.name", "Fixture"]);
        run(&["config", "user.email", "fixture@example.invalid"]);
        fs::write(repo_path.join("a"), "base\n").unwrap();
        run(&["add", "a"]);
        run(&["commit", "-qm", "base"]);
        for content in ["older\n", "newer\n"] {
            fs::write(repo_path.join("a"), content).unwrap();
            run(&["stash", "push", "-q"]);
        }
        let git = GitInstallation::probe(&executable).unwrap();
        let repo = crate::locate(&git, &repo_path).unwrap();
        let expected: Vec<_> = crate::stash_list(&git, &repo)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        let transaction = root.join("recovery/one");
        remove_locked(
            &git,
            &repo,
            &expected[1],
            &expected,
            &transaction,
            |phase| {
                if phase == Phase::Locked {
                    let attempted = Command::new(&executable)
                        .current_dir(&repo_path)
                        .args([
                            "stash",
                            "store",
                            "-m",
                            "racing external writer",
                            &expected[0],
                        ])
                        .output()
                        .unwrap();
                    assert!(
                        !attempted.status.success(),
                        "an external writer shifted the list inside the removal critical section"
                    );
                    assert!(String::from_utf8_lossy(&attempted.stderr).contains("stash.lock"));
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            crate::stash_list(&git, &repo)
                .unwrap()
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
            vec![expected[0].clone()]
        );
        assert_eq!(fs::read_to_string(repo_path.join("a")).unwrap(), "base\n");
        assert!(!transaction.exists());
        assert!(!repo.common_dir.join("refs/stash.lock").exists());
        let original_ref = fs::read(repo.common_dir.join("refs/stash")).unwrap();
        let original_log = fs::read(repo.common_dir.join("logs/refs/stash")).unwrap();
        let recovery = root.join("recovery/partial");
        let partial = remove_locked(
            &git,
            &repo,
            &expected[0],
            &expected[..1],
            &recovery,
            |phase| {
                if phase == Phase::ReflogPublished {
                    Err(GitError::OutputLimit)
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(partial, Err(GitError::StashPublicationIncomplete)));
        assert_eq!(
            fs::read(repo.common_dir.join("refs/stash")).unwrap(),
            original_ref,
            "failed ref publication does not claim completion"
        );
        assert_eq!(
            fs::read(recovery.join("original-ref")).unwrap(),
            original_ref
        );
        assert_eq!(
            fs::read(recovery.join("original-reflog")).unwrap(),
            original_log
        );
        assert!(recovery.join("publication-started").exists());
        assert!(!repo.common_dir.join("refs/stash.lock").exists());
        assert!(!repo.common_dir.join("logs/refs/stash.lock").exists());
        // Explicit manual repair in this isolated fixture, using its retained
        // bytes; production never silently overwrites newer external state.
        fs::write(repo.common_dir.join("logs/refs/stash"), &original_log).unwrap();
        assert_eq!(crate::stash_list(&git, &repo).unwrap()[0].id, expected[0]);
        let canonical = fs::canonicalize(&root).unwrap();
        assert_eq!(
            canonical.parent(),
            Some(fs::canonicalize(std::env::temp_dir()).unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }
}
