# Using law-of-cycles (`kami`)

Requires Linux. Build with Rust 1.90+; the native executable needs no Python
runtime. x86_64 Linux is tested; ARM64 and real TUN networking still require
target-machine validation.

## Build and run

```console
cargo build --release --locked
./target/release/kami --help
./target/release/kami tui --demo
./target/release/kami --controller http://127.0.0.1:9090
```

If Rust is not installed, build through Docker:

```console
./hako cargo build --release --locked
./target/release/kami tui --demo
```

`hako` uses `rust:1.90-bookworm`, the current UID/GID and repository-local caches
under `.devhome/`. It publishes no ports and starts no persistent service.
The container's localhost and service manager are isolated: run the resulting
binary **on the host** for a host-local Mihomo controller or systemd operations.
The Linux binary requires a compatible glibc runtime (the build image uses 2.36).
Use a target-native build for other architectures.

Optionally copy `target/release/kami` into a directory on your PATH. The remaining
examples use `kami`; substitute `./target/release/kami` when running from the
repository. With no subcommand or explicit connection flags, kami prints help
(even when environment/default-file settings exist). Supplying `-c`/`--config`
or `--controller` without a subcommand opens the TUI when a
controller is configured.
Explicit `kami tui` uses environment/default-file settings, but needs a
controller from a flag, environment variable or file before opening the terminal.
`kami tui --demo` needs no controller. Noninteractive input requires a named CLI
command. Global flags may precede or follow the command.

## Controller

Connection settings are supplied **at startup**. With a controller configured
and no subcommand, kami opens the TUI and connects automatically. With a
subcommand such as `status`, it performs that operation and exits. The TUI has
no connection prompt or embedded shell; exit and restart with different
settings to switch controllers.

```console
kami --controller http://127.0.0.1:9090
kami --controller http://127.0.0.1:9090 status
```

Mihomo must already be running with `external-controller` configured. The TUI
does not assume a controller address; one-shot CLI commands retain the
`http://127.0.0.1:9090` default. Kami looks for `config.toml`, then
`config.yaml`, then `config.yml` under `~/.config/kami/` (or `$XDG_CONFIG_HOME/kami/`).
Copy `examples/config.toml` there if you need a kami TOML file, and set file
permissions to `0600` if it contains the controller secret. `-c PATH` or `--config PATH`
selects a different file. The file is never rewritten by kami.

Precedence: flags > `KAMI_CONTROLLER` / `KAMI_TIMEOUT` / `KAMI_SECRET` > configuration file
> defaults. A controller can legitimately have no secret; set `KAMI_SECRET` or
put `secret` in the file when yours requires one. The secret has no command-line
flag to avoid putting it in process arguments. Export it from your secret
management mechanism or store it in the private TOML file. HTTPS uses
certificate validation; redirects are rejected.
Controller requests bypass ambient HTTP proxy variables.

### Read the core's existing YAML

```console
kami -c /path/to/mihomo.yaml
kami -c /path/to/mihomo.yaml status
kami --config /path/to/mihomo.yaml --json status
```

`-c`/`--config` detects `.yaml` / `.yml` (case-insensitive); other extensions use
the kami TOML format. An explicitly selected file replaces the default
configuration; the files are not merged. For a symbolic link, the target file's extension
determines the format.

kami reads the top-level `external-controller`, `external-controller-tls` and
`secret` fields. These are Mihomo's [API connection settings](https://wiki.metacubex.one/config/general/#api).
When both listeners exist, kami prefers TLS and retains normal certificate
verification. Other core settings, proxy definitions and rules are ignored.
This reads connection information only: it never writes the file, starts the
core, or sends the YAML to the controller. Runtime state still comes from the API.
Use the actual generated configuration if your launcher merges several files.

Wildcard listeners `:9090`, `0.0.0.0:9090` and `*:9090` connect through
`127.0.0.1`; `[::]:9090` connects through `[::1]`. A local file cannot identify
the remote machine that uses it. To connect to a remote target, override the
address while keeping the file's secret:

```console
kami -c ./remote-mihomo.yaml --controller http://192.0.2.10:9090
```

The same precedence applies: flags > environment > selected file > defaults.
An existing `KAMI_SECRET` overrides the YAML secret. Missing/empty `secret`
means no bearer credential unless supplied through the environment. Without
an HTTP(S) listener, YAML mode requires `--controller` or `KAMI_CONTROLLER`;
it does not guess port 9090. Unix sockets and named pipes are not supported.
Input must be UTF-8, a single YAML mapping, and at most 16 MiB. Parse errors
omit source excerpts to avoid exposing secrets.

