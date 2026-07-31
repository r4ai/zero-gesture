#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn app_bundle() -> PathBuf {
    std::env::var_os("ZG_P04A_APP_BUNDLE")
        .map(PathBuf::from)
        .expect("ZG_P04A_APP_BUNDLE must name the packaged .app")
}

fn command_output(program: &str, arguments: &[&str]) -> Output {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("failed to execute {program}: {error}"));
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn plist_value(bundle: &Path, key: &str) -> String {
    let plist = bundle.join("Contents/Info.plist");
    String::from_utf8(
        command_output(
            "/usr/bin/plutil",
            &["-extract", key, "raw", "-o", "-", plist.to_str().unwrap()],
        )
        .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}

fn main_executable(bundle: &Path) -> PathBuf {
    bundle
        .join("Contents/MacOS")
        .join(plist_value(bundle, "CFBundleExecutable"))
}

struct EngineProcess {
    child: Child,
}

impl EngineProcess {
    fn start(bundle: &Path) -> Self {
        let mut engine = Self {
            child: Command::new(main_executable(bundle))
                .arg("--engine")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("packaged Engine must start"),
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                engine.child.try_wait().unwrap().is_none(),
                "packaged Engine exited during the startup observation window"
            );
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        engine
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn bundle_architecture_is_apple_silicon_arm64() {
    let bundle = app_bundle();
    let executable = main_executable(&bundle);
    let architectures = command_output("/usr/bin/lipo", &["-archs", executable.to_str().unwrap()]);
    assert_eq!(
        String::from_utf8(architectures.stdout).unwrap().trim(),
        "arm64"
    );
}

#[test]
fn bundle_identifier_is_stable() {
    assert_eq!(
        plist_value(&app_bundle(), "CFBundleIdentifier"),
        "dev.r4ai.zero-gesture"
    );
}

#[test]
fn bundle_requires_the_current_stable_macos_generation() {
    assert_eq!(plist_value(&app_bundle(), "LSMinimumSystemVersion"), "26.0");
}

#[test]
fn bundle_contains_one_main_executable() {
    let bundle = app_bundle();
    let directory = bundle.join("Contents/MacOS");
    let entries = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![main_executable(&bundle)]);
}

#[test]
fn bundle_code_signature_is_valid() {
    let bundle = app_bundle();
    command_output(
        "/usr/bin/codesign",
        &[
            "--verify",
            "--deep",
            "--strict",
            "--verbose=2",
            bundle.to_str().unwrap(),
        ],
    );
}

#[test]
fn bundle_code_signature_enables_hardened_runtime() {
    let bundle = app_bundle();
    let output = command_output(
        "/usr/bin/codesign",
        &["-dv", "--verbose=4", bundle.to_str().unwrap()],
    );
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .any(|line| line.starts_with("CodeDirectory") && line.contains("runtime")));
}

#[test]
fn engine_mode_owns_no_content_window() {
    let bundle = app_bundle();
    let _engine = EngineProcess::start(&bundle);
}

#[test]
fn engine_mode_enforces_managed_webview_invariant_during_run_events() {
    let bundle = app_bundle();
    let _engine = EngineProcess::start(&bundle);
}
