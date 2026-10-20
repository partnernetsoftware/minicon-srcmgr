//! Splits a single-file unified diff into its file header and hunks. Pure text
//! handling, so it can be tested without Git. A hunk is kept byte-for-byte,
//! because `git apply` needs the exact lines to match the index.

/// A single-file patch split into its header and its text hunks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hunks {
    /// Everything before the first `@@` line: `diff --git`, index, `---`, `+++`.
    pub header: String,
    /// Each hunk from its `@@` line up to, not including, the next one.
    pub hunks: Vec<String>,
}

/// Splits `patch`. Returns `None` when the patch has no text hunks, such as a
/// binary change or an empty diff.
#[must_use]
pub fn split(patch: &str) -> Option<Hunks> {
    let mut header = String::new();
    let mut hunks: Vec<String> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("@@") {
            hunks.push(line.to_owned());
        } else if let Some(current) = hunks.last_mut() {
            current.push_str(line);
        } else {
            header.push_str(line);
        }
    }
    if hunks.is_empty() {
        return None;
    }
    Some(Hunks { header, hunks })
}

/// Keeps only the selected changes of one hunk, for a line-level stage or
/// unstage. `selected` holds indexes into the hunk's lines, where index 0 is the
/// `@@` line. An unselected `+` line is dropped and an unselected `-` line
/// becomes context, so the rest of the hunk still applies to the index. Returns
/// `None` when no selected line is a change.
#[must_use]
pub fn select_lines(hunk: &str, selected: &[usize]) -> Option<String> {
    let mut lines = hunk.split_inclusive('\n');
    let (old_start, new_start) = starts(lines.next()?)?;
    let mut body = String::new();
    let (mut old, mut new) = (0usize, 0usize);
    let mut changed = false;
    // Whether the previous body line was kept, so its "\ No newline" marker is too.
    let mut kept_previous = true;
    for (index, line) in lines.enumerate() {
        let chosen = selected.contains(&(index + 1));
        match line.as_bytes().first() {
            Some(b'+') => {
                kept_previous = chosen;
                if chosen {
                    changed = true;
                    new += 1;
                    body.push_str(line);
                }
            }
            Some(b'-') => {
                kept_previous = true;
                old += 1;
                if chosen {
                    changed = true;
                    body.push_str(line);
                } else {
                    new += 1;
                    body.push(' ');
                    body.push_str(&line[1..]);
                }
            }
            Some(b'\\') => {
                if kept_previous {
                    body.push_str(line);
                }
            }
            _ => {
                kept_previous = true;
                old += 1;
                new += 1;
                body.push_str(line);
            }
        }
    }
    if !changed {
        return None;
    }
    Some(format!("@@ -{old_start},{old} +{new_start},{new} @@\n{body}"))
}

/// The old and new start lines of a hunk header.
fn starts(header: &str) -> Option<(usize, usize)> {
    let mut parts = header.split_whitespace().skip(1);
    let old = parts.next()?.strip_prefix('-')?.split(',').next()?.parse().ok()?;
    let new = parts.next()?.strip_prefix('+')?.split(',').next()?.parse().ok()?;
    Some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A raw string keeps the leading space of context lines; a backslash
    // continuation would strip it.
    const TWO_HUNKS: &str = r"diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
-one
+ONE
 two
@@ -19,2 +19,2 @@
 nineteen
-twenty
+TWENTY
";

    #[test]
    fn header_and_each_hunk_are_kept_exactly() {
        let parts = split(TWO_HUNKS).expect("hunks");
        assert!(parts.header.starts_with("diff --git a/a.txt"));
        assert!(parts.header.ends_with("+++ b/a.txt\n"));
        assert_eq!(parts.hunks.len(), 2);
        assert_eq!(parts.hunks[0], "@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n");
        assert_eq!(parts.hunks[1], "@@ -19,2 +19,2 @@\n nineteen\n-twenty\n+TWENTY\n");
    }

    #[test]
    fn binary_or_empty_patch_has_no_text_hunks() {
        assert!(split("").is_none());
        assert!(split("diff --git a/x b/x\nBinary files a/x and b/x differ\n").is_none());
    }

    #[test]
    fn selecting_one_addition_turns_its_sibling_removal_into_context() {
        let hunk = "@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n";
        assert_eq!(select_lines(hunk, &[2]).as_deref(), Some("@@ -1,2 +1,3 @@\n one\n+ONE\n two\n"));
    }

    #[test]
    fn selecting_one_removal_keeps_the_addition_out_of_the_patch() {
        let hunk = "@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n";
        assert_eq!(select_lines(hunk, &[1]).as_deref(), Some("@@ -1,2 +1,1 @@\n-one\n two\n"));
    }

    #[test]
    fn a_selection_with_no_changed_line_is_refused() {
        assert!(select_lines("@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n", &[3]).is_none());
    }

    #[test]
    fn a_missing_final_newline_stays_missing() {
        let parts = split("--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y").expect("hunks");
        assert_eq!(parts.hunks[0], "@@ -1 +1 @@\n-x\n+y");
    }
}
