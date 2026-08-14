# FIPS Mac

[![CI](https://github.com/erskingardner/fips-mac/actions/workflows/ci.yml/badge.svg)](https://github.com/erskingardner/fips-mac/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

FIPS Mac is the native Mac app for installing, running, and understanding a
local [FIPS](https://github.com/jmcorgan/fips) node. It appears as **FIPS**
in Finder, the Dock, the App Switcher, the menu bar, and macOS settings. The Developer ID
edition includes pinned Apple Silicon and Intel copies of the standard FIPS
macOS installer. A new user can install from the app; someone who already has
FIPS gets the same monitoring and management without moving or duplicating it.

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
- Opens the standard upstream FIPS package when no node is installed.
- Starts, stops, and restarts FIPS from the dashboard or menu bar. Turning the
  service off persists across restarts of the Mac.
- Detects the standard `com.fips.daemon` installation and manages its existing
  identity and `/usr/local/etc/fips/fips.yaml` in place.
- Offers an opt-in Launch at Login menu item for direct-download builds.

## Security model

Local-system interaction stays in Rust. The webview receives typed Tauri
commands and events and has no shell, arbitrary filesystem, HTTP, or opener
capability. Its content security policy permits only bundled local content and
Tauri IPC.

The node and lifecycle sockets are `/var/run/fips/control.sock` and
`/var/run/fips-mac/service.sock`. The node keeps upstream's `fips` group
authorization model; a newly added group membership can require signing out and
back in. The root lifecycle service accepts only fixed
lifecycle and configuration operations, and never accepts a client-supplied
executable, filesystem path, launchd label, or shell argument. It validates
the canonical YAML with the pinned FIPS config types, preserves redacted secrets,
rejects stale revisions, writes atomically, and rolls back a failed restart.

The standard installer is built without source changes from the exact revision
in [`fips-source-revision`](fips-source-revision). It installs the upstream
layout under `/usr/local` plus `com.fips.daemon`. Only the fixed management
helper runs from the app bundle through `SMAppService`. The active configuration
remains `/usr/local/etc/fips/fips.yaml`; its initial and last-known-good copies
and apply journal live separately under `/Library/Application Support/FIPS/standard-management`
with mode `0600`. See [Developer ID distribution](docs/developer-id.md) and
[third-party notices](THIRD_PARTY_NOTICES.md).

The legacy Mac App Store build is isolated from the direct-download build. It enables
App Sandbox and grants narrowly scoped temporary read/write exceptions only
for the exact control socket under `/private/var/run/fips`. Apple requires an
explanation for these temporary exceptions during App Store submission. The intended long-term
replacement is an App Group or other sandbox-native IPC channel shared with the
daemon, but remains monitor-only and does not include node installation. See
[Mac App Store and TestFlight](docs/app-store.md).

## Requirements

- macOS 13 or newer
- [Bun](https://bun.sh/)
- Rust stable and the Apple target being built
- A local FIPS checkout at the revision in `fips-source-revision` when building
  the packaged Developer ID edition

## Run locally

Install dependencies and launch the Tauri development app:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Development builds start in **Product Preview** mode. A persistent banner marks
all node data and actions as simulated, and the scenario picker covers running,
stopped, existing-installation, onboarding, approval, conflict, and rollback
states. Product Preview drives the real dashboard, settings, tray, and command
flows but never touches FIPS, launchd, DNS, TUN, or configuration files. Turn it
off from the banner or **Settings → Developer** to inspect the live
local node instead.

Here, **local node** means the FIPS installation running on this Mac. Whether
its binary was compiled locally or downloaded is provenance, not a different
installation type. The app detects the daemon by its FIPS control protocol and
its registered macOS service; socket location alone does not determine
ownership.

To start directly with live data:

```sh
bun run tauri:dev:live
```

The monitor checks these supported macOS sockets in order:

1. `/var/run/fips/control.sock`
2. `/run/fips/control.sock`
3. `/tmp/fips-control.sock`

Packaged apps—including `bun run tauri:build:local`—use the same automatic
detection. They omit the Developer settings page and its manual socket override,
but still recognize an installed daemon configured to use `/tmp/fips-control.sock`.

For a source-built daemon using another socket, set the path before launching:

```sh
FIPS_MAC_SOCKET=/absolute/path/to/control.sock bun run tauri dev
```

During lifecycle-controller development, its socket can be overridden
independently:

```sh
FIPS_MAC_SERVICE_SOCKET=/absolute/path/to/service.sock bun run tauri dev
```

You can also change the source-built socket from **Settings → Developer** in
`tauri dev` after disabling Product Preview. The override is intentionally an
absolute local Unix-socket path; the app does not connect to remote HTTP
services.

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

For a fast pre-merge integration check, build a signed native-architecture
`.app` without a universal binary, DMG, notarization, installation, or launch:

```sh
bun run tauri:build:local
```

The local builder uses the checked-in FIPS revision. If the sibling checkout
has moved ahead or has tracked changes, it creates an ignored, cached checkout
under `src-tauri/target` rather than changing the sibling repository. It reuses
that source and a dedicated Cargo target directory between runs and prints the
resulting `FIPS.app` path. Copying the app into `/Applications` and opening it
remains an explicit manual step because that can replace an installed build and
register Background Items.
Although it uses an unoptimized debug binary for speed, the packaged local app
excludes Product Preview and connects to the installed FIPS daemon using normal
automatic detection. Product Preview and manual socket overrides are available
only through `tauri dev`.

Create the full universal Developer ID app and DMG, including both standard
FIPS installer architectures:

```sh
bun run tauri:build:developer-id
```

The Developer ID wrapper first runs `scripts/prepare-macos-bundle`, which builds
the standard FIPS packages and management helper for Intel and Apple Silicon.
Public builds set `APPLE_INSTALLER_SIGNING_IDENTITY` before Tauri signs and
notarizes the app. Set `FIPS_CHECKOUT_PATH` only when FIPS is not at `../fips`.
The script refuses a different revision or tracked local changes.

Release tags use `v<version>-build.<build>`, such as
`v2026.8.14-build.4`. They build the universal Apple Silicon/Intel target and
produce a signed, notarized DMG. The release workflow verifies the app and
helper signatures, both embedded installer signatures, Gatekeeper, stapling,
and universal architecture slices before publishing a public GitHub release.
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
- `src-tauri/resources/` — generated upstream installer packages and notices
- `src-tauri/capabilities/` — narrowly scoped webview permissions
- `src-tauri/Entitlements.appstore.plist` — Mac App Store sandbox entitlements
- `scripts/` — icon generation, FIPS contract smoke test, and App Store tooling
- `docs/` — release and operational documentation
- `CHANGELOG.md` — user-visible changes following Keep a Changelog

## Scope

The first release is macOS-only. It does not manage external hosts or ACL
files, the firewall, Linux gateway configuration, automatic updates, or iOS.
Only the Developer ID edition provides node installation. A future
Mac App Store/TestFlight edition would require a separate Network Extension
architecture; the root LaunchDaemon is intentionally not presented as an App
Store-compatible design. Existing standard installations remain monitorable
and become editable after the user enables the fixed management helper; no
migration or second node is created.

## License

FIPS Mac is free software licensed under the
[GNU Affero General Public License, version 3](LICENSE) (`AGPL-3.0-only`).
