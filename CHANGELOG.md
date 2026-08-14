# Changelog

All notable changes to FIPS Mac are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for source releases. macOS builds also carry an independently increasing build
number.

## [Unreleased]

## [2026.8.14+4] - 2026-08-14

### Added

- Added a debug-only Product Preview with a persistent fake-data banner,
  selectable lifecycle and onboarding scenarios, simulated tray state, peers,
  transports, configuration review, and safe no-op actions.
- Added `bun run tauri:build:local` for a signed native-architecture `.app`
  without a universal merge, DMG, notarization, installation, or automatic
  launch.
- Added the standard upstream FIPS macOS installer for both Apple Silicon and
  Intel, selected by the app when no node is installed.
- Added inline copy actions beside every full peer npub and address on both the
  Overview and Peers pages, plus the peer detail drawer.

### Changed

- Restricted Product Preview and manual control-socket overrides to `tauri dev`.
  Packaged apps still automatically detect every supported socket used by an
  installed macOS FIPS node.
- Replaced package-oriented installation labels with the clearer **Existing
  FIPS installation** and **Managed by this app** states.
- Replaced the ambiguous disabled service switch with a lifecycle-controls row
  that offers the next available action: enable controls, set up FIPS, repair
  an installation, or install a full build when running in development mode.
- Replaced the parallel app-owned node with one canonical installation:
  `com.fips.daemon`, `/usr/local/bin`, and `/usr/local/etc/fips/fips.yaml`.
  Existing nodes are managed in place without copying identity or configuration.
- Moved configuration backups and the apply journal to
  `/Library/Application Support/FIPS/standard-management` while keeping the
  active configuration in its standard upstream location.
- Simplified user-facing peer terminology, enlarged peer identities, and
  standardized technical values such as npubs, addresses, and counters on a
  monospace typeface.

### Fixed

- Removed the Developer page from fast local builds while preserving automatic
  detection of normally installed FIPS nodes with non-default socket paths.
- Corrected packet-loss reporting to use initialized smoothed MMP link
  measurements, convert FIPS fractions to percentages, and distinguish missing
  measurements from zero loss. New loss histories begin from a zero baseline,
  and peer details now show their measured loss.
- Corrected the Active Sessions sparkline, which previously plotted tree depth.
- Promoted the full npub to the primary node identity, added copy controls for
  both identity values, and moved lifecycle controls below them so neither the
  npub nor mesh IPv6 address wraps.
- Prevented peer npubs from overlapping their copy controls by truncating long
  values with an ellipsis while preserving the complete value in a tooltip.
- Made success and error toasts fade out automatically after five seconds while
  retaining click-to-dismiss behavior and restarting the timer for new events.

## [2026.8.13+3] - 2026-08-13

### Changed

- Made FIPS behave as a regular Mac app by default, opening the dashboard and
  appearing in the Dock and App Switcher while retaining its menu-bar control.
- Kept menu-bar-only operation available as an opt-in preference.

### Fixed

- Restored and focused the dashboard whenever macOS reopens an already-running
  FIPS app after its window was closed.
- Migrated prerelease visibility preferences and settings from the former
  `com.paper-robin.fips-monitor` bundle so existing users cannot be stranded in
  an inaccessible background-only process.
- Removed the static `LSUIElement` declaration that forced every packaged build
  into accessory-app behavior before runtime preferences were applied.

## [2026.8.13+2] - 2026-08-13

### Added

- Initial universal Developer ID release for Apple Silicon and Intel Macs.
- Bundled FIPS node installation and lifecycle management through macOS
  Background Items.
- Dashboard, menu-bar health controls, peer and transport monitoring, guided
  configuration, safe apply and rollback, and package-installation migration.
- Signed, notarized, and stapled DMG publishing through GitHub Actions.

[Unreleased]: https://github.com/erskingardner/fips-mac/compare/v2026.8.14-build.4...HEAD
[2026.8.14+4]: https://github.com/erskingardner/fips-mac/compare/v2026.8.13-build.3...v2026.8.14-build.4
[2026.8.13+3]: https://github.com/erskingardner/fips-mac/compare/v2026.8.13...v2026.8.13-build.3
[2026.8.13+2]: https://github.com/erskingardner/fips-mac/releases/tag/v2026.8.13
