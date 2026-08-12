# FIPS Monitor

[![CI](https://github.com/erskingardner/fips-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/erskingardner/fips-monitor/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

FIPS Monitor is a native macOS menu-bar companion for a local
[FIPS](https://github.com/erskingardner/fips) node. It keeps the node's health,
peers, transports, traffic, and mesh quality visible without requiring a
terminal, and provides safe configuration editing when paired with a compatible
FIPS daemon.

The app is built with Tauri 2, Rust, Svelte, TypeScript, Vite, and Bun. It is a
macOS accessory app: healthy launches stay in the menu bar, while the dashboard
and onboarding appear when requested or when the local node needs attention.

## What it does

- Reports node health, identity, version, uptime, mesh address, and TUN state.
- Lists peers and transports with connection and quality details.
- Supports confirmed peer connect and disconnect actions.
- Provides an Access Control page for the live peer whitelist and blocklist,
  including add/remove rules and active-peer disconnects.
- Shows LAN discovery diagnostics and actionable configuration warnings.
- Provides guided settings and an advanced YAML editor backed by FIPS's
  revisioned, redacted configuration API.
- Reviews semantic changes and activation impact before applying them.
- Tracks live applies, daemon restarts, failures, and automatic rollbacks.
- Offers an opt-in Launch at Login menu item for direct-download builds.

## Security model

Local-system interaction stays in Rust. The webview receives typed Tauri
commands and events and has no shell, arbitrary filesystem, HTTP, or opener
capability. Its content security policy permits only bundled local content and
Tauri IPC.

Access Control edits are limited to the `peers.allow` and `peers.deny` files
reported by the daemon, and only when they are in FIPS's standard
`/etc/fips` or `/usr/local/etc/fips` directories. The app does not elevate
privileges. If the installed ACL files are not writable by the current user,
the page reports the permission error and leaves the files unchanged.
Mac App Store builds remain read-only for these files because the sandbox does
not grant direct access to the system configuration directory.

The production control socket is `/var/run/fips/control.sock`. Access is still
subject to the socket's POSIX ownership and mode; packaged FIPS installations
authorize members of the local `fips` group. After adding a user to that group,
log out and back in so macOS refreshes supplementary group membership.

The Mac App Store build is isolated from the direct-download build. It enables
App Sandbox and grants a narrowly scoped temporary read/write exception only
for `/private/var/run/fips/control.sock`, the canonical path backing
`/var/run/fips/control.sock` on macOS. Apple requires an explanation for this
temporary exception during App Store submission. The intended long-term
replacement is an App Group or other sandbox-native IPC channel shared with the
daemon. See
[Mac App Store and TestFlight](docs/app-store.md).

## Requirements

- macOS 13 or newer
- [Bun](https://bun.sh/)
- Rust stable and the Apple target being built
- A local FIPS checkout or installed FIPS daemon

## Run locally

Install dependencies and launch the Tauri development app:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

By default the monitor checks these sockets in order:

1. `/var/run/fips/control.sock`
2. `/run/fips/control.sock`
3. `/tmp/fips-control.sock`

For a source-built daemon using another socket, set the path before launching:

```sh
FIPS_MONITOR_SOCKET=/absolute/path/to/control.sock bun run tauri dev
```

You can also change the development socket from **Settings → Development
connection**. The override is intentionally an absolute local Unix-socket path;
the app does not connect to remote HTTP services.

## Develop and test

Use Bun for every frontend command. `bun.lock` is the only frontend lockfile.

```sh
bun install --frozen-lockfile
bun run check
bun run test
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Run the sibling-checkout contract smoke test after changing the control client:

```sh
./scripts/smoke-fips.sh
```

It defaults to `../fips`. Set `FIPS_CHECKOUT_PATH` to use another checkout.

The application icon has one editable vector source at
`src-tauri/icons/fips-monitor-mark.svg`. Regenerate the bundled icon sizes with:

```sh
./scripts/generate-icons.sh
```

That script uses ImageMagick and macOS `iconutil` and adds no frontend package
dependency. Monochrome tray variants are rendered in Rust from the same visual
language so they can reflect node state dynamically.

## Build

Create the normal direct-download app and DMG:

```sh
bun run tauri build
```

Tags matching `v*` build the universal Apple Silicon/Intel target and produce a
signed, notarized DMG. The release workflow verifies signatures, Gatekeeper,
stapling, and both architecture slices before publishing a draft GitHub release.

For a signed Mac App Store package suitable for TestFlight, follow
[docs/app-store.md](docs/app-store.md). App Store packaging uses a separate
Tauri configuration and does not sandbox normal development or Developer ID
builds.

## Repository layout

- `src/` — Svelte dashboard, settings, and frontend tests
- `src-tauri/src/` — tray application, monitor task, and FIPS control client
- `src-tauri/capabilities/` — narrowly scoped webview permissions
- `src-tauri/Entitlements.appstore.plist` — Mac App Store sandbox entitlements
- `scripts/` — icon generation, FIPS contract smoke test, and App Store tooling
- `docs/` — release and operational documentation

## Scope

The first release is macOS-only. It does not manage external hosts or ACL
files outside the two standard FIPS peer ACL files, the firewall, Linux gateway
configuration, automatic updates, or iOS.
FIPS must release the managed configuration API before the settings controls are
considered supported. Older daemons remain monitorable and the Settings view
explains when an upgrade is required.

## License

FIPS Monitor is free software licensed under the
[GNU Affero General Public License, version 3](LICENSE) (`AGPL-3.0-only`).
