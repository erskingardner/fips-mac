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

`/usr/local/etc/fips/fips-monitor.yaml` and the corresponding app-managed
filename are FIPS configuration-protocol compatibility paths, not product
branding. They should remain unchanged unless FIPS itself defines and ships a
configuration migration.
