#![cfg(windows)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn acceptance_result() -> &'static Value {
    static RESULT: OnceLock<Value> = OnceLock::new();
    RESULT.get_or_init(run_acceptance)
}

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must be set by Windows installed acceptance CI"))
}

fn run_acceptance() -> Value {
    assert_eq!(
        std::env::var("GITHUB_ACTIONS").as_deref(),
        Ok("true"),
        "installed acceptance is restricted to a disposable GitHub runner"
    );
    let installer = required_path("ZG_P05C_INSTALLER");
    let artifact = required_path("ZG_P05C_ARTIFACT");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let script = repository.join("scripts/windows/p05c-installed-acceptance.ps1");
    let output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-File",
            script.to_str().unwrap(),
            "-InstallerPath",
            installer.to_str().unwrap(),
            "-ArtifactPath",
            artifact.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "installed acceptance failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&std::fs::read(artifact).unwrap()).unwrap()
}

#[test]
fn signed_current_user_nsis_installs_to_a_spaced_path_and_cleans_registration() {
    let result = acceptance_result();
    assert_eq!(result["result"], "passed");
    assert_eq!(result["install_scope"], "current-user");
    assert!(result["install_directory"]
        .as_str()
        .unwrap()
        .chars()
        .any(char::is_whitespace));
    let signing = &result["authenticode"];
    let expected_thumbprint = signing["expected_thumbprint"].as_str().unwrap();
    assert!(!expected_thumbprint.is_empty());
    for artifact in [
        "installer",
        "installed_executable",
        "reinstalled_executable",
    ] {
        assert_eq!(signing[artifact]["status"], "Valid");
        assert_eq!(
            signing[artifact]["thumbprint"]
                .as_str()
                .unwrap()
                .to_ascii_uppercase(),
            expected_thumbprint.to_ascii_uppercase()
        );
    }
    assert_eq!(result["autostart_exact_quoted"], true);
    assert_eq!(result["startup_approved_observed"], true);
    assert_eq!(
        result["cancelled_uninstall_preserved_install_and_autostart"],
        true
    );
    assert_eq!(result["uninstall_removed_autostart"], true);
}

#[test]
fn installed_settings_and_engine_lifecycle_uses_production_processes() {
    let result = acceptance_result();
    assert_eq!(result["engine_settings_coexisted"], true);
    assert_eq!(result["settings_single_instance"], true);
    assert_eq!(result["settings_close_removed_webview_tree"], true);
    assert_eq!(result["settings_close_kept_engine"], true);
    assert_eq!(result["quit_stopped_engine"], true);
    assert_eq!(result["quit_preserved_autostart"], true);
    assert_eq!(result["rejected_acceptance_modes_preserved_state"], true);
    assert_eq!(result["clean_shutdown_removed_control_secret"], true);
}

#[test]
fn installed_release_rejects_unset_and_wrong_acceptance_tokens_without_state_change() {
    assert_eq!(
        acceptance_result()["rejected_acceptance_modes_preserved_state"],
        true
    );
}

#[test]
fn cancelled_production_uninstall_keeps_install_and_autostart_registration() {
    assert_eq!(
        acceptance_result()["cancelled_uninstall_preserved_install_and_autostart"],
        true
    );
}

#[test]
fn reinstall_and_uninstall_retain_config_and_logs() {
    let result = acceptance_result();
    for field in [
        "config_retained_after_reinstall",
        "config_retained_after_uninstall",
        "logs_retained_after_reinstall",
        "logs_retained_after_uninstall",
        "sentinel_retained_after_reinstall",
        "sentinel_retained_after_uninstall",
    ] {
        assert_eq!(result[field], true, "{field}");
    }
    assert!(result["retained_log_evidence"]["before_reinstall"]
        .as_object()
        .is_some_and(|files| !files.is_empty()));
    assert!(result["retained_log_evidence"]["before_uninstall"]
        .as_object()
        .is_some_and(|files| !files.is_empty()));
}

#[test]
fn installed_release_resources_and_lifecycle_meet_kpi_gates() {
    let result = acceptance_result();
    assert_eq!(result["kpi_gates_passed"], true);
    let measurements = &result["measurements"];
    assert_eq!(measurements["engine"]["webview_count"], 0);
    assert_eq!(measurements["engine"]["descendant_webview_count"], 0);
    assert!(
        measurements["engine"]["working_set_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(measurements["engine"]["thread_count"].as_u64().unwrap() > 0);
    assert!(measurements["engine"]["handle_count"].as_u64().unwrap() > 0);
}
