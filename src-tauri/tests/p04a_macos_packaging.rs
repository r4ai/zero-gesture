#![cfg(target_os = "macos")]

use std::collections::{HashMap, HashSet};
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
    fn start(bundle: &Path, mut observe: impl FnMut(u32)) -> Self {
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
            observe(engine.id());
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        engine
    }

    fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn descendant_commands(root: u32) -> Vec<String> {
    let output = command_output("/bin/ps", &["-axo", "pid=,ppid=,comm="]);
    let mut children = HashMap::<u32, Vec<(u32, String)>>::new();
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let mut fields = line.split_whitespace();
        let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Some(parent) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        children
            .entry(parent)
            .or_default()
            .push((pid, fields.collect::<Vec<_>>().join(" ")));
    }

    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut commands = Vec::new();
    while let Some(parent) = pending.pop() {
        for (pid, command) in children.get(&parent).into_iter().flatten() {
            if seen.insert(*pid) {
                pending.push(*pid);
                commands.push(command.clone());
            }
        }
    }
    commands
}

fn content_window_count(probe: &Path, pid: u32) -> (usize, String) {
    let output = command_output(probe.to_str().unwrap(), &[&pid.to_string()]);
    (
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
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
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("window-count.swift");
    let probe = directory.path().join("window-count");
    fs::write(
        &source,
        r#"import AppKit
import CoreGraphics
import Foundation

let pid = Int32(CommandLine.arguments[1])!
let mainDisplay = CGDisplayBounds(CGMainDisplayID())
let primaryScreen = NSScreen.screens[0]
let menuBarHeight = Double(primaryScreen.frame.maxY - primaryScreen.visibleFrame.maxY)
let options: CGWindowListOption = [.optionAll, .excludeDesktopElements]
let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID)
    as? [[String: Any]] ?? []
let contentWindows = windows.filter { window in
    let owner = (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value
    let layer = (window[kCGWindowLayer as String] as? NSNumber)?.intValue
    let bounds = window[kCGWindowBounds as String] as? [String: NSNumber]
    let x = bounds?["X"]?.doubleValue ?? 0
    let y = bounds?["Y"]?.doubleValue ?? 0
    let width = bounds?["Width"]?.doubleValue ?? 0
    let height = bounds?["Height"]?.doubleValue ?? 0
    let isStatusBarBacking =
        x == Double(mainDisplay.minX) &&
        y == Double(mainDisplay.minY) &&
        width == Double(mainDisplay.width) &&
        height == menuBarHeight
    return owner == pid && layer == 0 && width > 0 && height > 0 && !isStatusBarBacking
}
let details = contentWindows.map { window in
    let name = window[kCGWindowName as String] ?? "<none>"
    let bounds = window[kCGWindowBounds as String] ?? "<none>"
    let alpha = window[kCGWindowAlpha as String] ?? "<none>"
    let onScreen = window[kCGWindowIsOnscreen as String] ?? "<none>"
    let store = window[kCGWindowStoreType as String] ?? "<none>"
    let sharing = window[kCGWindowSharingState as String] ?? "<none>"
    return "name=\(name) bounds=\(bounds) alpha=\(alpha) onScreen=\(onScreen) store=\(store) sharing=\(sharing)"
}.joined(separator: "\n")
FileHandle.standardError.write(details.data(using: .utf8)!)
print(contentWindows.count)
"#,
    )
    .unwrap();
    command_output(
        "/usr/bin/swiftc",
        &[source.to_str().unwrap(), "-o", probe.to_str().unwrap()],
    );
    let _engine = EngineProcess::start(&bundle, |pid| {
        let (count, details) = content_window_count(&probe, pid);
        assert_eq!(
            count, 0,
            "Engine owned a content window during startup:\n{details}"
        );
    });
}

#[test]
fn engine_mode_starts_no_webview_process() {
    let bundle = app_bundle();
    let _engine = EngineProcess::start(&bundle, |pid| {
        let descendants = descendant_commands(pid);
        assert!(
            descendants
                .iter()
                .all(|command| !command.to_ascii_lowercase().contains("webkit")),
            "unexpected Engine descendant processes: {descendants:?}"
        );
    });
}
