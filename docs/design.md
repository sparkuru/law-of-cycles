# law-of-cycles / kami

## Product boundary

`kami` is a Linux terminal control surface for Mihomo. Target environments are
x86_64 and ARM64 distributions with systemd. The core owns packet forwarding,
DNS and routing; systemd owns process lifetime; kami owns user interaction.
Exiting the TUI never stops the core.

The initial release connects to an existing controller and provides:

- Overview: core version, mode, TUN status and live traffic.
- Proxies: search groups and members, choose a node, measure delay.
- Connections: stable selection by connection ID and explicit disconnect.
- Logs: bounded live history, search and reconnection.
- CLI equivalents for automation, with JSON output.
- Local systemd status, start/stop/restart and enable/disable commands.
- Runtime TUN switching and command-scoped proxy environment variables.

Subscription conversion, core downloads, desktop proxy integration, persistent
YAML rewriting and automatic network repair are separate future work. Updating
an existing provider is supported; it is not subscription management.

## Interaction contract

Bare `kami` prints help. Connection flags or explicit `tui` open the TUI when
a controller is configured; `tui --demo` uses sample data without one.
Other named subcommands perform one operation. A controller URL comes from a
flag, environment variable or user configuration file; a secret comes from the
environment or file when required. Secrets must not appear in the UI,
diagnostic messages or command history examples.

The four TUI pages share a connection state. Network work runs outside the input
loop. Failed reads retain the previous snapshot and label it stale. Streams
reconnect with bounded backoff. Lists preserve selection by identity, not row
number. Search never changes the underlying selection silently during a
destructive confirmation. Logs have a fixed maximum size.

Mode and TUN changes affect the running core only. TUN controls only the
`enable` field; DNS, routing and exclusions remain the core configuration's
responsibility. A successful PATCH is followed by a readback; API state alone
does not prove that packets traverse the expected route.

Connection deletion and TUN changes require confirmation in the TUI. Explicit
CLI commands are the corresponding intent. No service or network mutations
occur at startup. Local service control is distinct from the possibly remote
controller; no remote API setting is interpreted as a local service target.

## Boundaries and failure handling

1. Transport: bounded HTTP requests, bearer authentication, escaped path
   segments, HTTP JSON-line streams, no ambient proxy or redirect forwarding.
2. Operations: proxy membership validation, runtime configuration readback,
   connection/provider actions and shared error semantics.
3. Linux integration: fixed systemctl actions, argument arrays without a shell,
explicit privilege escalation by the user, child-only proxy environment.
4. Presentation: CLI formatting and TUI state; neither owns kernel lifetime.

Controller errors must not echo arbitrary authenticated request data. The
default controller binds to localhost. Remote plaintext HTTP is an explicit
user configuration choice; HTTPS uses normal certificate verification.
Configuration examples contain no real credentials.

Service operations should use existing system authorization. The UI must not
collect a sudo password. A supplied service unit is a reviewable installation
artifact, not something installed automatically on startup.

## Validation

Use a local fake HTTP controller to verify authentication, escaping, 204
responses, errors, streams, runtime changes and CLI exit codes. Exercise UI
state independently of the terminal and smoke-test startup/exit in a pseudo-terminal.
Validate systemctl argument construction and child execution; do not alter the development host's network
or services. Real TUN behavior and ARM64 execution require separate target
validation and must not be claimed from mocked tests.

## References

- https://wiki.metacubex.one/api/
- https://wiki.metacubex.one/startup/service/
- https://wiki.metacubex.one/config/inbound/tun/

Scope recovered from Codex session `01a0c318-d163-7c82-b3f0-50428d155fae`.

## Rust architecture (0.2)

The implementation is Rust 1.90+, Ratatui 0.30, Crossterm 0.29 and Tokio.
The previous Python/curses prototype has been replaced; the CLI command names
and controller configuration format are retained.

```text
Crossterm key / mouse / resize
             |
         App + Intent -> Effect -> Tokio task -> Backend -> Mihomo
             ^                         |
             +---------- Update -------+
             |
         Ratatui widgets -> terminal cells + Hit rectangles
```

- `config.rs`: validated TOML or Mihomo YAML/environment/flag settings; YAML
  contributes only controller listeners and the secret. Wildcard listeners map
  to loopback, TLS takes precedence, and source excerpts never enter errors.
- `model.rs`: typed snapshots, operations, topics and updates shared by both UIs.
- `backend.rs`: Reqwest transport, Mihomo operations, stream framing/reconnect,
  and an in-memory demo using the same operation contract.
- `system.rs`: local systemctl argument construction and process proxy environment.
- `cli.rs`: Clap commands and human/JSON output; reuses Backend operations.
- `app.rs`: UI state, stable object identities, menus, confirmations and intents;
  independent of terminal and network I/O.
- `ui.rs`: Ratatui layouts, tables, inspector and popups. Rendering also records
  matching mouse rectangles. Table scrolling and Unicode cells use Ratatui.
- `tui.rs`: terminal lifetime guard, Crossterm events, bounded update channel,
  independent polling/stream tasks and operation dispatch.

The event loop selects between input, updates and a 50 ms redraw tick. Network
work never blocks input; redraws occur on input or changed data. Three snapshot
tasks poll independently; two stream tasks reconnect independently. Refresh uses
a watch channel, while updates use a 128-item bounded channel. Log history is
limited to 500 entries. Exiting aborts tasks and restores raw mode, mouse capture
and the alternate screen; a panic hook also restores the terminal.

The mouse/keyboard design and Herdr references are in
[interaction.md](interaction.md). kami borrows explicit focus, visible targets
and contextual actions; it does not need Herdr's PTY server or pane split tree.
