#![cfg(windows)]

use std::collections::{HashSet, VecDeque};
use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

const ENGINE_EXE: &str = env!("CARGO_BIN_EXE_zero-gesture");
const CONFIG_FILE: &str = "zero-gesture.config.json";
const START_TIMEOUT: Duration = Duration::from_secs(3);
const SETTINGS_RUNTIME_TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_NAMESPACE: AtomicU64 = AtomicU64::new(1);
static SETTINGS_TEST_LOCK: Mutex<()> = Mutex::new(());

struct EngineChild {
    child: Child,
}

impl Drop for EngineChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

struct EngineFixture {
    directory: TempDir,
    config_dir: PathBuf,
    ready_marker: PathBuf,
    worker_marker: PathBuf,
    namespace: String,
}

impl EngineFixture {
    fn new(config_bytes: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config_dir = directory.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join(CONFIG_FILE), config_bytes).unwrap();
        let namespace = format!(
            "engine-process-{}-{}",
            std::process::id(),
            NEXT_NAMESPACE.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            ready_marker: directory.path().join("ready"),
            worker_marker: directory.path().join("worker-started"),
            directory,
            config_dir,
            namespace,
        }
    }

    fn spawn(&self, fail_first_pipe: bool) -> EngineChild {
        self.spawn_with_env(fail_first_pipe.then_some(("ZG_P03_TEST_FAIL_FIRST_PIPE", "1")))
    }

    fn spawn_with_env(&self, environment: Option<(&str, &str)>) -> EngineChild {
        let mut command = Command::new(ENGINE_EXE);
        command
            .arg("--engine")
            .env("ZG_P03_TEST_CONFIG_DIR", &self.config_dir)
            .env("ZG_P03_TEST_NAMESPACE", &self.namespace)
            .env("ZG_P03_TEST_READY_MARKER", &self.ready_marker)
            .env("ZG_P03_TEST_WORKER_START_MARKER", &self.worker_marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some((name, value)) = environment {
            command.env(name, value);
        }
        EngineChild {
            child: command.spawn().unwrap(),
        }
    }

    fn spawn_settings(&self) -> EngineChild {
        self.spawn_settings_with_envs(&[])
    }

    fn spawn_settings_without_initial_window(&self) -> EngineChild {
        self.spawn_settings_with_envs(&[(
            "ZG_P05A_TEST_SKIP_SETTINGS_WINDOW",
            std::ffi::OsStr::new("1"),
        )])
    }

    fn spawn_settings_without_engine(&self) -> EngineChild {
        self.spawn_settings_with_envs(&[
            (
                "ZG_P05A_TEST_ENGINE_UNAVAILABLE_DELAY_MS",
                std::ffi::OsStr::new("750"),
            ),
            (
                "ZG_P05A_TEST_SKIP_SETTINGS_WINDOW",
                std::ffi::OsStr::new("1"),
            ),
        ])
    }

    fn spawn_settings_with_envs(&self, environments: &[(&str, &std::ffi::OsStr)]) -> EngineChild {
        let mut command = Command::new(ENGINE_EXE);
        command
            .arg("--settings")
            .env("ZG_P03_TEST_CONFIG_DIR", &self.config_dir)
            .env("ZG_P03_TEST_NAMESPACE", &self.namespace)
            .env("ZG_P05A_TEST_SKIP_AUTOSTART", "1")
            .env(
                "ZG_P04B1_TEST_SETTINGS_CONNECTED_MARKER",
                self.settings_connected_marker(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (name, value) in environments {
            command.env(name, value);
        }
        EngineChild {
            child: command.spawn().unwrap(),
        }
    }

    fn settings_connected_marker(&self) -> PathBuf {
        self.directory.path().join("settings-connected")
    }

    fn settings_exit_trigger(&self) -> PathBuf {
        self.directory.path().join("settings-exit-trigger")
    }

    fn wait_for_path(&self, child: &mut Child, path: &std::path::Path, description: &str) {
        let deadline = Instant::now() + START_TIMEOUT;
        while Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "{description} exited before signaling readiness"
            );
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{description} did not signal readiness within {START_TIMEOUT:?}");
    }

    fn wait_ready(&self, child: &mut Child) {
        let deadline = Instant::now() + START_TIMEOUT;
        while Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "Engine exited before becoming ready"
            );
            if self.ready_marker.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("Engine did not become ready within {START_TIMEOUT:?}");
    }

    fn wait_failed(&self, child: &mut Child) {
        let deadline = Instant::now() + START_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success(), "faulted Engine must exit non-zero");
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("faulted Engine did not exit within {START_TIMEOUT:?}");
    }
}

