#![cfg(windows)]

use serde_json::Value;
use std::process::Command;

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

#[test]
fn uninstall_hook_removes_autostart_only_after_successful_uninstall() {
    let hooks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows/hooks.nsh"),
    )
    .unwrap();
    let pre = hooks.find("!macro NSIS_HOOK_PREUNINSTALL").unwrap();
    let post = hooks.find("!macro NSIS_HOOK_POSTUNINSTALL").unwrap();
    let delete = hooks.find("DeleteRegValue").unwrap();
    assert!(pre < post);
    assert!(post < delete);
    assert!(hooks[pre..post].contains("Abort"));
    assert!(!hooks[pre..post].contains("DeleteRegValue"));
}

#[test]
fn installed_acceptance_rejects_sibling_prefix_artifact_path() {
    let temp = std::env::temp_dir().join(format!(
        "zero-gesture-p05c-containment-{}",
        std::process::id()
    ));
    let runner = temp.join("runner");
    let sibling_artifact = temp.join("runner-sibling").join("artifact.json");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-File",
            repository
                .join("scripts/windows/p05c-installed-acceptance.ps1")
                .to_str()
                .unwrap(),
            "-InstallerPath",
            temp.join("missing-installer.exe").to_str().unwrap(),
            "-ArtifactPath",
            sibling_artifact.to_str().unwrap(),
        ])
        .env("GITHUB_ACTIONS", "true")
        .env("RUNNER_TEMP", runner)
        .output()
        .unwrap();
    let error = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success());
    assert!(
        error.contains("Artifact path must stay below RUNNER_TEMP."),
        "{error}"
    );
}
