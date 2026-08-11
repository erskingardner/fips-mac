# FIPS Monitor

FIPS Monitor is a tray-only macOS companion for a local
[FIPS](https://github.com/erskingardner/fips) node. It monitors daemon health,
peers, transports, traffic, and mesh quality, and—when paired with a compatible
FIPS release—provides revisioned, redacted configuration editing.

The app is a Tauri 2 accessory application with a Svelte/TypeScript webview.
All local-system access stays in Rust: the webview can call only the typed
commands declared in `src-tauri/src/control.rs`. It has no shell, arbitrary
filesystem, HTTP, or opener capability.

## Requirements

- macOS 13 or newer
- Rust stable with the Apple target(s) being built
- [Bun](https://bun.sh/)
- A FIPS daemon control socket (normally `/var/run/fips/control.sock`)

Members of the local `fips` group can access the packaged daemon. A newly added
member must log out and back in before macOS refreshes supplementary groups.

## Development

```sh
bun install --frozen-lockfile
bun run check
bun run test
(cd src-tauri && cargo test)
bun run tauri dev
```

The app never requires a package manager other than Bun. `bun.lock` is the only
frontend lockfile. For a source-built daemon, expand **Development connection**
in Settings or set `FIPS_MONITOR_SOCKET` before launching the app.

The application icon has one editable vector source at
`src-tauri/icons/fips-monitor-mark.svg`. Regenerate every bundled size after
changing it with:

```sh
./scripts/generate-icons.sh
```

The script uses ImageMagick and macOS `iconutil`; it does not introduce a
frontend package dependency. The monochrome tray variants mirror the same
node-built F mark in Rust so health states can be drawn dynamically.

Run the sibling-checkout smoke test after changing the control contract:

```sh
./scripts/smoke-fips.sh
```

It defaults to `../fips`; set `FIPS_CHECKOUT_PATH` to use another checkout.

## Build and release

```sh
bun run tauri build
```

Pull requests run Bun frontend checks, Rust tests, and an unsigned native build.
Tags matching `v*` build the `universal-apple-darwin` target and produce one
signed, notarized DMG. The release workflow verifies signatures, Gatekeeper,
stapling, and both `arm64` and `x86_64` slices before publishing the artifact.

FIPS must release the managed configuration API before FIPS Monitor's settings
controls are considered supported. Older daemons remain fully monitorable; the
Settings view explains the upgrade requirement.

## v1 boundaries

The first release is macOS-only. It does not manage external hosts/ACL files,
the firewall, Linux gateway configuration, automatic updates, iOS, or App Store
distribution.
