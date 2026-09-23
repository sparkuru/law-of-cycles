# Interaction model: learning from Herdr

## What makes Herdr navigable

Herdr organizes workspaces, tabs and real terminal panes. Its visible sidebar
and pane focus give actions an explicit target. Mouse users can select objects,
open context menus, and resize split boundaries. Keyboard users reach actions
through bindings and interaction modes. Status rolls up into the sidebar so
attention does not require opening every pane.

The examined upstream implementation uses Rust, Ratatui, Crossterm and
portable-pty. Its layout module models pane identities, rectangles, focus and
split ratios. The UI computes pane view geometry; mouse code translates input
coordinates, and keyboard code resolves bindings into actions. The background
server owns terminals and processes independently of attached UI clients.

Primary references, inspected 2026-09-23 (upstream master can change):

- [Interaction and mouse controls](https://herdr.dev/docs/quick-start/)
- [Workspace, pane, mode and server model](https://herdr.dev/docs/concepts/)
- [Rust dependencies](https://github.com/herdrdev/herdr/blob/master/Cargo.toml)
- [Pane identities, geometry, focus and split tree](https://github.com/herdrdev/herdr/blob/master/src/layout.rs)
- [Computed view state](https://github.com/herdrdev/herdr/blob/master/src/ui.rs)
- [Mouse coordinates](https://github.com/herdrdev/herdr/blob/master/src/input/mouse.rs)
- [Keyboard dispatch exports](https://github.com/herdrdev/herdr/blob/master/src/input/mod.rs)

## Applying that model to kami

kami now uses Rust with Ratatui/Crossterm and independently implements an
object-first interaction model. Mihomo remains the independent core, so kami
does not need to own PTYs, spawn agents or reproduce a multiplexer server.

| User question | Visible answer |
| --- | --- |
| Where am I? | Highlighted sidebar page and titled panel |
| Which object am I operating on? | Selected row plus inspector |
| What can I do? | Action button and contextual menu |
| Will clicking this node change traffic? | Single click only selects; Use node changes it |
| Why is an action unavailable? | Disabled menu entry with a reason |
| What will be disconnected? | Confirmation captures and displays the original connection ID |
| Did the operation finish? | Persistent success/failure message; click for full result |
| Is this live data? | Header distinguishes online, stale, connecting and demo states |

The dark interface uses muted violet for focus, pink for the brand and primary
content, and green/amber/red for status. The header centers 円環の理 between
Law of Cycles and the controller state; the controller address belongs in
Overview. A focus strip names NAVIGATION, CONTENT or SEARCH. The focused
area is highlighted and inactive selections are dimmed; search has a visible cursor. In proxy groups, ● identifies the active node and › identifies
the cursor independently.

Wide terminals show the proxy list and inspector side by side; drag their
separator to change the split ratio. Connections always put selected details
below the full-width list, reducing to a compact summary when space is limited.
Logs use full width. Details popups retain access to complete information, and
narrow terminals replace the sidebar with a top navigation strip. Double-click
a proxy group to open it; double-clicking a node never changes traffic. Menus and popups fit within
the current terminal, and list selections survive ordinary snapshot refreshes.

The overview exposes explicit mode and TUN controls plus a local service menu.
Runtime mode selection is a choice among named options, not a blind cycle.
TUN and service mutations and connection disconnects use confirmation dialogs.
Cancel is initially selected. Overlays exclusively consume input, so a click
cannot pass through to a background control.

Group-level **Refresh group** and log **Clean** are always placed beside the
page title. Refresh measures every group member with bounded concurrency and
reports partial failures without selecting a node. Clean clears local history
without restarting or stopping the stream. Connections omit process information
unless `[connections] show_process = true` is set in the kami TOML.

Search owns keyboard input until Esc: Ctrl+C clears the input, q inserts a
character, and Enter/Tab keep editing. Esc returns to content while preserving
the filter. Global quit keys are handled only outside search.

## Implementation boundaries

```text
HTTP / JSON-line streams -> Tokio tasks -> Update -> App
                                                    |
mouse coordinates -> rendered Hit -> Intent --------+-> Effect -> Backend
keyboard shortcuts -----------------+               |
                                                    v
                                             Ratatui widgets
                                       terminal cells + Hit rectangles
```

- `app.rs`: snapshots, bounded logs, stable selection identities, focus, menus
  and captured confirmation targets. Keyboard and mouse share Intent dispatch.
- `ui.rs`: Ratatui Layout/Rect, Table/TableState, List and Paragraph render the
  panels, inspector and overlays. Each interactive region emits a matching Hit.
- `tui.rs`: Crossterm EventStream and Tokio select loop; tasks exchange typed
  updates through a bounded channel. Terminal ownership has a cleanup guard.
- `backend.rs`: shared controller operations and in-memory demo; demo services
  are disabled and controller operations do not open sockets.

This iteration provides one resizable list/inspector split, not arbitrary
terminal tiling, persistent layouts, multiple controllers or an agent runtime.
The data refresh loop and existing CLI contracts remain in place.

## Validation

Regression coverage includes row selection versus mutation, context menu
target capture, explicit mode selection, disabled automatic groups, default
cancel, modal input isolation, click confirmation, split dragging, responsive
geometry, and demo service isolation. A pseudo-terminal test sends real SGR
mouse input to the native Rust process and checks terminal restoration on exit.
The same renderer can be exercised with Ratatui TestBackend to inspect layouts.
Real terminal emulators may differ in mouse capture and modified-click handling;
keyboard equivalents do not depend on those features.
