#![cfg(windows)]

use std::collections::{HashSet, VecDeque};
use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId,
};

const ENGINE_EXE: &str = env!("CARGO_BIN_EXE_zero-gesture");
const CONFIG_FILE: &str = "zero-gesture.config.json";
const START_TIMEOUT: Duration = Duration::from_secs(3);
static NEXT_NAMESPACE: AtomicU64 = AtomicU64::new(1);

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
    _directory: TempDir,
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
            _directory: directory,
            config_dir,
            namespace,
        }
    }

    fn spawn(&self, fail_first_pipe: bool) -> EngineChild {
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
        if fail_first_pipe {
            command.env("ZG_P03_TEST_FAIL_FIRST_PIPE", "1");
        }
        EngineChild {
            child: command.spawn().unwrap(),
        }
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