#[test]
fn first_pipe_failure_keeps_config_bytes_unchanged() {
    let original = br#"{"enabled":true}"#;
    let fixture = EngineFixture::new(original);
    let mut engine = fixture.spawn(true);
    fixture.wait_failed(&mut engine.child);

    assert_eq!(
        fs::read(fixture.config_dir.join(CONFIG_FILE)).unwrap(),
        original
    );
    assert!(fs::read_dir(&fixture.config_dir)
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("backup")));
}

#[test]
fn first_pipe_failure_does_not_start_input_workers() {
    let fixture = EngineFixture::new(br#"{"enabled":true}"#);
    let mut engine = fixture.spawn(true);
    fixture.wait_failed(&mut engine.child);

    assert!(!fixture.worker_marker.exists());
}

#[test]
fn actual_engine_child_owns_no_content_windows() {
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);

    assert_eq!(content_window_count(engine.child.id()), 0);
}

#[test]
fn actual_engine_child_starts_no_webview2_descendant() {
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);

    assert_eq!(
        webview2_descendants(engine.child.id()),
        Vec::<String>::new()
    );
}

#[test]
fn engine_and_settings_processes_coexist() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);
    let mut settings = fixture.spawn_settings();
    fixture.wait_for_path(
        &mut settings.child,
        &fixture.settings_connected_marker(),
        "Settings",
    );

    assert!(engine.child.try_wait().unwrap().is_none());
    assert!(settings.child.try_wait().unwrap().is_none());
}

#[test]
fn settings_coexistence_keeps_engine_webview_count_zero() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);
    let mut settings = fixture.spawn_settings();
    fixture.wait_for_path(
        &mut settings.child,
        &fixture.settings_connected_marker(),
        "Settings",
    );

    assert_eq!(content_window_count(engine.child.id()), 0);
    assert!(webview2_descendants(engine.child.id()).is_empty());
    assert!(settings.child.try_wait().unwrap().is_none());
}

#[test]
fn hook_install_failure_prevents_readiness_and_a_fresh_engine_restarts() {
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut failed = fixture.spawn_with_env(Some(("ZG_P03C_TEST_HOOK_INSTALL_FAILURE", "1")));
    fixture.wait_failed(&mut failed.child);
    assert!(!fixture.ready_marker.exists());

    let mut restarted = fixture.spawn(false);
    fixture.wait_ready(&mut restarted.child);
}

#[test]
fn context_worker_termination_exits_engine_and_a_fresh_engine_restarts() {
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let failure_marker = fixture.config_dir.join("fail-context");
    let marker = failure_marker.to_string_lossy().into_owned();
    let mut failed = fixture.spawn_with_env(Some(("ZG_P03C_TEST_CONTEXT_FAILURE_MARKER", &marker)));
    fixture.wait_ready(&mut failed.child);
    fs::write(&failure_marker, b"fail").unwrap();
    fixture.wait_failed(&mut failed.child);
    fs::remove_file(failure_marker).unwrap();
    fs::remove_file(&fixture.ready_marker).unwrap();

    let mut restarted = fixture.spawn(false);
    fixture.wait_ready(&mut restarted.child);
}

#[test]
fn renderer_worker_termination_exits_engine() {
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let failure_marker = fixture.config_dir.join("fail-renderer");
    let marker = failure_marker.to_string_lossy().into_owned();
    let mut failed =
        fixture.spawn_with_env(Some(("ZG_P03C_TEST_RENDERER_FAILURE_MARKER", &marker)));
    fixture.wait_ready(&mut failed.child);
    fs::write(failure_marker, b"fail").unwrap();
    fixture.wait_failed(&mut failed.child);
}