### One-shot commands

```console
kami status
kami --controller http://127.0.0.1:9090 --json status
kami proxies --search Tokyo
kami select 'Proxy' 'Node name'
kami delay 'Node name'
kami mode rule
kami tun on
kami connections
kami close CONNECTION_ID
kami providers
kami update 'Provider name'
kami logs --level info
kami --json traffic
```

`status` lists each proxy group's current selection and samples one frame of
Mihomo's live traffic stream for upload/download bytes per second. Its JSON
output adds `selected` (group names mapped to node and `delay_ms`) and `traffic`
(`up` and `down` in bytes per second). If the proxy or traffic endpoint is
unavailable, the core status still prints and the affected section says so.

`proxies` prints a tree of groups and their nodes, with the latest recorded
latency and a `SELECTED` marker. It does not start delay tests. A group name
search shows all its nodes; a node name search shows that node under each
matching group. When stdout is a terminal, group names and selected node names
use their latency color: green through 100 ms, yellow through 300 ms, red above
300 ms, and gray when unknown. `NO_COLOR` disables color. `--json proxies`
retains the raw proxy map for scripts.

JSON responses are single objects, or one object per line for streams. Errors
go to stderr. Exit codes: 0 success, 1 operation failure, 2 argument error,
130 interruption. `--log` adds sanitized diagnostic context, never raw response
bodies or credentials. CLI commands use no automatic mutation retries.

Only `Selector` groups support selection in this release. Other groups remain
visible and can be measured. Delay tests use
`https://www.gstatic.com/generate_204` with a 5-second core timeout.

## TUI

The sidebar contains the four pages. The focus strip explicitly names NAVIGATION,
CONTENT or SEARCH. Tab switches navigation/content focus; the active area is
highlighted and the inactive list selection is dimmed. The current page stays
highlighted in navigation even when content has focus. Select an item to inspect it.
Proxies show a list and right-hand inspector on wide terminals; drag their
separator to resize. Connections use a full-width list with selected details
below it, including a compact summary in small windows. Logs use the full
content width; open Details for a long message. On smaller terminals, use the Details action to open
a scrollable popup. Click a sidebar entry to change pages. Below 80 columns the
sidebar becomes a top navigation strip.

Right-click an item, click **Actions**, or press `a` to open its action menu.
Buttons and keyboard shortcuts invoke the same commands. Double-clicking a
proxy group opens it just like Enter; a single click only selects it. Inside a
group, click the pinned **.. Back to groups** row above the nodes to return,
keeping that group selected. Esc outside search also returns. Selecting a node does
not switch traffic: choose **Use node** to apply it. Automatic groups show why
manual selection is unavailable. Mode changes use an explicit three-option
menu. Confirmation dialogs default to **Cancel**; use the mouse, `y`, or
Tab/Right then Enter to confirm. Menus and confirmations retain the original
target even if the live list changes.

| Key | Action |
| --- | --- |
| `1`–`4` | Overview, proxies, connections, logs |
| Tab | Switch navigation/content focus |
| `j` / `k`, arrows | Move selection, or change page when navigation is focused |
| Page Up / Page Down | Move through a list by one visible page |
| `/` | Focus search; filtering updates while typing |
| Ctrl+C in search | Clear the input, keeping search focus |
| Esc in search | Leave input, retaining the current filter |
| Esc outside search | Clear the filter / leave a proxy group |
| Enter in proxies | Open group / show actions for a node |
| `a`, right-click | Contextual actions for the selected item |
| `d` in proxies | Measure selected node/group |
| `x` in connections | Confirm disconnect of selected ID |
| Enter in connections | Open actions, including full details |
| `r` / `R` inside a proxy group | Measure all members of the group |
| `c` in logs / **Clean** | Clear local log history; new logs continue arriving |
| `G` in logs | Follow latest matching logs |
| `m` in overview | Choose rule, global or direct mode |
| `t` in overview | Confirm runtime TUN toggle |
| `s` in overview | Local service menu: status, start, stop, restart, enable, disable |
| `r` outside a proxy group, `?`, `q` | Refresh snapshots, help, quit |

While search is focused, `q` is text and Ctrl+C clears the input. Enter and Tab
keep editing; press Esc to leave input before using `q` to quit. The highlighted
search line and visible cursor identify the active input, including long queries.

Overview also summarizes reported listener ports, current proxy-group routes,
TCP/UDP connection counts and the latest warning/error retained in the log buffer.
Smaller windows show fewer summary rows.

