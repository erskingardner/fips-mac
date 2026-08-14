# Developer ID distribution

The direct-download edition is the full FIPS Mac product, shown to users as
**FIPS**. It is a notarized universal app that installs or adopts one standard
FIPS node; it never creates an app-private second installation.

## Bundle and system layout

```text
FIPS.app/
  Contents/MacOS/fips-mac
  Contents/MacOS/fips-mac-service
  Contents/Library/LaunchDaemons/
    com.paper-robin.fips-mac.service-control.plist
  Contents/Resources/
    fips-macos-arm64.pkg
    fips-macos-x86_64.pkg
    FIPS-LICENSE.txt

/usr/local/bin/{fips,fipsctl,fipstop}
/usr/local/etc/fips/fips.yaml
/Library/LaunchDaemons/com.fips.daemon.plist
```

The two `.pkg` resources are built from the pinned, unmodified upstream FIPS
revision. The app selects the package matching the running architecture and
opens macOS Installer. Upstream's installer preserves existing configuration
and identity during upgrades.

Only the fixed management helper uses `SMAppService`. It starts, stops, and
restarts `com.fips.daemon` and validates and atomically replaces only
`/usr/local/etc/fips/fips.yaml`. Initial, last-known-good, and apply-journal
files stay under `/Library/Application Support/FIPS/standard-management` with
mode `0600`.

## Product flows

For a new Mac, the app opens the standard FIPS installer. After Installer
finishes, the app detects `com.fips.daemon`, registers its management helper,
and may direct the user to **System Settings → General → Login Items** for the
one-time Background Items approval.

For an existing standard installation, the app immediately monitors the node.
Enabling management registers only the helper; it does not copy configuration,
change identity, reinstall FIPS, or switch launchd labels. Once enabled, the
same node can be configured and controlled from the dashboard.

Configuration drafts redact the identity secret and Tor passwords, require the
revision that was loaded, validate against the exact pinned FIPS types, and are
written atomically. Semantic changes restart FIPS only after the apply response
has been flushed. A failed startup restores the last-known-good file and tries
the standard daemon again.

## Fast local packaged build

```sh
bun install --frozen-lockfile
bun run tauri:build:local
```

This produces a Developer ID Application-signed native-architecture app. Its
embedded FIPS package is unsigned unless
`APPLE_INSTALLER_SIGNING_IDENTITY` is set, so this path is for local testing,
not public distribution. It skips the universal merge, DMG, and notarization.
Copy the printed app to `/Applications/FIPS.app` and open it manually to test
Background Items. Product Preview and the Developer page remain exclusive to
`tauri dev`.

## Universal release build

```sh
bun install --frozen-lockfile
rustup target add aarch64-apple-darwin x86_64-apple-darwin
APPLE_INSTALLER_SIGNING_IDENTITY="Developer ID Installer: Paper Robin (…)" \
  bun run tauri:build:developer-id
```

The FIPS checkout must exactly match `fips-source-revision` and have no tracked
changes. The DMG is produced under
`src-tauri/target/universal-apple-darwin/release/bundle/dmg/`.

## GitHub release secrets

The app and its embedded installer use two different Apple certificate types.
Create both under **Certificates, Identifiers & Profiles → Certificates**:

1. **Developer ID Application** signs `FIPS.app` and its executables.
2. **Developer ID Installer** signs the architecture-specific FIPS `.pkg`
   resources that the app opens for a new installation.

For the Installer certificate, create or select a Keychain Access certificate
signing request, download and install the issued certificate, then export the
certificate and its private key together from Keychain Access as a
password-protected `.p12`. Confirm the identity is available with:

```sh
security find-identity -v -p basic | grep "Developer ID Installer"
```

Encode the `.p12` directly into the repository secret without writing the
base64 value to the terminal:

```sh
base64 -i DeveloperIDInstaller.p12 | gh secret set APPLE_INSTALLER_CERTIFICATE
gh secret set APPLE_INSTALLER_CERTIFICATE_PASSWORD
gh secret set APPLE_INSTALLER_SIGNING_IDENTITY
```

The signing-identity secret must be the complete displayed identity, including
the team identifier in parentheses.

Application signing:

- `APPLE_CERTIFICATE` — base64 Developer ID Application `.p12`;
- `APPLE_CERTIFICATE_PASSWORD`;
- `APPLE_SIGNING_IDENTITY`;
- `APPLE_TEAM_ID`.

Embedded package signing:

- `APPLE_INSTALLER_CERTIFICATE` — base64 Developer ID Installer `.p12`;
- `APPLE_INSTALLER_CERTIFICATE_PASSWORD`;
- `APPLE_INSTALLER_SIGNING_IDENTITY` — complete Developer ID Installer name.

Notarization uses either `APPLE_API_ISSUER`, `APPLE_API_KEY`, and
`APPLE_API_KEY_P8_BASE64`, or `APPLE_ID` plus an app-specific
`APPLE_PASSWORD`.

Release tags are `v<version>-build.<build>`, for example:

```sh
git tag -a v2026.8.14-build.4 -m "FIPS 2026.8.14 (build 4)"
git push origin v2026.8.14-build.4
```

The workflow checks both installer signatures, the management helper and app
signatures, universal slices, Gatekeeper, notarization, stapling, and the DMG
checksum before publishing.

## Manual release-candidate checks

Test both paths before tagging:

1. A Mac with no FIPS installation: open Installer, finish installation,
   approve the helper, then configure/start/stop/restart.
2. A Mac with an existing standard installation: verify the identity and
   configuration are unchanged, enable management, edit and apply one safe
   setting, and confirm rollback behavior with an invalid startup setting.
3. Disable app management and verify `com.fips.daemon` remains installed and
   continues in its prior running or stopped state.