#[test]
fn concurrent_cold_settings_launches_converge_on_one_process_and_window() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);
    let first = fixture.spawn_settings_without_initial_window();
    let second = fixture.spawn_settings_without_initial_window();

    assert_concurrent_settings_converge(first, second, Some(&mut engine.child));
}

#[test]
fn concurrent_cold_settings_launches_converge_while_engine_is_unavailable() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let first = fixture.spawn_settings_without_engine();
    let second = fixture.spawn_settings_without_engine();

    assert_concurrent_settings_converge(first, second, None);
    assert!(!fixture.ready_marker.exists());
}

fn assert_concurrent_settings_converge(
    mut first: EngineChild,
    mut second: EngineChild,
    mut engine: Option<&mut Child>,
) {
    let first_process_id = first.child.id();
    let second_process_id = second.child.id();
    let deadline = Instant::now() + SETTINGS_RUNTIME_TIMEOUT;

    let (survivor, exited) = loop {
        let first_status = first.child.try_wait().unwrap();
        let second_status = second.child.try_wait().unwrap();
        match (first_status, second_status) {
            (None, Some(status)) => break (&mut first.child, status),
            (Some(status), None) => break (&mut second.child, status),
            (Some(first), Some(second)) => {
                panic!("both concurrent Settings launches exited: {first:?}, {second:?}")
            }
            (None, None) => {}
        }
        assert!(
            Instant::now() < deadline,
            "concurrent Settings launches did not converge within {SETTINGS_RUNTIME_TIMEOUT:?}; windows=({}, {})",
            u32::from(settings_window(first_process_id).is_some()),
            u32::from(settings_window(second_process_id).is_some())
        );
        thread::sleep(Duration::from_millis(20));
    };

    assert!(exited.success());
    assert!(
        u32::from(settings_window(first_process_id).is_some())
            + u32::from(settings_window(second_process_id).is_some())
            <= 1
    );
    assert!(survivor.try_wait().unwrap().is_none());
    if let Some(engine) = engine.as_mut() {
        assert!(engine.try_wait().unwrap().is_none());
    }
}

#[test]
fn second_settings_forwards_to_the_existing_window_and_exits() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);
    let mut first = fixture.spawn_settings_without_initial_window();
    fixture.wait_for_path(
        &mut first.child,
        &fixture.settings_connected_marker(),
        "first Settings",
    );
    assert!(settings_window(first.child.id()).is_none());

    let mut second = fixture.spawn_settings_without_initial_window();
    let deadline = Instant::now() + SETTINGS_RUNTIME_TIMEOUT;
    let status = loop {
        if let Some(status) = second.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "second Settings did not exit within {SETTINGS_RUNTIME_TIMEOUT:?}"
        );
        thread::sleep(Duration::from_millis(20));
    };

    assert!(status.success());
    wait_for_content_window(&mut first.child);
    assert!(first.child.try_wait().unwrap().is_none());
    assert!(engine.child.try_wait().unwrap().is_none());
}

