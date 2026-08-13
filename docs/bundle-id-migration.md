# Bundle ID migration

FIPS Mac is shown to users as **FIPS** and uses
`com.paper-robin.fips-mac`. The former `com.paper-robin.fips-monitor`
identifier was replaced atomically across:

- Tauri's base and App Store bundle identifiers;
- the App Sandbox application identifier entitlement;
- the App Store provisioning project and profile validation scripts;
- both app-bundled Service Management plist filenames and launchd labels;
- the controller's fixed launchd labels;
- Developer ID release assertions and documentation.

The App Store provisioning profile must be regenerated for the new explicit
App ID before building that distribution. Existing development installations
that registered the former background-service labels should remove the old
development build's service before installing this build.

An older app-managed `/Library/Application Support/FIPS/fips-monitor.yaml` is
recognized only as a one-time migration input. New installations use the
single app-owned `/Library/Application Support/FIPS/fips.yaml`; FIPS itself is
launched without a managed-config overlay.
