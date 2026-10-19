//! Pure view model for the status screen. It turns a domain snapshot into
//! toned text lines and tracks the selected entry, without touching a window,
//! so it is testable on any host.

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
    Added,
    Removed,
    Hunk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub tone: Tone,
    pub text: String,
}

/// Everything the status window shows: a heading, one row per changed entry,
/// and the diff of the selected entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusView {
    pub title: String,
    /// Heading first, then one row per entry (or the clean-tree note).
    pub lines: Vec<Line>,
    entries: usize,
    diffs: Vec<Vec<Line>>,
    selected: usize,
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
            entries: snapshot.entries.len(),
            diffs: Vec::new(),
            selected: 0,
        }
    }

    /// Attaches one diff text per entry, in snapshot order. Missing trailing
    /// entries show as empty; extra texts are ignored.
    pub fn attach_diffs(&mut self, texts: Vec<String>) {
        self.diffs = (0..self.entries)
            .map(|index| texts.get(index).map_or_else(Vec::new, |text| diff_lines(text)))
            .collect();
    }

    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.entries
    }

    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Moves the selection by one entry, clamped to the list.
    pub fn move_selection(&mut self, forward: bool) {
        if self.entries == 0 {
            return;
        }
        self.selected = if forward {
            (self.selected + 1).min(self.entries - 1)
        } else {
            self.selected.saturating_sub(1)
        };
    }

    /// The diff of the selected entry, or nothing when none is attached.
    #[must_use]
    pub fn selected_diff(&self) -> &[Line] {
        self.diffs.get(self.selected).map_or(&[], Vec::as_slice)
    }
}

fn diff_lines(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| {
            let tone = if line.starts_with("+++") || line.starts_with("---") {
                Tone::Muted
            } else if line.starts_with('+') {
                Tone::Added
            } else if line.starts_with('-') {
                Tone::Removed
            } else if line.starts_with("@@") {
                Tone::Hunk
            } else {
                Tone::Text
            };
            Line {
                tone,
                text: line.to_owned(),
            }
        })
        .collect()
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

    fn three_entries() -> StatusView {
        StatusView::from_snapshot(
            &snapshot(vec![
                entry("a.txt", ChangeKind::Modified, ChangeKind::Unmodified),
                entry("b.txt", ChangeKind::Unmodified, ChangeKind::Modified),
                entry("c.txt", ChangeKind::Unmodified, ChangeKind::Untracked),
            ]),
            "t",
        )
    }

    #[test]
    fn clean_tree_says_so_under_the_heading() {
        let view = StatusView::from_snapshot(&snapshot(Vec::new()), "t");
        assert_eq!(view.lines[0].text, "On branch main (0123456789ab)");
        assert_eq!(view.lines[1], Line { tone: Tone::Muted, text: "Working tree clean".to_owned() });
        assert_eq!(view.entry_count(), 0);
    }

    #[test]
    fn entries_carry_codes_and_tones() {
        let view = three_entries();
        assert_eq!(view.lines[1], Line { tone: Tone::Staged, text: "M. a.txt".to_owned() });
        assert_eq!(view.lines[2], Line { tone: Tone::Changed, text: ".M b.txt".to_owned() });
        assert_eq!(view.lines[3], Line { tone: Tone::Untracked, text: ".? c.txt".to_owned() });
    }

    #[test]
    fn selection_is_clamped_to_the_list() {
        let mut view = three_entries();
        view.move_selection(false);
        assert_eq!(view.selected(), 0);
        for _ in 0..5 {
            view.move_selection(true);
        }
        assert_eq!(view.selected(), 2);
    }

    #[test]
    fn selected_diff_follows_selection_and_tones_hunks() {
        let mut view = three_entries();
        view.attach_diffs(vec![
            String::new(),
            "@@ -1 +1 @@\n-old\n+new\n context\n".to_owned(),
            String::new(),
        ]);
        assert!(view.selected_diff().is_empty());
        view.move_selection(true);
        let tones: Vec<Tone> = view.selected_diff().iter().map(|line| line.tone).collect();
        assert_eq!(tones, vec![Tone::Hunk, Tone::Removed, Tone::Added, Tone::Text]);
    }

    #[test]
    fn unborn_branch_is_labelled() {
        let mut unborn = snapshot(Vec::new());
        unborn.head = HeadState::Branch { name: "trunk".to_owned(), revision: None };
        let view = StatusView::from_snapshot(&unborn, "t");
        assert_eq!(view.lines[0].text, "On branch trunk (no commits yet)");
    }
}
