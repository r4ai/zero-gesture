#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(5);

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

fn main_executable(bundle: &Path) -> PathBuf {
    let plist = bundle.join("Contents/Info.plist");
    let executable_name = String::from_utf8(
        command_output(
            "/usr/bin/plutil",
            &[
                "-extract",
                "CFBundleExecutable",
                "raw",
                "-o",
                "-",
                plist.to_str().unwrap(),
            ],
        )
        .stdout,
    )
    .unwrap();
    bundle.join("Contents/MacOS").join(executable_name.trim())
}

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn wait_for_marker(process: &mut Child, marker: &Path, description: &str) {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        assert!(
            process.try_wait().unwrap().is_none(),
            "{description} exited before signaling readiness"
        );
        if marker.exists() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{description} did not signal readiness within {START_TIMEOUT:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn packaged_same_executable_settings_connects_to_engine_over_uds() {
    let directory = tempfile::tempdir().unwrap();
    let config_dir = directory.path().join("config");
    fs::create_dir(&config_dir).unwrap();
    let ready = directory.path().join("engine-ready");
    let connected = directory.path().join("settings-connected");
    let namespace = format!("packaged-{}", std::process::id());
    let executable = main_executable(&app_bundle());

    let mut engine = Process(
        Command::new(&executable)
            .arg("--engine")
            .env("ZG_P03_TEST_CONFIG_DIR", &config_dir)
            .env("ZG_P03_TEST_NAMESPACE", &namespace)
            .env("ZG_P03_TEST_READY_MARKER", &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("packaged Engine must start"),
    );
    wait_for_marker(&mut engine.0, &ready, "packaged Engine");

    let mut settings = Process(
        Command::new(&executable)
            .arg("--settings")
            .env("ZG_P03_TEST_NAMESPACE", &namespace)
            .env("ZG_P04B1_TEST_SETTINGS_CONNECTED_MARKER", &connected)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("packaged Settings must start"),
    );
    wait_for_marker(&mut settings.0, &connected, "packaged Settings");
    assert!(
        engine.0.try_wait().unwrap().is_none(),
        "Settings connection must not stop the Engine"
    );
}
