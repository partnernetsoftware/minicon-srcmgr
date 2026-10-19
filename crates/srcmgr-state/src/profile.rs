use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The platform's default profile root (PRD 13 templates). `None` when the
/// environment does not name a home or local-data location.
#[must_use]
pub fn default_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("PartnerNetSoftware").join("MiniConSrcMgr"))
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("MiniConSrcMgr")
        })
    } else {
        env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state")))
            .map(|base| base.join("minicon-srcmgr"))
    }
}

/// Replaces `path` with `bytes` through a flushed temporary file and a rename,
/// so a reader sees either the old complete file or the new one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    {
        let mut file = create_private(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)
}

fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