**Refresh group** appears at the top right inside a proxy group. Click it or
press `r` to measure all members, including nodes hidden by the search filter.
Up to four measurements run concurrently. Progress and success/failure counts
appear at the bottom; a failed measurement is marked **Failed**. Refreshing
does not change the active node, and duplicate operations are disabled while busy.

Connections show **DESTINATION** and **ROUTE** by default. To also display
process information in the list and details, add this to the kami TOML file:

```toml
[connections]
show_process = true
```

This controls display only; it does not enable process lookup in Mihomo. An
Mihomo YAML file uses the default columns.

Mouse wheel scrolling moves through lists; clicking a log entry pauses follow.
Click the operation message near the bottom to read its full result. Mouse
capture depends on terminal support; keyboard operation remains available.
Most terminal emulators allow native text selection while holding Shift.

`kami tui --demo` uses an in-memory sample controller. It supports node, mode,
TUN and connection interactions without network access or host changes; local
service actions are disabled. Demo state resets on exit. The header always
labels this mode. See [interaction.md](interaction.md) for the Herdr reference
and the implemented interaction model.

Minimum terminal size: 50 columns by 14 rows; 100 by 24 or larger is more
comfortable. The UI refreshes snapshots every two seconds after requests
complete. Streams reconnect with backoff capped at 15 seconds; an idle log
stream may reconnect when the configured socket timeout elapses. Logs retain
500 entries. Slow/offline controllers leave the last snapshot visible with
an error; the UI continues accepting input. Exiting kami leaves Mihomo running.

## Runtime state and TUN

Mode and TUN changes are runtime-only. Mihomo's configuration reload can
overwrite them. Selection persistence is owned by Mihomo's `profile.store-selected`
setting. kami does not merge YAML, import subscriptions or rewrite routing.

`kami tun on` patches only `tun.enable`, then checks the API readback. It does
not configure DNS/routes, grant core privileges or prove real traffic flow.
Prepare the core TUN/DNS configuration first. `examples/mihomo.yaml` is a
minimal DIRECT-only starting point with TUN initially disabled; replace its
secret and add your providers before actual use. The core needs `/dev/net/tun`
and suitable network capabilities. Recovery is `kami tun off` or stopping the
core through the service manager. No automatic rollback is promised if changing
routes makes a remote controller unreachable.

## Local service

```console
kami service status
kami service start
kami service restart
kami service enable
kami service disable
kami service stop
kami service status --unit custom-mihomo.service
kami service status --user
```

These commands always address the **local** systemd manager, independently of
`--controller`. `enable` does not start a stopped service; `disable` does not
stop a running one. The TUI uses the default system unit; use CLI flags for a
different name or user service.

`examples/mihomo.service` is an optional unit template, assuming a separately
installed `/usr/local/bin/mihomo` and `/etc/mihomo/config.yaml`. It runs the core
as root with a restricted capability set for basic networking; review paths,
privileges and your feature requirements before installation. kami neither
downloads the core nor installs this unit. After manually installing a unit,
use your normal administrator workflow to reload systemd and authorize service
operations. kami uses `systemctl --no-ask-password` and never collects a password
or invokes sudo. Service success is distinct from API readiness; check
`kami status` separately. Use `journalctl -u mihomo.service` for service failures.

## Shell and child proxying

```console
eval "$(kami env)"
eval "$(kami env --unset)"
kami exec -- curl https://example.com
kami exec --proxy http://127.0.0.1:7890 -- curl https://example.com
```

`env` emits shell-quoted POSIX exports; `exec` changes only its child environment
and preserves the child's signals and exit code. The child does not inherit
`KAMI_SECRET`. An explicit `--proxy` avoids controller discovery. Otherwise
kami uses the controller hostname and its mixed/HTTP port, falling back to a
SOCKS port; remote listeners still need to be reachable. These variables affect
only programs that honor them and use a localhost-only `NO_PROXY` list.
`env --unset` removes variables rather than restoring earlier values. No GNOME,
KDE, parent shell or machine-wide settings are changed automatically.

## Development

```console
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Without a host toolchain, run `./hako --setup-tools` once to install rustfmt and
Clippy into the repository cache, then prefix these commands with `./hako`.
`./hako cargo run -- tui --demo` also works in an interactive terminal.

Tests use a loopback fake controller, an in-memory demo and Ratatui's
`TestBackend`. They cover auth, redirects, encoding, timeouts, stream reconnects,
readback checks, CLI child exit codes, modal input and responsive hit geometry.
They do not modify host services or networking. See [design.md](design.md) for
architecture and boundaries.
