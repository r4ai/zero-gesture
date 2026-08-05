#![cfg(windows)]

use serde_json::Value;

fn json(path: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(path),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn windows_bundle_is_current_user_nsis_only() {
    let config = json("src-tauri/tauri.windows.conf.json");
    let bundle = &config["bundle"];
    assert_eq!(bundle["targets"], serde_json::json!(["nsis"]));
    assert_eq!(bundle["windows"]["allowDowngrades"], false);
    assert_eq!(bundle["windows"]["digestAlgorithm"], "sha256");
    assert_eq!(bundle["windows"]["nsis"]["installMode"], "currentUser");
    assert_eq!(
        bundle["windows"]["nsis"]["installerHooks"],
        "./windows/hooks.nsh"
    );
}

#[test]
fn package_identity_and_version_are_fixed_across_manifests() {
    let config = json("src-tauri/tauri.conf.json");
    let package = json("package.json");
    assert_eq!(config["productName"], "Zero Gesture");
    assert_eq!(config["identifier"], "dev.r4ai.zero-gesture");
    assert_eq!(config["version"], "0.1.0");
    assert_eq!(package["name"], "zero-gesture");
    assert_eq!(package["version"], config["version"]);
    assert_eq!(env!("CARGO_PKG_NAME"), "zero-gesture");
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.0");
}
