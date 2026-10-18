# 10 · Desktop interaction and MiniCon design

Parent: [PRD index](../PRD.md). Owns visual language, layout, interaction, localization and accessibility. Reuse source status is owned by [02](PRD_02_architecture_reuse.md).

## Outcome tree

```text
Outcome: see repository context and act accurately with any supported input
├── Shell — compact panes, collapsible navigation and persistent status
├── Components — graph, file list, diff, forms and operation center
├── Simplicity — contextual actions, progressive detail and direct task paths
├── Design — semantic themes, DPI and readable type
└── Access — keyboard, IME, screen readers and three UI languages
```

```mermaid
flowchart TD
    T["Semantic theme + DIP layout"] --> V["Rendered controls"]
    M["Domain snapshots"] --> V & A["Semantic accessibility tree"]
    K["Keyboard / pointer / IME / accessibility action"] --> C["Shared action router"]
    V & A --> C --> G["Same operation gate"]
    G --> M
```

## Information architecture

```text
┌ repository tabs / Open / Clone / command palette ────────────────────┐
│ repository + branch + operation context · Fetch / Pull / Push         │
├ navigation ───────┬ primary content ────────────┬ optional inspector ┤
│ favorites/repos  │ Changes: files + diff       │ commit metadata    │
│ branches/tags    │ History: graph + detail     │ selected operation │
│ remotes/stashes  │ Conflict: sides + result    │ contextual help    │
│ worktrees        │                            │                    │
├──────────────────┴────────────────────────────┴────────────────────┤
│ branch / counts / fetch freshness / job progress / diagnostics       │
└────────────────────────────────────────────────────────────────────┘
```

At 1280×800 DIP, navigation defaults to 224 DIP, adjustable 180–420 and collapsible to a 44 DIP rail. Main views divide file/graph lists from content with draggable splitters and explicit minimums. Optional inspector defaults to 280 DIP, adjustable 220–420. Toolbar rows are 36 DIP, dense list rows 28 DIP, status 24 DIP; pointer targets at least 24×24 DIP. These are SrcMgr defaults derived from MiniCon's compact shell, not imported private constants.

At 960×640 DIP, collapse the inspector first and stack detail below lists as needed; preserve active repository, action labels and selected content. Below this design size, use scrollable/stacked content and a resize hint, never inaccessible confirmation controls. Pane sizes are clamped after monitor/DPI changes and stored per view. Long branch/path labels truncate visually with accessible full text and copy action. No screen requires horizontal scrolling to reach its primary action.

Changes, History and Conflicts are primary views. Repository switching does not reset an unfinished message or operation context. Operations center lists running/completed/reconciling jobs; status opens its relevant item. All views define loading, empty, stale, busy, error, unsupported and permission/auth-needed states, with a concrete next action and retry. Toasts report completion but cannot be the sole place an error or recovery action exists.

## Simple, practical and intuitive workflows

The functional reference and milestone mapping are owned by [01](PRD_01_product.md). Translate its efficient panel navigation into a GUI with discoverable labeled actions, readable content previews and equivalent keyboard paths. Keep one clear primary action per task surface; put uncommon flags and history rewrites in contextual menus or an expandable Advanced section. Avoid duplicate dashboards, permanent expert controls, decorative metrics and modal chains for routine inspection. Safety confirmations remain explicit under 08.

Selecting a file immediately presents its diff and applicable actions. Routine Stage/Unstage takes one deliberate action from the selected content; Commit uses the visible composition panel; switching a clean branch uses one named action after target selection. The user does not need to memorize TUI keys or type raw Git commands. Keyboard help shows actions for the focused pane; Escape returns to the prior pane with selection/scroll intact. Disabled actions have a short reason and next step. No context menu with the entire Git command catalog.

Usability acceptance adds a task-path audit: from an already selected applicable item, inspect diff and stage/unstage each require at most one action; open the commit composer or branch switch action requires at most one action, excluding target selection, text entry and required safety confirmation. QA-J09 records pointer/keyboard action counts separately and compares task completion against the pinned lazygit reference checklist from 01. These are SrcMgr flow targets, not claims about lazygit speed. An advanced feature must justify its visible space by a concrete task; defer it or hide its detail when it does not help the current task.

