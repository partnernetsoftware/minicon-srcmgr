//! Product semantics reuse the actual draw geometry and pointer action path.
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
            "Action review",
            &text,
            confirmation_body(main, grid, matches!(confirm, Confirm::Ignore { .. })),
        );
        let ignore = matches!(confirm, Confirm::Ignore { .. });
        if ignore {
            for (rect, name) in ignore_scope_buttons(main, grid)
                .into_iter()
                .zip(["Project .gitignore", "Local exclude"])
            {
                b.button(name, rect, pending == 0);
            }
        }
        b.button(
            if ignore {
                "Save rule"
            } else {
                "Confirm reviewed action"
            },
            allow,
            workspace.confirmation_ready() && pending == 0,
        );
        b.button("Cancel", cancel, true);
    } else if let Some(review) = workspace.trust_review() {
        b.text(
            "Repository trust review",
            &trust_lines(review, usize::MAX / 4)
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            trust_content(main, grid).0,
        );
        b.button("Trust and open", allow, !review.submitted && pending == 0);
        b.button("Cancel", cancel, !review.submitted);
    } else if let Some(form) = workspace.commit_composer() {
        let geometry = commit_geometry(width, height, grid);
        b.input(
            "Commit message",
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
        b.button("Commit", geometry.commit, ready && pending == 0);
        b.button("Refresh review", geometry.refresh, pending == 0);
        b.button("Save and close", geometry.close, true);
        b.text(
            "Complete staged index",
            &form
                .review
                .as_ref()
                .map(|r| {
                    r.files
                        .iter()
                        .map(|f| format!("{} {}", f.change, f.path.display_label()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "Loading review".into()),
            geometry.index,
        );
        let inner = inner_of(geometry.diff, grid);
        b.text(
            "Staged Diff",
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
    } else if let Some(prompt) = workspace.prompt().filter(|p| {
        matches!(
            p.kind(),
            PromptKind::OpenRepository | PromptKind::GitSettings
        )
    }) {
        let body = trust_content(main, grid).0;
        let field = create_fields(body, grid).0;
        let label = match prompt.kind() {
            PromptKind::OpenRepository => "Repository path",
            PromptKind::GitSettings => "Git executable",
            PromptKind::NewBranch => "Branch name",
            PromptKind::Commit => "Commit message",
        };
        b.input(label, prompt.text(), field, !prompt.pending(), true);
        if prompt.kind() == PromptKind::OpenRepository
            && workspace.is_welcome()
            && !workspace.snapshot().recents.is_empty()
        {
            for (rect, name) in
                recent_controls(main, grid)
                    .into_iter()
                    .zip(["Open", "Locate recent", "Back"])
            {
                b.button(name, rect, !prompt.pending());
            }
        } else {
            b.button(
                if prompt.kind() == PromptKind::GitSettings {
                    "Test and save"
                } else {
                    "Open"
                },
                allow,
                !prompt.pending() && pending == 0,
            );
            b.button("Back", cancel, !prompt.pending());
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
            ("Open", 27, 8),
            ("Git settings", 6, 6),
            (
                if workspace.is_welcome() {
                    "Create"
                } else {
                    "Identity settings"
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
                    "Open selected",
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
                "Prepare commit",
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
                "Selected Diff visible content",
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
        "Current action status",
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
