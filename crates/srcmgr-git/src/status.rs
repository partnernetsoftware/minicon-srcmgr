//! Parser for `git status --porcelain=v2 -z --branch` (Git's documented,
//! locale-independent machine format). Only raw OIDs and NUL-delimited path
//! bytes are consumed; localized human text is never read.

use srcmgr_domain::{
    BackendKind, ChangeEntry, ChangeKind, EntryId, Generation, HeadState, NativePath,
    RepositoryId, RevisionToken, StatusSnapshot,
};

use crate::error::GitError;

/// Parses one complete status report into a generation-tagged snapshot.
pub fn parse_status(
    bytes: &[u8],
    repository: RepositoryId,
    generation: Generation,
) -> Result<StatusSnapshot, GitError> {
    let mut records = bytes.split(|byte| *byte == 0).enumerate().peekable();
    let mut head_oid: Option<&[u8]> = None;
    let mut head_name: Option<&[u8]> = None;
    let mut entries = Vec::new();

    while let Some((index, record)) = records.next() {
        if record.is_empty() {
            // The final NUL terminator yields one trailing empty record.
            if records.peek().is_none() {
                break;
            }
            return Err(GitError::MalformedOutput { record: index });
        }
        match record.first() {
            Some(b'#') => parse_header(record, index, &mut head_oid, &mut head_name)?,
            Some(b'1') => {
                let fields = split_fields(record, 9, index)?;
                entries.push(change_entry(
                    generation,
                    entries.len(),
                    fields[1],
                    fields[8],
                    None,
                    index,
                )?);
            }
            Some(b'2') => {
                let fields = split_fields(record, 10, index)?;
                // A rename or copy carries its original path in the next record.
                let (_, original) = records.next().ok_or(GitError::MalformedOutput { record: index })?;
                if original.is_empty() {
                    return Err(GitError::MalformedOutput { record: index });
                }
                entries.push(change_entry(
                    generation,
                    entries.len(),
                    fields[1],
                    fields[9],
                    Some(original),
                    index,
                )?);
            }
            Some(b'u') => {
                let fields = split_fields(record, 11, index)?;
                entries.push(ChangeEntry {
                    id: EntryId::new(generation, entries.len()),
                    path: native(fields[10])?,
                    original_path: None,
                    staged: ChangeKind::Unmerged,
                    unstaged: ChangeKind::Unmerged,
                });
            }
            Some(b'?') => {
                let path = split_fields(record, 2, index)?[1];
                entries.push(ChangeEntry {
                    id: EntryId::new(generation, entries.len()),
                    path: native(path)?,
                    original_path: None,
                    staged: ChangeKind::Unmodified,
                    unstaged: ChangeKind::Untracked,
                });
            }
            Some(b'!') => {
                let path = split_fields(record, 2, index)?[1];
                entries.push(ChangeEntry {
                    id: EntryId::new(generation, entries.len()),
                    path: native(path)?,
                    original_path: None,
                    staged: ChangeKind::Unmodified,
                    unstaged: ChangeKind::Ignored,
                });
            }
            _ => return Err(GitError::MalformedOutput { record: index }),
        }
    }

    let head = head_state(head_oid, head_name)?;
    Ok(StatusSnapshot {
        backend: BackendKind::Git,
        repository,
        generation,
        head,
        entries,
    })
}

fn parse_header<'a>(
    record: &'a [u8],
    index: usize,
    head_oid: &mut Option<&'a [u8]>,
    head_name: &mut Option<&'a [u8]>,
) -> Result<(), GitError> {
    if let Some(value) = record.strip_prefix(b"# branch.oid ") {
        *head_oid = Some(value);
    } else if let Some(value) = record.strip_prefix(b"# branch.head ") {
        *head_name = Some(value);
    } else if !record.starts_with(b"# branch.") {
        return Err(GitError::MalformedOutput { record: index });
    }
    Ok(())
}

fn head_state(oid: Option<&[u8]>, name: Option<&[u8]>) -> Result<HeadState, GitError> {
    let oid = oid.ok_or(GitError::MissingBranchHeader)?;
    let name = name.ok_or(GitError::MissingBranchHeader)?;
    let name = std::str::from_utf8(name).map_err(|_| GitError::UnrepresentableName)?;
    if name == "(detached)" {
        return Ok(HeadState::Detached {
            revision: revision(oid)?,
        });
    }
    let revision = if oid == b"(initial)" {
        None
    } else {
        Some(revision(oid)?)
    };
    Ok(HeadState::Branch {
        name: name.to_owned(),
        revision,
    })
}

