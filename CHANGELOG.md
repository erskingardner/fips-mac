# Changelog

All notable changes to FIPS Mac are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for source releases. macOS builds also carry an independently increasing build
number.

## [Unreleased]

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

[Unreleased]: https://github.com/erskingardner/fips-mac/compare/v2026.8.13-build.3...HEAD
[2026.8.13+3]: https://github.com/erskingardner/fips-mac/compare/v2026.8.13...v2026.8.13-build.3
[2026.8.13+2]: https://github.com/erskingardner/fips-mac/releases/tag/v2026.8.13
