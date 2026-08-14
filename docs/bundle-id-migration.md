# Bundle ID migration

FIPS Mac is shown to users as **FIPS** and uses
`com.paper-robin.fips-mac`. The former `com.paper-robin.fips-monitor`
identifier was replaced atomically across:

- Tauri's base and App Store bundle identifiers;
- the App Sandbox application identifier entitlement;
- the App Store provisioning project and profile validation scripts;
- the app-bundled management-helper plist filename and launchd label;
- the controller's fixed launchd labels;
- Developer ID release assertions and documentation.

The App Store provisioning profile must be regenerated for the new explicit
App ID before building that distribution. Existing development installations
that registered the former background-service labels should remove the old
development build's service before installing this build.

Current builds no longer create an app-owned node or configuration. They use
the standard `com.fips.daemon` service and `/usr/local/etc/fips/fips.yaml`
directly. New rollback snapshots and the apply journal use
`/Library/Application Support/FIPS/standard-management`, leaving older
app-owned data untouched.