Performance affects interaction design: virtualize long lists/diffs, reuse bounded glyph/layout caches, redraw only damage and suspend animation/redraw while idle or occluded. Do not preload every tab's history or instantiate hidden heavy panels. Backend-specific controls follow [15](PRD_15_vcs_backends.md); future extensibility must not clutter the Git-first screen. Measurable resource/latency budgets remain solely in 14.

## Visual system

Keep MiniCon theme IDs `neutral`, `docs`, `paper`, `dark` and semantic roles: canvas, sidebar, surface, panel, border, text, muted, accent, error. Neutral is the default. Add SrcMgr roles for added/deleted/modified/conflicted files, diff additions/deletions and graph lanes. Palette values must be verified against a published upstream revision when implemented; copying local uncommitted theme values is not a stable contract.

Use restrained flat surfaces, thin separators, visible focus rings and compact labeled controls. Red/green additions also have `+`/`−`, line markers and accessible text; graph selection has outline/weight, not color alone. Aim for 4.5:1 ordinary text contrast, 3:1 large text and meaningful control boundaries in every theme; verify actual rendered pixels. Support system high-contrast preferences without removing state indicators. Motion is minimal and honors reduced motion.

UI text uses the platform UI font with CJK fallback; diff/commit IDs use a configurable monospace stack. Default UI size 13 DIP, code 13 DIP; independent text zoom 80–200%. Layout and hit testing use the same DIP-to-pixel transforms and rounding. Test 100/125/150/200% and monitor transitions. Retained frames/damage redraw are mechanisms, not permission to omit repaint after occlusion, resizing or theme change.

## Input, language and accessibility

Support pointer, context menus and keyboard equivalently. Tab/Shift+Tab follows visible logical order; Escape closes the nearest dismissible surface and restores focus. The command palette exposes discoverable named actions with disabled reasons. Initial bindings: Ctrl+O Open, Ctrl+Shift+O Clone, Ctrl+P palette, Ctrl+1 Changes, Ctrl+2 History, Ctrl+Enter commit when commit input is focused, F5 refresh. macOS uses Command for primary shortcuts. No destructive one-key shortcut. Native text editing bindings take precedence while editing. Rebinding checks collisions and persists a profile.

IME composition cannot trigger Commit, palette actions or shortcuts until committed/cancelled; candidate windows follow the active caret. Text fields support selection, clipboard, undo/redo and multiline input. Diff navigation and selection use codepoint/grapheme-aware display without changing byte identity. Keyboard users can select hunks/lines and inspect their exact target before staging.

English and Simplified Chinese are required by M2; Traditional Chinese by M3. Locale defaults to system with explicit override, changes without data loss, and uses stable internal IDs. No string concatenation for translated grammar; operation labels/path tokens remain distinguishable. Dates/numbers localize while full OIDs and literal paths remain unchanged. Test long translations and CJK filenames; UI language does not change Git parsing.

Pixel rendering must expose native accessibility via UI Automation, macOS accessibility and Linux AT-SPI, directly or through a proven bridge. Required semantics: names/roles/states, focus/action routing, selected graph row, list virtual item navigation, editable text/caret/ranges, diff line descriptions and live progress announcements without flooding. Screen readers must complete the daily journey and conflict continuation. [02](PRD_02_architecture_reuse.md) gates feasibility; screenshots alone are insufficient evidence.

## Delivery contracts

| ID / leaf | Invariant | Observable evidence | Safe failure | Dependencies | Non-goal |
|---|---|---|---|---|---|
| UX-01 · shell | Active context and primary action remain reachable | QA-J09 minimum window, long labels and splitters | Stack/collapse with preserved focus | 03, 13 | Exact copy of terminal layout |
| UX-02 · visual | Theme/DPI transformations preserve legibility and hit targets | QA-J09 rendered contrast and mixed-DPI interaction | Repaint/clamp with readable fallback | 02 | Decorative animation |
| UX-03 · input | IME/text input cannot accidentally execute operations | QA-J09 composition, held keys and keyboard journeys | Keep draft/focus; explicit action | 04, 08 | Mouse-only operations |
| UX-04 · access/language | Assistive actions share real application semantics | QA-J09 screen reader per OS plus localization checks | Gate milestone when required bridge is missing | 02, 13 | Accessibility inferred from pixels |
| UX-05 · simplicity | Common actions have direct discoverable paths and advanced detail stays contextual | QA-J09 task-path counts, keyboard/pointer audit and reference checklist | Simplify failed flow before milestone exit | 01, 04–09, 15 | Expose every option permanently |
