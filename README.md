# FIPS Mac

[![CI](https://github.com/erskingardner/fips-mac/actions/workflows/ci.yml/badge.svg)](https://github.com/erskingardner/fips-mac/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

FIPS Mac is the native Mac app for installing, running, and understanding a
local [FIPS](https://github.com/erskingardner/fips) node. It appears as **FIPS**
in Finder, the Dock, the App Switcher, the menu bar, and macOS settings. The Developer ID
edition includes a pinned universal FIPS executable, so a new user can install
one app, approve its background service once, and manage the node without a
package manager or Terminal.

The app is built with Tauri 2, Rust, Svelte, TypeScript, Vite, and Bun. It opens
as a regular Mac app by default, with its dashboard, Dock icon, App Switcher
entry, and menu-bar control all available. Users who prefer a menu-bar-only app
can hide the Dock icon and dashboard-at-launch behavior in Settings.

## Naming

- **FIPS Mac** is the repository and project name.
- **FIPS** is the user-facing application name everywhere on macOS.
- `com.paper-robin.fips-mac` is the bundle identifier. Signing, entitlements,
  Service Management labels, and App Store tooling use it consistently. See
  [the bundle-ID migration record](docs/bundle-id-migration.md).

## What it does

- Reports node health, identity, version, uptime, mesh address, and TUN state.
- Lists peers and transports with connection and quality details.
- Supports confirmed peer connect and disconnect actions.
- Shows actionable warnings when LAN discovery is enabled without a reachable
  UDP listener.
- Provides guided settings and an advanced YAML editor backed by the app's
  revisioned, redacted configuration manager.
- Reviews semantic changes and activation impact before applying them.
- Tracks live applies, daemon restarts, failures, and automatic rollbacks.
- Installs a safe-default FIPS node through macOS 13's Background Items flow.
- Starts, stops, and restarts FIPS from the dashboard or menu bar. Turning the
  service off persists across restarts of the Mac.
- Detects package-managed FIPS installations and supports either companion mode
  or a rollback-safe migration that preserves configuration and identity.
- Offers an opt-in Launch at Login menu item for direct-download builds.

## Security model

Local-system interaction stays in Rust. The webview receives typed Tauri
commands and events and has no shell, arbitrary filesystem, HTTP, or opener
capability. Its content security policy permits only bundled local content and
Tauri IPC.

The node and lifecycle sockets are `/var/run/fips/control.sock` and
`/var/run/fips-mac/service.sock`. App-managed installations grant local
administrators access without creating a new Unix group or requiring a new
login session. The root lifecycle service is owned by FIPS, accepts only fixed
lifecycle and configuration operations, and never accepts a client-supplied
executable, filesystem path, launchd label, or shell argument. It validates
app-managed YAML with the pinned FIPS config types, preserves redacted secrets,
rejects stale revisions, writes atomically, and rolls back a failed restart.

The bundled node is built without source changes from the exact revision in
[`fips-source-revision`](fips-source-revision). Both it and the lifecycle
service remain inside the signed application bundle and are registered using
Apple's `SMAppService`; nothing is copied into `/usr/local/bin` or
`/Library/LaunchDaemons`. App-managed configuration lives at
`/Library/Application Support/FIPS/fips.yaml`; its initial and last-known-good
copies and apply journal stay beside it with mode `0600`. Configuration is
preserved when the service is removed. See [Developer ID distribution](docs/developer-id.md) and
[third-party notices](THIRD_PARTY_NOTICES.md).

The legacy Mac App Store build is isolated from the direct-download build. It enables
App Sandbox and grants narrowly scoped temporary read/write exceptions only
for the exact control socket under `/private/var/run/fips`. Apple requires an
explanation for these temporary exceptions during App Store submission. The intended long-term
replacement is an App Group or other sandbox-native IPC channel shared with the
daemon, but remains monitor-only and does not include the app-managed node. See
[Mac App Store and TestFlight](docs/app-store.md).

## Requirements

- macOS 13 or newer
- [Bun](https://bun.sh/)
- Rust stable and the Apple target being built
- A local FIPS checkout at the revision in `fips-source-revision` when building
  the bundled Developer ID edition

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
FIPS_MAC_SOCKET=/absolute/path/to/control.sock bun run tauri dev
```

During lifecycle-controller development, its socket can be overridden
independently:

```sh
FIPS_MAC_SERVICE_SOCKET=/absolute/path/to/service.sock bun run tauri dev
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
cargo fmt --manifest-path src-tauri/service/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/service/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/service/Cargo.toml
```

Run the sibling-checkout contract smoke test after changing the control client:

```sh
./scripts/smoke-fips.sh
```

It defaults to `../fips`. Set `FIPS_CHECKOUT_PATH` to use another checkout.

The application icon has one editable vector source at
`src-tauri/icons/fips-mac-mark.svg`. Regenerate the bundled icon sizes with:

```sh
./scripts/generate-icons.sh
```

That script uses ImageMagick and macOS `iconutil` and adds no frontend package
dependency. Monochrome tray variants are rendered in Rust from the same visual
language so they can reflect node state dynamically.

## Build

Create the full universal Developer ID app and DMG, including the pinned FIPS
node:

```sh
bun run tauri:build:developer-id
```

The Developer ID wrapper first runs `scripts/prepare-macos-bundle`, which compiles FIPS and the lifecycle
service for Intel and Apple Silicon before Tauri signs the nested executables
and app. Set `FIPS_CHECKOUT_PATH` only when FIPS is not available at `../fips`.
The script refuses a different revision or tracked local changes.

Release tags use `v<version>-build.<build>`, such as
`v2026.8.13-build.3`. They build the universal Apple Silicon/Intel target and
produce a signed, notarized DMG. The release workflow verifies all three
executable signatures, Gatekeeper, stapling, launchd plists, and architecture
slices before publishing a public GitHub release with generated release notes
and a SHA-256 checksum. See
[Developer ID distribution](docs/developer-id.md) for the required repository
secrets and release procedure.

For the separate monitor-only Mac App Store package, follow
[docs/app-store.md](docs/app-store.md). App Store packaging uses a separate
Tauri configuration and does not sandbox normal development or Developer ID
builds.

## Repository layout

- `src/` — Svelte dashboard, settings, and frontend tests
- `src-tauri/src/` — tray application, monitor task, and FIPS control client
- `src-tauri/service/` — separately built privileged lifecycle controller
- `src-tauri/launchd/` — app-bundled `SMAppService` LaunchDaemon definitions
- `src-tauri/resources/` — safe default configuration and bundled notices
- `src-tauri/capabilities/` — narrowly scoped webview permissions
- `src-tauri/Entitlements.appstore.plist` — Mac App Store sandbox entitlements
- `scripts/` — icon generation, FIPS contract smoke test, and App Store tooling
- `docs/` — release and operational documentation
- `CHANGELOG.md` — user-visible changes following Keep a Changelog

## Scope

The first release is macOS-only. It does not manage external hosts or ACL
files, the firewall, Linux gateway configuration, automatic updates, or iOS.
Only the Developer ID edition provides the one-app node installation. A future
Mac App Store/TestFlight edition would require a separate Network Extension
architecture; the root LaunchDaemon is intentionally not presented as an App
Store-compatible design. Package-managed daemons remain monitorable. Their
configuration stays untouched and becomes editable only after the user
explicitly migrates the node into the app.

## License

FIPS Mac is free software licensed under the
[GNU Affero General Public License, version 3](LICENSE) (`AGPL-3.0-only`).
