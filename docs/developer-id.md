# Developer ID distribution

The direct-download edition is the full FIPS Mac product, installed and shown
to users as **FIPS**. It is a
notarized, universal Mac app that owns its user experience while running the
FIPS networking executable without source changes.

## Bundle layout

```text
FIPS.app/
  Contents/MacOS/fips-mac
  Contents/MacOS/fips
  Contents/MacOS/fips-mac-service
  Contents/Library/LaunchDaemons/
    com.paper-robin.fips-mac.node.plist
    com.paper-robin.fips-mac.service-control.plist
  Contents/Resources/fips.default.yaml
  Contents/Resources/FIPS-LICENSE.txt
```

Both LaunchDaemons use `BundleProgram`, so Service Management runs the signed
executables in the app instead of copying code into a shared system directory.
The node begins disabled and is enabled only after onboarding prepares its data
directory. The independent controller remains available when the node is off.

## First run and approval

1. The user drags FIPS to Applications and opens it.
2. FIPS detects an app-managed, package-managed, conflicting, or absent
   node installation.
3. For a new installation, macOS asks an administrator to authorize the
   background services. If approval remains pending, the app opens **System
   Settings → General → Login Items** and explains the exact switch.
4. The controller creates `/Library/Application Support/FIPS` and installs a
   safe starting configuration: persistent identity and local UDP enabled; TUN,
   DNS, LAN discovery, and Nostr rendezvous disabled.
5. The app starts the bundled node, waits for `show_status`, and switches to the
   control socket that actually answers. Failure to become controllable rolls
   the installation back instead of reporting a false success.

When the app-managed configuration enables DNS, the controller maintains
`/etc/resolver/fips` and flushes the macOS DNS cache. It removes or replaces
only a file carrying FIPS's marker, except when migration adopts the
exact resolver file installed by the existing FIPS package.

The same controller owns configuration safety rather than requiring a custom
FIPS daemon build. It keeps the initial and last-known-good YAML beside
`fips.yaml`, redacts identity and Tor password values before returning drafts,
requires an expected revision for writes, validates with the exact pinned FIPS
config types, and uses atomic `0600` replacements. Semantic changes restart
the node only after the apply response is flushed. If the control socket does
not return, the controller restores the last-known-good file and restarts it.
Package-managed configurations remain monitor-only until migration.

Removing the app-managed node unregisters both services but preserves identity
and configuration. It never silently deletes operator data.

## Existing package installations

Only one node may be active because both variants share sockets, transport
ports, discovery, TUN routes, and DNS behavior. Onboarding offers:

- **Use existing installation** — register FIPS's lifecycle controller
  and continue using `/usr/local` paths. The bundled node remains unregistered.
- **Move into FIPS** — stop and disable `com.fips.daemon`, copy regular
  configuration and identity files into the app-managed data directory, start
  the bundled node, and finalize only after it is running.
- **Not now** — leave the system unchanged.

If import, registration, or startup fails, migration unregisters the bundled
node, restores the package DNS resolver when it adopted that known file, and
restores the previous package service when it was enabled. A resolver file not
created by the FIPS package or FIPS is never overwritten.

## Build locally

Install Bun dependencies and the two macOS Rust targets:

```sh
bun install --frozen-lockfile
rustup target add aarch64-apple-darwin x86_64-apple-darwin
FIPS_CHECKOUT_PATH=/path/to/fips bun run tauri:build:developer-id
```

The FIPS checkout must exactly match `fips-source-revision` and have no tracked
local modifications. The resulting DMG is under
`src-tauri/target/universal-apple-darwin/release/bundle/dmg/`.

Tauri reads the standard signing and notarization environment variables. A
local signed-only validation can set `APPLE_SIGNING_IDENTITY`; a public release
must additionally provide either Apple ID/app-password/team credentials or an
App Store Connect API key so notarization and stapling complete.

## GitHub release setup

The tag workflow publishes one universal DMG for both Apple Silicon and Intel.
Configure these GitHub Actions repository secrets before creating a release
tag:

- `APPLE_CERTIFICATE` — base64-encoded Developer ID Application `.p12`;
- `APPLE_CERTIFICATE_PASSWORD` — the password used when exporting that `.p12`;
- `APPLE_SIGNING_IDENTITY` — the complete Developer ID Application identity;
- `APPLE_TEAM_ID` — the Paper Robin Apple Developer Team ID.

For notarization, the preferred non-personal credential is an App Store Connect
API key. Configure:

- `APPLE_API_ISSUER` — the App Store Connect issuer ID;
- `APPLE_API_KEY` — the API key ID;
- `APPLE_API_KEY_P8_BASE64` — the downloaded `.p8` encoded as a single-line
  base64 value.

The workflow also accepts `APPLE_ID` plus an app-specific `APPLE_PASSWORD`
instead of those three API-key secrets. A Mac signed into Xcode does not make
those local credentials available to GitHub-hosted runners.

After completing the manual release-candidate checks below, create and push a
tag matching the app version:

```sh
git tag -a v2026.8.13 -m "FIPS 2026.8.13"
git push origin v2026.8.13
```

The workflow rejects a tag that does not match both `package.json` and
`tauri.conf.json`. It publishes the release only after the notarized DMG passes
all verification gates. A failed run does not create a public release.

## Release verification

The tag workflow verifies:

- `codesign --verify --deep --strict` for the app;
- independent Developer ID signatures and matching Paper Robin Team ID for all
  three executables;
- Intel and Apple Silicon slices in all three executables;
- both embedded LaunchDaemon property lists;
- Gatekeeper acceptance and stapled tickets for the app and DMG; and
- a SHA-256 checksum uploaded with the public GitHub release.

Registration itself is an intentional manual release-candidate test because it
changes the host's system background services. Test new install, approval,
stop/start/restart, package migration, rollback, removal-with-data-preserved,
and moving the app out of `/Applications` before pushing the release tag.
