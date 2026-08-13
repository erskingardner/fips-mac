# Mac App Store and TestFlight

FIPS Mac is shown to users as **FIPS**. Apple Developer team `8Z6Q2LZ77W`,
bundle identifier `com.paper-robin.fips-mac`, version `2026.8.6`, build `1`,
and the macOS 13 minimum remain current.

This remains a **monitor-only** distribution track. It does not contain or
register the Developer ID edition's root FIPS node and lifecycle controller.
Delivering the one-app node through TestFlight would require the separate
Network Extension architecture described in the project roadmap.

The App Store configuration is deliberately separate from normal development
and Developer ID builds. Only builds made with `tauri.appstore.conf.json` are
sandboxed.

## Entitlements

`src-tauri/Entitlements.appstore.plist` declares only the privileges used by the
App Store build:

| Entitlement | Reason |
| --- | --- |
| `com.apple.security.app-sandbox` | Required for Mac App Store distribution. |
| `com.apple.security.network.client` | Allows the monitor to initiate its local control connection. |
| `com.apple.application-identifier` | Binds the app to the Paper Robin team and bundle ID. |
| `com.apple.developer.team-identifier` | Binds the app to Apple Developer team `8Z6Q2LZ77W`. |
| `com.apple.security.temporary-exception.files.absolute-path.read-write` | Allows connection to the single daemon socket at `/private/var/run/fips/control.sock`. |

The exception is intentionally a file path, not access to `/var/run`, `/private`,
or another directory. The application still needs the normal POSIX permission
granted by the daemon to members of the local `fips` group.

Apple requires usage information for every temporary exception. Use this as the
basis of the App Sandbox Entitlement Usage Information response:

> FIPS is a local status and configuration client for the separately
> installed FIPS networking daemon. It connects only to the daemon-owned Unix
> domain socket at `/private/var/run/fips/control.sock`; it does not enumerate,
> create, delete, or modify other files in `/private/var/run`. The daemon uses
> POSIX ownership and the local `fips` group to authorize the signed-in user.
> The exception enables status queries and explicitly confirmed configuration
> operations. Install and start FIPS, then launch the FIPS Mac app to assess it.

Before submitting the build for review, file a Feedback Assistant report for
the temporary exception and add its ID to that explanation. The long-term fix
is to move the IPC endpoint to a registered App Group container or a supported
privileged-helper/XPC design, then remove the exception.

Launch at Login currently uses a user LaunchAgent in direct-download builds.
Do not treat it as supported in the sandboxed build until it has been migrated
to a sandbox-compatible Service Management login item and tested from a
TestFlight installation.

## Apple account setup

1. Ensure the explicit `com.paper-robin.fips-mac` App ID exists for team
   `8Z6Q2LZ77W` and has App Sandbox enabled.
2. Create the FIPS app record in App Store Connect using
   `com.paper-robin.fips-mac`.
3. Sign in to the Paper Robin team under **Xcode → Settings → Accounts**.
4. Ensure these identities are available in the keychain:
   - `Apple Distribution`
   - `Mac Installer Distribution` (displayed locally as
     `3rd Party Mac Developer Installer`)

The repository's helper generates a temporary Xcode project only to ask Xcode
for a managed Mac App Store distribution profile. It never uploads or packages
that helper as FIPS.

```sh
bun run tauri:profile:app-store
```

The generated `src-tauri/appstore/FIPS.provisionprofile` is ignored by
Git. Regenerate it after capabilities change or before it expires.

## Build and verify

Install both Rust targets once:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Create a signed universal `.app` and installer package:

```sh
bun install --frozen-lockfile
bun run check
bun run test
cargo test --manifest-path src-tauri/Cargo.toml
bun run tauri:build:app-store
```

The build script verifies the profile's team, application identifier,
certificate, type, and expiration before building. It then verifies the signed
bundle metadata, both architecture slices, all required entitlements, embedded
profile, and installer signature.

The resulting package is written to:

```text
src-tauri/target/universal-apple-darwin/release/bundle/macos/FIPS-2026.8.6-1.pkg
```

## Upload

Upload the `.pkg` with Transporter or App Store Connect API credentials. Wait
for App Store Connect processing to finish, complete the export-compliance and
App Sandbox exception questions, and then add the build to an internal
TestFlight group.

Build numbers are immutable after upload. Keep build `1` until it is uploaded;
use build `2` for the next uploaded binary.