/// Object names are opaque: the two defined Git object formats are 40 (SHA-1)
/// and 64 (SHA-256) hexadecimal digits. Nothing else is accepted.
pub(crate) fn revision(oid: &[u8]) -> Result<RevisionToken, GitError> {
    let valid = matches!(oid.len(), 40 | 64) && oid.iter().all(u8::is_ascii_hexdigit);
    if !valid {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    let text = std::str::from_utf8(oid).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    Ok(RevisionToken::new(BackendKind::Git, text.to_ascii_lowercase()))
}

/// Splits the first `count - 1` space-separated fields and keeps the remainder
/// as the final field, so paths containing spaces survive intact.
fn split_fields(record: &[u8], count: usize, index: usize) -> Result<Vec<&[u8]>, GitError> {
    let mut fields = Vec::with_capacity(count);
    let mut rest = record;
    for _ in 0..count - 1 {
        let space = rest
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or(GitError::MalformedOutput { record: index })?;
        fields.push(&rest[..space]);
        rest = &rest[space + 1..];
    }
    fields.push(rest);
    Ok(fields)
}

fn change_entry(
    generation: Generation,
    ordinal: usize,
    xy: &[u8],
    path: &[u8],
    original: Option<&[u8]>,
    index: usize,
) -> Result<ChangeEntry, GitError> {
    let [x, y] = xy else {
        return Err(GitError::MalformedOutput { record: index });
    };
    Ok(ChangeEntry {
        id: EntryId::new(generation, ordinal),
        path: native(path)?,
        original_path: original.map(native).transpose()?,
        staged: area_kind(*x, index)?,
        unstaged: area_kind(*y, index)?,
    })
}

fn area_kind(code: u8, index: usize) -> Result<ChangeKind, GitError> {
    Ok(match code {
        b'.' => ChangeKind::Unmodified,
        b'M' => ChangeKind::Modified,
        b'T' => ChangeKind::TypeChanged,
        b'A' => ChangeKind::Added,
        b'D' => ChangeKind::Deleted,
        b'R' => ChangeKind::Renamed,
        b'C' => ChangeKind::Copied,
        b'U' => ChangeKind::Unmerged,
        _ => return Err(GitError::MalformedOutput { record: index }),
    })
}

fn native(bytes: &[u8]) -> Result<NativePath, GitError> {
    NativePath::from_tool_bytes(bytes).map_err(|_| GitError::UnrepresentableName)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    fn repository() -> RepositoryId {
        RepositoryId::new(BackendKind::Git, "/repo/.git")
    }

    fn report(body: &str) -> Vec<u8> {
        body.as_bytes().to_vec()
    }

    #[test]
    fn branch_header_and_mixed_entries_are_read() {
        let body = format!(
            "# branch.oid {OID}\0# branch.head main\0\
             1 .M N... 100644 100644 100644 aaa bbb src/lib.rs\0\
             1 M. N... 100644 100644 100644 aaa bbb docs/with space.md\0\
             ? new file.txt\0"
        );
        let snapshot = parse_status(&report(&body), repository(), Generation(7)).expect("parse");
        assert_eq!(snapshot.generation, Generation(7));
        assert_eq!(
            snapshot.head,
            HeadState::Branch {
                name: "main".to_owned(),
                revision: Some(RevisionToken::new(BackendKind::Git, OID)),
            }
        );
        assert_eq!(snapshot.entries.len(), 3);
        assert_eq!(snapshot.entries[0].unstaged, ChangeKind::Modified);
        assert_eq!(snapshot.entries[0].staged, ChangeKind::Unmodified);
        assert_eq!(snapshot.entries[1].path.as_os_str(), "docs/with space.md");
        assert_eq!(snapshot.entries[2].unstaged, ChangeKind::Untracked);
        assert_eq!(snapshot.entries[2].id.as_str(), "g7-2");
    }

    #[test]
    fn rename_consumes_its_original_path_record() {
        let body = format!(
            "# branch.oid {OID}\0# branch.head main\0\
             2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\0old.rs\0"
        );
        let snapshot = parse_status(&report(&body), repository(), Generation(1)).expect("parse");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].staged, ChangeKind::Renamed);
        assert_eq!(
            snapshot.entries[0].original_path.as_ref().map(NativePath::display_label),
            Some("old.rs".to_owned())
        );
    }

    #[test]
    fn unborn_branch_has_no_revision() {
        let body = "# branch.oid (initial)\0# branch.head trunk\0";
        let snapshot = parse_status(&report(body), repository(), Generation(1)).expect("parse");
        assert_eq!(
            snapshot.head,
            HeadState::Branch {
                name: "trunk".to_owned(),
                revision: None,
            }
        );
    }

    #[test]
    fn detached_head_reports_its_revision() {
        let body = format!("# branch.oid {OID}\0# branch.head (detached)\0");
        let snapshot = parse_status(&report(&body), repository(), Generation(1)).expect("parse");
        assert!(matches!(snapshot.head, HeadState::Detached { .. }));
    }

    #[test]
    fn unexpected_record_is_refused_without_echoing_it() {
        let body = format!("# branch.oid {OID}\0# branch.head main\0zzz secret\0");
        match parse_status(&report(&body), repository(), Generation(1)) {
            Err(GitError::MalformedOutput { record: 2 }) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn short_object_name_is_refused() {
        let body = "# branch.oid abc\0# branch.head main\0";
        assert!(parse_status(&report(body), repository(), Generation(1)).is_err());
    }
}
