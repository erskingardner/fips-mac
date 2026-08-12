fn main() {
    println!("cargo:rerun-if-env-changed=FIPS_MONITOR_APP_STORE");
    println!("cargo:rustc-check-cfg=cfg(fips_monitor_app_store)");
    if std::env::var_os("FIPS_MONITOR_APP_STORE").is_some() {
        println!("cargo:rustc-cfg=fips_monitor_app_store");
    }
    tauri_build::build()
}
