//! Inspecting and removing the partial directory a failed clone left behind.
//!
//! A clone that fails keeps its target so the user can see what happened. The
//! product never deletes it on its own: removal is offered only for a target
//! this attempt created, only from an explicit review, and only through a
//! one-use ticket. Everything here refuses rather than guesses.

use std::path::{Component, Path};

/// How many entries a listing shows before it says there are more.
pub const LIST_LIMIT: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Listing {
    /// Paths relative to the target, with `/` separators, sorted.
    pub entries: Vec<String>,
    /// True when the walk stopped at the limit.
    pub more: bool,
}

/// Lists the target without following a symlink and without reading file
/// contents. Names are shown literally except for control characters, which
/// would disturb the line they are drawn on.
pub fn inspect(target: &Path) -> Result<Listing, String> {
    let metadata = std::fs::symlink_metadata(target)
        .map_err(|_| "the clone target is no longer there; nothing to inspect".to_owned())?;
    if metadata.file_type().is_symlink() {
        return Err("the clone target became a symbolic link; refusing to follow it".into());
    }
    if !metadata.is_dir() {
        return Err("the clone target is not a directory; refusing to inspect it".into());
    }
    let mut entries = Vec::new();
    let mut more = false;
    walk(target, target, &mut entries, &mut more)?;
    entries.sort();
    Ok(Listing { entries, more })
}

fn walk(root: &Path, directory: &Path, entries: &mut Vec<String>, more: &mut bool) -> Result<(), String> {
    let list = match std::fs::read_dir(directory) {
        Ok(list) => list,
        // A directory the clone removed between the two reads is not an error.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", display(directory, root))),
    };
    for entry in list {
        if entries.len() >= LIST_LIMIT {
            *more = true;
            return Ok(());
        }
        let entry = entry.map_err(|error| format!("cannot read an entry: {error}"))?;
        let path = entry.path();
        let name = display(&path, root);
        let file_type = entry.file_type().map_err(|error| format!("cannot inspect {name}: {error}"))?;
        entries.push(if file_type.is_dir() && !file_type.is_symlink() { format!("{name}/") } else { name });
        // A symlink is listed but never entered: it may point anywhere.
        if file_type.is_dir() && !file_type.is_symlink() {
            walk(root, &path, entries, more)?;
            if *more {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Removes the target and everything under it. The caller must have shown the
/// listing and obtained a one-use ticket; this function only repeats the checks
/// that make the operation safe to perform at all.
pub fn remove(target: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err("refusing to remove a relative path".into());
    }
    if target
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("refusing to remove a path containing '..'".into());
    }
    // A root or a bare drive has no parent; the clone preview never allows one,
    // and this check keeps a corrupt record from becoming a disaster.
    if target.parent().is_none() {
        return Err("refusing to remove a filesystem root".into());
    }
    let metadata = std::fs::symlink_metadata(target)
        .map_err(|_| "the clone target is already gone".to_owned())?;
    if metadata.file_type().is_symlink() {
        return Err("the clone target became a symbolic link; refusing to follow it".into());
    }
    if !metadata.is_dir() {
        return Err("the clone target is not a directory; refusing to remove it".into());
    }
    std::fs::remove_dir_all(target).map_err(|error| format!("the target could not be removed: {error}"))
}

fn display(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    text.chars().map(|ch| if ch.is_control() { '?' } else { ch }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let base = {
            #[cfg(unix)]
            {
                fs::canonicalize(std::env::temp_dir()).unwrap()
            }
            #[cfg(windows)]
            {
                std::env::temp_dir()
            }
        };
        let root = base.join(format!("srcmgr-cleanup-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_listing_names_relative_paths_and_marks_directories() {
        let root = scratch("listing");
        fs::create_dir_all(root.join("target").join(".git")).unwrap();
        fs::write(root.join("target").join(".git").join("HEAD"), "ref: x\n").unwrap();
        fs::write(root.join("target").join("notes.txt"), "x").unwrap();
        let listing = inspect(&root.join("target")).unwrap();
        assert_eq!(listing.entries, vec![".git/", ".git/HEAD", "notes.txt"]);
        assert!(!listing.more);
    }

    #[test]
    fn a_listing_stops_at_the_limit_and_says_so() {
        let root = scratch("limit");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        for index in 0..(LIST_LIMIT + 5) {
            fs::write(target.join(format!("f{index:04}")), "x").unwrap();
        }
        let listing = inspect(&target).unwrap();
        assert_eq!(listing.entries.len(), LIST_LIMIT);
        assert!(listing.more);
    }

    #[test]
    fn a_missing_target_is_reported_rather_than_created() {
        let root = scratch("missing");
        assert!(inspect(&root.join("absent")).is_err());
        assert!(remove(&root.join("absent")).is_err());
        assert!(!root.join("absent").exists());
    }

    #[test]
    fn removal_deletes_only_the_target_it_was_given() {
        let root = scratch("remove");
        fs::create_dir_all(root.join("target").join("sub")).unwrap();
        fs::write(root.join("target").join("sub").join("a"), "x").unwrap();
        fs::write(root.join("keep.txt"), "keep\n").unwrap();
        remove(&root.join("target")).unwrap();
        assert!(!root.join("target").exists());
        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "keep\n");
    }

    #[test]
    fn a_relative_parent_or_root_path_is_refused() {
        assert!(remove(Path::new("target")).is_err());
        assert!(remove(Path::new("a/../b")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_target_is_never_followed_or_removed() {
        use std::os::unix::fs::symlink;
        let root = scratch("symlink");
        fs::create_dir_all(root.join("real")).unwrap();
        fs::write(root.join("real").join("a"), "x").unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();
        assert!(inspect(&root.join("link")).is_err());
        assert!(remove(&root.join("link")).is_err());
        assert!(root.join("real").join("a").exists(), "the link target must survive");
    }
}
