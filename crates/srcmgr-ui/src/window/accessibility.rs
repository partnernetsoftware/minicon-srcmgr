//! Product semantics reuse the actual draw geometry and pointer action path.
use crate::messages::{english as message, Key as TextKey};
use super::*;
use srcmgr_platform_ext::{
    AccessibilityBounds, Action, PublishedNode, PublishedRole, PublishedTree,
    Snapshot as AccessibleSnapshot,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Scene {
    tree: PublishedTree,
    disabled: Vec<u32>,
    bindings: BTreeMap<u32, Binding>,
}
#[derive(Clone, Debug, PartialEq)]
struct Binding {
    rect: Rect,
    editable: bool,
}
#[derive(Default)]
struct Flags {
    enabled: bool,
    focused: bool,
    focusable: bool,
    editable: bool,
    clickable: bool,
}
struct Builder {
    scene: Scene,
    next: u32,
}
impl Builder {
    fn new(title: &str, surface: Surface) -> Self {
        let root = PublishedNode {
            id: 1,
            parent: None,
            role: PublishedRole::Frame,
            name: title.into(),
            text: String::new(),
            bounds: AccessibilityBounds {
                x: 0,
                y: 0,
                width: surface.width as i32,
                height: surface.height as i32,
            },
            focusable: false,
            focused: false,
            editable: false,
            clickable: false,
        };
        Self {
            scene: Scene {
                tree: PublishedTree {
                    app_name: "MiniCon SrcMgr".into(),
                    nodes: vec![root],
                },
                disabled: Vec::new(),
                bindings: BTreeMap::new(),
            },
            next: 2,
        }
    }
    fn node(&mut self, name: &str, text: &str, rect: Rect, role: PublishedRole, flags: Flags) {
        let Flags {
            enabled,
            focused,
            focusable,
            editable,
            clickable,
        } = flags;
        let id = self.next;
        self.next += 1;
        self.scene.tree.nodes.push(PublishedNode {
            id,
            parent: Some(1),
            role,
            name: name.into(),
            text: text.into(),
            bounds: AccessibilityBounds {
                x: rect.x as i32,
                y: rect.y as i32,
                width: rect.width.max(0) as i32,
                height: rect.height.max(0) as i32,
            },
            focusable: editable || focusable,
            focused,
            editable,
            clickable,
        });
        if !enabled {
            self.scene.disabled.push(id);
        }
        if editable || clickable {
            self.scene.bindings.insert(id, Binding { rect, editable });
        }
    }
    fn button(&mut self, name: &str, rect: Rect, enabled: bool) {
        self.node(
            name,
            "",
            rect,
            PublishedRole::Button,
            Flags {
                enabled,
                clickable: true,
                ..Flags::default()
            },
        );
    }
    fn input(&mut self, name: &str, text: &str, rect: Rect, enabled: bool, focused: bool) {
        self.node(
            name,
            text,
            rect,
            PublishedRole::Text,
            Flags {
                enabled,
                focused,
                editable: true,
                ..Flags::default()
            },
        );
    }
    fn text(&mut self, name: &str, text: &str, rect: Rect) {
        self.node(
            name,
            text,
            rect,
            PublishedRole::Text,
            Flags {
                enabled: true,
                ..Flags::default()
            },
        );
    }
}

fn build(workspace: &Workspace, surface: Surface, pending: usize) -> Scene {
    let Surface {
        width,
        height,
        grid,
    } = surface;
    let panels = layout(width, height, grid);
    let main = panels.main;
    let mut b = Builder::new(&workspace.snapshot().title, surface);
    let (allow, cancel) = trust_buttons(main, grid);
    if let Some(confirm) = workspace.confirm() {
        let text = std::iter::once(confirmation_title(confirm))
            .chain(workspace.confirmation_lines().iter().cloned())
            .collect::<Vec<_>>()
            .join("\n");
        b.text(
            message(TextKey::ChangesReview),
            &text,
            confirmation_body(main, grid, matches!(confirm, Confirm::Ignore { .. })),
        );
        let ignore = matches!(confirm, Confirm::Ignore { .. });
        if ignore {
            for (rect, name) in ignore_scope_buttons(main, grid)
                .into_iter()
                .zip([message(TextKey::ChangesProjectIgnore), message(TextKey::ChangesLocalExclude)])
            {
                b.button(name, rect, pending == 0);
            }
        }
        b.button(
            if ignore {
                message(TextKey::ChangesSaveRule)
            } else {
                message(TextKey::ChangesConfirm)
            },
            allow,
            workspace.confirmation_ready() && pending == 0,
        );
        b.button(message(TextKey::ActionCancel), cancel, true);
    } else if let Some(review) = workspace.trust_review() {
        b.text(
            message(TextKey::TrustReview),
            &trust_lines(review, usize::MAX / 4)
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            trust_content(main, grid).0,
        );
        b.button(message(TextKey::TrustApprove), allow, !review.submitted && pending == 0);
        b.button(message(TextKey::ActionCancel), cancel, !review.submitted);
    } else if let Some(form) = workspace.commit_composer() {
        let geometry = commit_geometry(width, height, grid);
        b.input(
            message(TextKey::CommitMessage),
            &form.editor.text,
            geometry.message,
            !workspace.prompt().is_some_and(|p| p.pending()) && form.editor.preedit.is_empty(),
            form.focus == CommitFocus::Message,
        );
        let ready = form.review.is_some()
            && !form.review.as_ref().is_some_and(|r| r.blocked.is_some())
            && form.diff_ready
            && !form.editor.text.trim().is_empty()
            && form.editor.text.len() <= composer::PASTE_LIMIT_BYTES
            && form.editor.preedit.is_empty()
            && !workspace.prompt().is_some_and(|p| p.pending());
        b.button(message(TextKey::CommitApprove), geometry.commit, ready && pending == 0);
        b.button(message(TextKey::CommitRefresh), geometry.refresh, pending == 0);
        b.button(message(TextKey::CommitClose), geometry.close, true);
        b.text(
            message(TextKey::CommitIndex),
            &form
                .review
                .as_ref()
                .map(|r| {
                    let inner = inner_of(geometry.index, grid);
                    let rows = (inner.rows - COMMIT_CONTEXT_ROWS).max(1);
                    let offset = window_start(form.selected, r.files.len(), rows);
                    let end = (offset + rows as usize).min(r.files.len());
                    let heading = format!(
                        "{}\n{} paths | unfiltered | visible {}..{}",
                        r.head,
                        r.files.len(),
                        if r.files.is_empty() { 0 } else { offset + 1 },
                        end,
                    );
                    std::iter::once(heading)
                        .chain(
                            r.files
                                .iter()
                                .skip(offset)
                                .take(rows as usize)
                                .map(|f| format!("{} {}", f.change, f.path.display_label())),
                        )
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "Loading review".into()),
            geometry.index,
        );
        let inner = inner_of(geometry.diff, grid);
        b.text(
            message(TextKey::CommitDiff),
            &form
                .diff
                .lines
                .iter()
                .skip(form.diff_scroll)
                .take(inner.rows.max(0) as usize)
                .map(|l| fit(&l.text, inner.cols))
                .collect::<Vec<_>>()
                .join("\n"),
            geometry.diff,
        );
    } else if let Some(form) = workspace.create_form() {
        if let Some((_, lines)) = &form.review {
            b.text(
                message(TextKey::CreateReviewTitle),
                &lines.join("\n"),
                trust_content(main, grid).0,
            );
        } else {
            let (path, branch) = create_fields(main, grid);
            b.input(
                message(TextKey::CreateDirectory),
                &form.path,
                path,
                !form.submitted && !workspace.entry_composing(),
                !form.branch_focus,
            );
            b.input(
                message(TextKey::CreateBranch),
                &form.branch,
                branch,
                !form.submitted && !workspace.entry_composing(),
                form.branch_focus,
            );
        }
        let reviewing = form.review.is_some();
        b.button(
            if reviewing {
                message(TextKey::CreateApprove)
            } else {
                message(TextKey::CreateReview)
            },
            allow,
            !form.submitted
                && !workspace.entry_composing()
                && pending == 0
                && (reviewing || (!form.path.is_empty() && !form.branch.is_empty())),
        );
        b.button(
            if reviewing { message(TextKey::ActionBack) } else { message(TextKey::ActionCancel) },
            cancel,
            !form.submitted && !workspace.entry_composing(),
        );
    } else if let Some(form) = workspace.identity_form() {
        let (name, email) = create_fields(main, grid);
        b.input(
            message(TextKey::IdentityName),
            &form.name,
            name,
            !form.submitted && !workspace.entry_composing(),
            !form.email_focus,
        );
        b.input(
            message(TextKey::IdentityEmail),
            &form.email,
            email,
            !form.submitted && !workspace.entry_composing(),
            form.email_focus,
        );
        b.text(
            message(TextKey::IdentitySources),
            &form.detail,
            identity_info(main, grid),
        );
        b.button(
            if form.ticket.is_none() {
                message(TextKey::IdentityReload)
            } else {
                message(TextKey::IdentitySave)
            },
            allow,
            !form.submitted && !workspace.entry_composing() && pending == 0,
        );
        b.button(
            message(TextKey::ActionCancel),
            cancel,
            !form.submitted && !workspace.entry_composing() || form.ticket.is_none(),
        );
    } else if let Some(prompt) = workspace.prompt().filter(|p| {
        matches!(
            p.kind(),
            PromptKind::OpenRepository | PromptKind::GitSettings
        )
    }) {
        let body = trust_content(main, grid).0;
        let field = create_fields(body, grid).0;
        let label = match prompt.kind() {
            PromptKind::OpenRepository => message(TextKey::EntryRepositoryPath),
            PromptKind::GitSettings => "Git executable",
            PromptKind::NewBranch => "Branch name",
            PromptKind::Commit => message(TextKey::CommitMessage),
        };
        b.input(
            label,
            prompt.text(),
            field,
            !prompt.pending() && !workspace.entry_composing(),
            true,
        );
        if prompt.kind() == PromptKind::OpenRepository
            && workspace.is_welcome()
            && !workspace.snapshot().recents.is_empty()
        {
            for (rect, name) in
                recent_controls(main, grid)
                    .into_iter()
                    .zip([message(TextKey::ActionOpen), message(TextKey::RecentLocate), message(TextKey::ActionBack)])
            {
                b.button(
                    name,
                    rect,
                    !prompt.pending() && !workspace.entry_composing(),
                );
            }
        } else {
            b.button(
                if prompt.kind() == PromptKind::GitSettings {
                    message(TextKey::EntryTestSave)
                } else {
                    message(TextKey::ActionOpen)
                },
                allow,
                !prompt.pending() && !workspace.entry_composing() && pending == 0,
            );
            b.button(
                message(TextKey::ActionBack),
                cancel,
                !prompt.pending() && !workspace.entry_composing(),
            );
        }
    } else if workspace.modal() {
        // Other forms are named/readable while their field/action semantics are
        // being connected; never advertise an invented actionable control.
        let draws = super::scene(workspace, width, height, grid, pending);
        let text = draws
            .iter()
            .filter_map(|d| {
                if let Draw::Text { text, .. } = d {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        b.text("Dialog content",&format!("{text}\nUse this dialog's keyboard controls; assistive field editing is unavailable in this view."),main);
    } else {
        for (name, slots, cells) in [
            (message(TextKey::ActionOpen), 27, 8),
            (message(TextKey::EntryGitSettings), 6, 6),
            (
                if workspace.is_welcome() {
                    message(TextKey::EntryCreate)
                } else {
                    message(TextKey::EntryIdentitySettings)
                },
                17,
                10,
            ),
            ("Tools", 51, 10),
        ] {
            b.button(
                name,
                Rect {
                    x: width - grid.margin - (grid.advance * slots as f32) as i64,
                    y: grid.margin,
                    width: (grid.advance * cells as f32) as i64,
                    height: grid.row.max(2 * grid.margin),
                },
                pending == 0,
            );
        }
        if workspace.is_welcome() {
            let recents = Rect {
                x: grid.margin,
                y: main.y,
                width: panels.files.width,
                height: main.height,
            };
            let inner = inner_of(recents, grid);
            let rows = inner.rows.max(0) as usize;
            let offset = window_start(
                workspace.selected(Pane::Files),
                workspace.snapshot().recents.len(),
                inner.rows,
            );
            for (row, item) in workspace
                .snapshot()
                .recents
                .iter()
                .skip(offset)
                .take(rows)
                .enumerate()
            {
                b.button(
                    &item.label,
                    Rect {
                        x: inner.x,
                        y: inner.top + row as i64 * grid.row,
                        width: recents.width - 2 * grid.gap,
                        height: grid.row,
                    },
                    true,
                );
            }
            if !workspace.snapshot().recents.is_empty() {
                for (rect, name) in recent_controls(main, grid).into_iter().zip([
                    message(TextKey::RecentOpen),
                    "Locate selected",
                    "Remove selected",
                ]) {
                    b.button(name, rect, pending == 0);
                }
            }
        } else {
            b.text(
                "Repository status",
                &workspace.snapshot().head,
                panels.status,
            );
            for (i, rect) in tab_rectangles(workspace, width, grid) {
                b.button(
                    &format!("Repository {}", workspace.tabs()[i].path.display_label()),
                    rect,
                    true,
                );
            }
            for (rect, name) in tab_controls(width, grid).into_iter().zip([
                "Previous repository",
                "Next repository",
                "Close repository",
            ]) {
                b.button(name, rect, true);
            }
            b.button(
                message(TextKey::CommitPrepare),
                Rect {
                    x: width - grid.margin - (grid.advance * 39.0) as i64,
                    y: grid.margin,
                    width: (grid.advance * 10.0) as i64,
                    height: grid.row.max(2 * grid.margin),
                },
                pending == 0,
            );
            for (pane, rect) in [
                (
                    if workspace.stash_list_visible() {
                        Pane::Stashes
                    } else {
                        Pane::Files
                    },
                    if workspace.stash_list_visible() {
                        panels.files
                    } else {
                        file_list_body(panels.files, grid)
                    },
                ),
                (Pane::Branches, panels.branches),
                (Pane::Commits, panels.commits),
            ] {
                let inner = filtered_inner_of(rect, grid);
                let visible = workspace.visible_rows(pane);
                let selected = workspace.selected(pane);
                let position = visible.iter().position(|i| *i == selected).unwrap_or(0);
                let offset = window_start(position, visible.len(), inner.rows);
                for (row, index) in visible
                    .iter()
                    .skip(offset)
                    .take(inner.rows.max(0) as usize)
                    .enumerate()
                {
                    let label = match pane {
                        Pane::Files => &workspace.snapshot().files[*index].label,
                        Pane::Branches => &workspace.snapshot().branches[*index].label,
                        Pane::Commits => &workspace.snapshot().commits[*index].subject,
                        Pane::Stashes => &workspace.snapshot().stashes[*index].subject,
                        _ => unreachable!(),
                    };
                    b.node(
                        label,
                        "",
                        Rect {
                            x: inner.x,
                            y: inner.top + row as i64 * grid.row,
                            width: rect.width - 2 * grid.gap,
                            height: grid.row,
                        },
                        PublishedRole::Button,
                        Flags {
                            enabled: true,
                            focused: workspace.focus() == pane && selected == *index,
                            focusable: true,
                            clickable: true,
                            ..Flags::default()
                        },
                    );
                }
            }
            if !workspace.stash_list_visible() {
                for (rect, action, name) in file_actions(panels.files, grid) {
                    b.button(
                        name,
                        rect,
                        workspace.change_available(action) && pending == 0,
                    );
                }
            }
            if workspace.diff_actions_visible() {
                for (rect, action, name) in diff_sides(workspace, main, grid) {
                    b.button(
                        name,
                        rect,
                        workspace.change_available(action) && pending == 0,
                    );
                }
            }
            let inner = inner_of(diff_body(workspace, main, grid), grid);
            let text = workspace
                .main()
                .lines
                .iter()
                .skip(workspace.scroll())
                .take(inner.rows.max(0) as usize)
                .map(|l| fit(&l.text, inner.cols))
                .collect::<Vec<_>>()
                .join("\n");
            b.text(
                message(TextKey::ChangesVisibleDiff),
                &text,
                diff_body(workspace, main, grid),
            );
        }
    }
    let status = workspace
        .notice()
        .or_else(|| {
            if workspace.confirm().is_none() {
                workspace.snapshot().stage_result.as_deref()
            } else {
                None
            }
        })
        .unwrap_or("");
    b.text(
        message(TextKey::CurrentActionStatus),
        status,
        Rect {
            x: grid.margin,
            y: panels.footer_top,
            width: width - 2 * grid.margin,
            height: grid.row,
        },
    );
    b.scene
}

impl StatusWindow {
    pub(super) fn invalidate_accessibility(&mut self) {
        self.accessible_scene = None;
        if let Some(publisher) = &self.accessibility {
            publisher.invalidate();
        }
    }
    pub(super) fn publish_accessibility(&mut self, _window: &PixelWindow, surface: Surface) {
        let Some(publisher) = &self.accessibility else {
            return;
        };
        let next = build(&self.workspace, surface, self.pending + self.queued.len());
        if self.accessible_scene.as_ref() == Some(&next) {
            return;
        }
        self.accessibility_revision = self.accessibility_revision.saturating_add(1);
        let snapshot = AccessibleSnapshot {
            revision: self.accessibility_revision,
            context: self.workspace.accessibility_context(),
            tree: next.tree.clone(),
            disabled: next.disabled.clone(),
        };
        match publisher.publish(snapshot) {
            Ok(()) => self.accessible_scene = Some(next),
            Err(error) => {
                publisher.invalidate();
                self.accessible_scene = None;
                let _ = self.workspace.reply(Reply::Notice(format!(
                    "Accessibility view unavailable: {error}"
                )));
            }
        }
    }
    pub(super) fn accessibility_requests(
        &mut self,
        window: &PixelWindow,
    ) -> Result<(), PixelWindowError> {
        let requests = self
            .accessibility
            .as_ref()
            .map_or_else(Vec::new, |p| p.requests());
        for request in requests {
            if request.context != self.workspace.accessibility_context() {
                continue;
            }
            let Some(surface) = self.surface else {
                continue;
            };
            let current = build(&self.workspace, surface, self.pending + self.queued.len());
            if current.disabled.contains(&request.node)
                || !current
                    .tree
                    .node(request.node)
                    .is_some_and(|n| n.name == request.name)
            {
                continue;
            }
            let Some(binding) = current.bindings.get(&request.node).cloned() else {
                continue;
            };
            let context = self.workspace.accessibility_context();
            let invokes = matches!(&request.action, Action::Invoke);
            let scale = window.scale_factor()?;
            let point = Some((
                (binding.rect.x + binding.rect.width / 2) as f64 / scale,
                (binding.rect.y + binding.rect.height / 2) as f64 / scale,
            ));
            if let Some(publisher) = &self.accessibility {
                publisher.focus();
            }
            self.pointer_gesture(window, point, Gesture::Click)?;
            if let Action::SetValue(value) = request.action
                && binding.editable
            {
                let effect = self.workspace.replace_editable_text(value);
                self.dispatch(effect);
            }
            if invokes || context != self.workspace.accessibility_context() {
                self.invalidate_accessibility();
            }
            self.sync_title(window);
            window.request_redraw();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface() -> Surface {
        Surface {
            width: 1040,
            height: 680,
            grid: super::super::tests::GRID,
        }
    }
    fn enabled(scene: &Scene, name: &str) -> bool {
        let node = scene.tree.nodes.iter().find(|n| n.name == name).unwrap();
        !scene.disabled.contains(&node.id)
    }
    #[test]
    fn create_semantics_require_review_and_revoke_old_form_and_review_contexts() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        workspace.request_create();
        let original = workspace.accessibility_context();
        assert!(!enabled(
            &build(&workspace, surface(), 0),
            "Review creation"
        ));
        workspace.replace_editable_text("target 中文".into());
        workspace.focus_create_field(true);
        workspace.replace_editable_text("feature/native".into());
        workspace.replace_editable_text("x".repeat(256));
        assert_eq!(workspace.create_form().unwrap().branch, "feature/native");
        let scene = build(&workspace, surface(), 0);
        assert!(enabled(&scene, "Review creation"));
        assert!(
            scene
                .tree
                .nodes
                .iter()
                .any(|n| n.name == "Initial branch" && n.editable && n.text == "feature/native")
        );
        workspace.ime_preedit("未完成".into());
        let composing = build(&workspace, surface(), 0);
        assert!(!enabled(&composing, "Review creation"));
        assert!(!enabled(&composing, "Initial branch"));
        workspace.ime_preedit(String::new());
        assert_eq!(
            workspace.key(Key::Enter),
            Effect::Send(Job::PrepareCreate {
                path: NativePath::new("target 中文"),
                branch: "feature/native".into()
            })
        );
        assert!(!enabled(
            &build(&workspace, surface(), 1),
            "Review creation"
        ));
        workspace.replace_editable_text("late value".into());
        assert_eq!(workspace.create_form().unwrap().branch, "feature/native");
        workspace.reply(Reply::CreateReview {
            ticket: "create-ticket".into(),
            path: NativePath::new("target 中文"),
            branch: "feature/native".into(),
            new_directory: true,
            entries: vec![],
            more_entries: false,
        });
        let reviewed = workspace.accessibility_context();
        assert_ne!(original, reviewed);
        let scene = build(&workspace, surface(), 0);
        assert!(enabled(&scene, "Create repository"));
        assert!(!scene.tree.nodes.iter().any(|n| n.editable));
        assert_eq!(
            workspace.key(Key::Enter),
            Effect::Nothing,
            "text submission never approves creation"
        );
        assert_eq!(
            workspace.answer_create(false),
            Effect::Send(Job::CancelEntry)
        );
        assert_ne!(reviewed, workspace.accessibility_context());
        workspace.key(Key::Escape);
        workspace.request_create();
        assert_ne!(original, workspace.accessibility_context());
    }
    #[test]
    fn identity_semantics_follow_read_save_and_field_limits_without_implicit_submission() {
        let mut workspace = Workspace::new(super::super::tests::snapshot(1));
        assert_eq!(
            workspace.request_identity(),
            Effect::Send(Job::LoadIdentity)
        );
        let original = workspace.accessibility_context();
        let scene = build(&workspace, surface(), 1);
        assert!(!enabled(&scene, "Name"));
        assert!(
            enabled(&scene, "Cancel"),
            "an identity read can be cancelled"
        );
        workspace.replace_editable_text("late read edit".into());
        workspace.reply(Reply::IdentitySettings {
            ticket: "identity-ticket".into(),
            name: "old".into(),
            email: "old@example.invalid".into(),
            detail: "local scope".into(),
        });
        workspace.replace_editable_text("New 中文".into());
        workspace.focus_identity_field(true);
        workspace.replace_editable_text("new@example.invalid".into());
        workspace.replace_editable_text("bad\naddress".into());
        workspace.replace_editable_text("x".repeat(1025));
        let scene = build(&workspace, surface(), 0);
        assert!(enabled(&scene, "Save local"));
        assert!(
            scene
                .tree
                .nodes
                .iter()
                .any(|n| n.name == "Name" && n.text == "New 中文")
        );
        workspace.ime_preedit("未完成".into());
        let composing = build(&workspace, surface(), 0);
        assert!(!enabled(&composing, "Save local"));
        assert!(!enabled(&composing, "Email"));
        workspace.ime_preedit(String::new());
        assert_eq!(
            workspace.key(Key::Enter),
            Effect::Send(Job::SaveIdentity {
                ticket: "identity-ticket".into(),
                name: "New 中文".into(),
                email: "new@example.invalid".into()
            })
        );
        let scene = build(&workspace, surface(), 1);
        assert!(!enabled(&scene, "Save local"));
        assert!(!enabled(&scene, "Cancel"));
        workspace.replace_editable_text("late save edit".into());
        assert_eq!(
            workspace.identity_form().unwrap().email,
            "new@example.invalid"
        );
        workspace.reply(Reply::PromptFailed {
            action: PromptAction::IdentitySettings,
            text: "save refused".into(),
        });
        workspace.key(Key::Escape);
        workspace.request_identity();
        assert_ne!(original, workspace.accessibility_context());
    }

    #[test]
    fn large_index_publication_tracks_the_viewport_without_copying_hidden_paths() {
        let mut workspace = Workspace::new(super::super::tests::snapshot(1));
        let Effect::Send(Job::LoadCommitReview { generation }) = workspace.request_commit() else {
            panic!("commit review must load");
        };
        let count = 20_000;
        workspace.reply(Reply::CommitReview {
            generation,
            review: crate::CommitReview {
                ticket: "large-index".into(),
                head: "main | ordinary commit".into(),
                files: (0..count)
                    .map(|index| crate::StagedRow {
                        path: NativePath::new(format!("{index:05}-{}.txt", "中文-path".repeat(12))),
                        change: "M".into(),
                        added: Some(1),
                        deleted: Some(1),
                    })
                    .collect(),
                blocked: None,
            },
        });
        let surface = Surface {
            width: 1040,
            height: 680,
            grid: super::super::tests::GRID,
        };
        for last in [false, true] {
            if last {
                workspace.key(Key::Tab);
                workspace.key(Key::End);
            }
            let scene = build(&workspace, surface, 0);
            let text = &scene
                .tree
                .nodes
                .iter()
                .find(|n| n.name == "Complete staged index")
                .unwrap()
                .text;
            assert!(
                text.len() < 16_384,
                "hidden paths must not be copied into each frame"
            );
            assert!(text.contains("20000 paths | unfiltered | visible"));
            assert!(text.contains(if last { "19999-" } else { "00000-" }));
            assert!(!text.contains(if last { "00000-" } else { "19999-" }));
            AccessibleSnapshot {
                revision: 1,
                context: workspace.accessibility_context(),
                tree: scene.tree,
                disabled: scene.disabled,
            }
            .validate()
            .unwrap();
        }
    }

    #[test]
    fn semantic_review_uses_real_guards_and_omits_background_actions() {
        let mut workspace = Workspace::new(super::super::tests::snapshot(1));
        let Effect::Send(Job::Prepare { generation, .. }) = workspace.change(ChangesAction::Ignore)
        else {
            panic!()
        };
        let surface = Surface {
            width: 1040,
            height: 680,
            grid: super::super::tests::GRID,
        };
        let scene = build(&workspace, surface, 0);
        let save = scene
            .tree
            .nodes
            .iter()
            .find(|n| n.name == "Save rule")
            .unwrap();
        assert!(scene.disabled.contains(&save.id));
        assert!(!scene.tree.nodes.iter().any(|n| n.name == "Prepare commit"));
        workspace.reply(Reply::Prepared {
            generation,
            ticket: "reviewed".into(),
            lines: vec!["exact target".into()],
        });
        let scene = build(&workspace, surface, 0);
        let save = scene
            .tree
            .nodes
            .iter()
            .find(|n| n.name == "Save rule")
            .unwrap();
        assert!(!scene.disabled.contains(&save.id));
        assert!(
            scene
                .tree
                .nodes
                .iter()
                .any(|n| n.text.contains("exact target"))
        );
        AccessibleSnapshot {
            revision: 1,
            context: workspace.accessibility_context(),
            tree: scene.tree,
            disabled: scene.disabled,
        }
        .validate()
        .unwrap();
    }
}