#[test]
fn settings_exit_seam_removes_webview_and_keeps_engine_running() {
    let _settings_test = SETTINGS_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let fixture = EngineFixture::new(br#"{"enabled":false}"#);
    let mut engine = fixture.spawn(false);
    fixture.wait_ready(&mut engine.child);
    let exit_trigger = fixture.settings_exit_trigger();
    let mut settings = fixture.spawn_settings_with_envs(&[(
        "ZG_P05A_TEST_EXIT_SETTINGS_TRIGGER",
        exit_trigger.as_os_str(),
    )]);
    let settings_process_id = settings.child.id();
    wait_for_content_window(&mut settings.child);
    let webview_deadline = Instant::now() + SETTINGS_RUNTIME_TIMEOUT;
    while webview2_descendants(settings_process_id).is_empty() {
        assert!(
            settings.child.try_wait().unwrap().is_none(),
            "Settings exited before creating a WebView2 descendant"
        );
        assert!(
            Instant::now() < webview_deadline,
            "Settings did not create a WebView2 descendant"
        );
        thread::sleep(Duration::from_millis(20));
    }
    fs::write(&exit_trigger, b"exit").unwrap();

    let deadline = Instant::now() + SETTINGS_RUNTIME_TIMEOUT;
    let status = loop {
        if let Some(status) = settings.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "Settings test exit seam did not terminate the process"
        );
        thread::sleep(Duration::from_millis(20));
    };

    assert!(status.success());
    assert!(
        fixture.settings_connected_marker().exists(),
        "Settings must connect to Engine before exercising its exit seam"
    );
    let deadline = Instant::now() + START_TIMEOUT;
    while !webview2_descendants(settings_process_id).is_empty() {
        assert!(
            Instant::now() < deadline,
            "WebView2 descendants remained after Settings exited"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(engine.child.try_wait().unwrap().is_none());
}

fn wait_for_content_window(child: &mut Child) -> HWND {
    let deadline = Instant::now() + SETTINGS_RUNTIME_TIMEOUT;
    loop {
        assert!(
            child.try_wait().unwrap().is_none(),
            "process exited before creating its content window"
        );
        if let Some(window) = settings_window(child.id()) {
            return window;
        }
        assert!(
            Instant::now() < deadline,
            "process did not create its content window within {SETTINGS_RUNTIME_TIMEOUT:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn settings_window(process_id: u32) -> Option<HWND> {
    struct Context {
        process_id: u32,
        window: HWND,
    }

    unsafe extern "system" fn find_window(window: HWND, context: LPARAM) -> BOOL {
        let context = unsafe { &mut *(context as *mut Context) };
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut owner);
        }
        if owner == context.process_id {
            let mut title = [0_u16; 256];
            let title_len =
                unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
            let title = String::from_utf16_lossy(&title[..title_len as usize]);
            let mut class = [0_u16; 256];
            let class_len =
                unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
            let class = String::from_utf16_lossy(&class[..class_len as usize]);
            if title == "Zero Gesture"
                && unsafe { IsWindowVisible(window) } != 0
                && !matches!(
                    class.as_ref(),
                    "Tao Thread Event Target" | "tray_icon_app" | "IME"
                )
            {
                context.window = window;
                return 0;
            }
        }
        1
    }

    let mut context = Context {
        process_id,
        window: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(find_window),
            (&mut context as *mut Context).cast::<c_void>() as LPARAM,
        );
    }
    (!context.window.is_null()).then_some(context.window)
}

fn content_window_count(process_id: u32) -> u32 {
    struct Context {
        process_id: u32,
        count: u32,
    }

    unsafe extern "system" fn count_window(window: HWND, context: LPARAM) -> BOOL {
        let context = unsafe { &mut *(context as *mut Context) };
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut owner);
        }
        if owner == context.process_id {
            let mut class = [0_u16; 256];
            let class_len =
                unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
            let class = String::from_utf16_lossy(&class[..class_len as usize]);
            if !matches!(
                class.as_ref(),
                "Tao Thread Event Target" | "tray_icon_app" | "IME"
            ) {
                context.count += 1;
            }
        }
        1
    }

    let mut context = Context {
        process_id,
        count: 0,
    };
    unsafe {
        EnumWindows(
            Some(count_window),
            (&mut context as *mut Context).cast::<c_void>() as LPARAM,
        );
    }
    context.count
}

fn webview2_descendants(root_process_id: u32) -> Vec<String> {
    let processes = process_snapshot();
    let mut seen = HashSet::from([root_process_id]);
    let mut pending = VecDeque::from([root_process_id]);
    let mut webviews = Vec::new();
    while let Some(parent) = pending.pop_front() {
        for (process_id, _, name) in processes.iter().filter(|entry| entry.1 == parent) {
            if seen.insert(*process_id) {
                pending.push_back(*process_id);
                if name.eq_ignore_ascii_case("msedgewebview2.exe") {
                    webviews.push(name.clone());
                }
            }
        }
    }
    webviews
}

fn process_snapshot() -> Vec<(u32, u32, String)> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    assert_ne!(snapshot, INVALID_HANDLE_VALUE);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            let name_length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            processes.push((
                entry.th32ProcessID,
                entry.th32ParentProcessID,
                String::from_utf16_lossy(&entry.szExeFile[..name_length]),
            ));
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe {
        CloseHandle(snapshot);
    }
    processes
}
