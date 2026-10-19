//! Pure view model for the status screen. It turns a domain snapshot into
//! toned text lines without touching a window, so it is testable on any host.

use srcmgr_domain::{ChangeKind, HeadState, StatusSnapshot};

/// Semantic colour role. The window maps roles to pixels; the view never does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Heading,
    Text,
    Muted,
    Staged,
    Changed,
    Untracked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub tone: Tone,
    pub text: String,
}

/// Everything the status window shows, in order from top to bottom.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusView {
    pub title: String,
    pub lines: Vec<Line>,
}

impl StatusView {
    #[must_use]
    pub fn from_snapshot(snapshot: &StatusSnapshot, title: impl Into<String>) -> Self {
        let mut lines = vec![Line {
            tone: Tone::Heading,
            text: head_text(&snapshot.head),
        }];
        if snapshot.entries.is_empty() {
            lines.push(Line {
                tone: Tone::Muted,
                text: "Working tree clean".to_owned(),
            });
        }
        for entry in &snapshot.entries {
            lines.push(Line {
                tone: tone_of(entry.staged, entry.unstaged),
                text: format!(
                    "{}{} {}",
                    code(entry.staged),
                    code(entry.unstaged),
                    entry.path.display_label()
                ),
            });
        }
        Self {
            title: title.into(),
            lines,
        }
    }
}

fn head_text(head: &HeadState) -> String {
    match head {
        HeadState::Branch { name, revision: Some(revision) } => {
            format!("On branch {name} ({})", short(revision.token()))
        }
        HeadState::Branch { name, revision: None } => format!("On branch {name} (no commits yet)"),
        HeadState::Detached { revision } => format!("HEAD detached at {}", short(revision.token())),
    }
}

fn tone_of(staged: ChangeKind, unstaged: ChangeKind) -> Tone {
    if unstaged == ChangeKind::Untracked {
        Tone::Untracked
    } else if staged != ChangeKind::Unmodified {
        Tone::Staged
    } else {
        Tone::Changed
    }
}

fn short(token: &str) -> &str {
    &token[..token.len().min(12)]
}

fn code(kind: ChangeKind) -> char {
    match kind {
        ChangeKind::Unmodified => '.',
        ChangeKind::Modified => 'M',
        ChangeKind::TypeChanged => 'T',
        ChangeKind::Added => 'A',
        ChangeKind::Deleted => 'D',
        ChangeKind::Renamed => 'R',
        ChangeKind::Copied => 'C',
        ChangeKind::Unmerged => 'U',
        ChangeKind::Untracked => '?',
        ChangeKind::Ignored => '!',
    }
}

#[cfg(test)]
mod tests {
    use srcmgr_domain::{
        BackendKind, ChangeEntry, EntryId, Generation, NativePath, RepositoryId, RevisionToken,
    };

    use super::*;

    fn snapshot(entries: Vec<ChangeEntry>) -> StatusSnapshot {
        StatusSnapshot {
            backend: BackendKind::Git,
            repository: RepositoryId::new(BackendKind::Git, "/r/.git"),
            generation: Generation(1),
            head: HeadState::Branch {
                name: "main".to_owned(),
                revision: Some(RevisionToken::new(BackendKind::Git, "0123456789abcdef0123456789abcdef01234567")),
            },
            entries,
        }
    }

    fn entry(path: &str, staged: ChangeKind, unstaged: ChangeKind) -> ChangeEntry {
        ChangeEntry {
            id: EntryId::new(Generation(1), 0),
            path: NativePath::new(path),
            original_path: None,
            staged,
            unstaged,
        }
    }

    #[test]
    fn clean_tree_says_so_under_the_heading() {
        let view = StatusView::from_snapshot(&snapshot(Vec::new()), "t");
        assert_eq!(view.lines[0].text, "On branch main (0123456789ab)");
        assert_eq!(view.lines[1], Line { tone: Tone::Muted, text: "Working tree clean".to_owned() });
    }

    #[test]
    fn entries_carry_codes_and_tones() {
        let view = StatusView::from_snapshot(
            &snapshot(vec![
                entry("a.txt", ChangeKind::Modified, ChangeKind::Unmodified),
                entry("b.txt", ChangeKind::Unmodified, ChangeKind::Modified),
                entry("c.txt", ChangeKind::Unmodified, ChangeKind::Untracked),
            ]),
            "t",
        );
        assert_eq!(view.lines[1], Line { tone: Tone::Staged, text: "M. a.txt".to_owned() });
        assert_eq!(view.lines[2], Line { tone: Tone::Changed, text: ".M b.txt".to_owned() });
        assert_eq!(view.lines[3], Line { tone: Tone::Untracked, text: ".? c.txt".to_owned() });
    }

    #[test]
    fn unborn_branch_is_labelled() {
        let mut unborn = snapshot(Vec::new());
        unborn.head = HeadState::Branch { name: "trunk".to_owned(), revision: None };
        let view = StatusView::from_snapshot(&unborn, "t");
        assert_eq!(view.lines[0].text, "On branch trunk (no commits yet)");
    }
}
