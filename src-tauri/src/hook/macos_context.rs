//! macOS Accessibility context resolver.
//!
//! The Event Tap callback cannot reach this module. Its run-loop owner submits
//! coalesced observations to one worker, which publishes one numeric snapshot.

use std::sync::atomic::{fence, AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use super::owner::{ContextView, CONTEXT_MAX_AGE_MS};
use super::HookFailure;
use crate::config::ConfigSnapshotReader;
use crate::domain::input::TargetToken;
use crate::domain::{BindingSetId, MouseEvent, Point};

const REQUEST_PERIOD_MS: u32 = 25;
const WORKER_POLL: Duration = Duration::from_millis(10);
const READY_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_CONTEXT_UTF16_UNITS: usize = 512;
const MAX_CONTEXT_UTF8_BYTES: usize = MAX_CONTEXT_UTF16_UNITS * 4;
#[cfg(target_os = "macos")]
const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 0.05;
#[cfg(target_os = "macos")]
const AX_ERROR_CANNOT_COMPLETE: i32 = -25204;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextRequest {
    point: Point,
    tick: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessIdentity {
    pid: i32,
    started_seconds: u64,
    started_microseconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextIdentity {
    process: ProcessIdentity,
    window_hash: u64,
}

#[derive(Clone, Copy)]
struct ResolvedContext {
    view: ContextView,
    identity: ContextIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolveFailure {
    PermissionDenied,
    Accessibility,
    Timeout,
    TargetExited,
    InvalidData,
    Oversized,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Unavailable,
}

type Resolution = Result<ResolvedContext, ResolveFailure>;
type CapabilityCheck = fn() -> bool;
type Resolver = fn(&ConfigSnapshotReader, ContextRequest) -> Resolution;

struct WorkerResources {
    reader: ConfigSnapshotReader,
    requests: Arc<RequestMailbox>,
    receiver: Receiver<()>,
    snapshots: Arc<SnapshotMailbox>,
    stop: Arc<AtomicBool>,
}

struct RequestMailbox {
    sequence: AtomicU64,
    point: AtomicU64,
    tick: AtomicU32,
    wake: Sender<()>,
}

impl RequestMailbox {
    fn new() -> (Arc<Self>, Receiver<()>) {
        let (wake, receiver) = bounded(1);
        (
            Arc::new(Self {
                sequence: AtomicU64::new(0),
                point: AtomicU64::new(0),
                tick: AtomicU32::new(0),
                wake,
            }),
            receiver,
        )
    }

    fn publish(&self, request: ContextRequest) {
        let writing = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        debug_assert_eq!(writing & 1, 1);
        self.point
            .store(pack_point(request.point), Ordering::Relaxed);
        self.tick.store(request.tick, Ordering::Relaxed);
        self.sequence.store(writing + 1, Ordering::Release);
        let _ = self.wake.try_send(());
    }

    fn latest(&self) -> ContextRequest {
        loop {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let point = self.point.load(Ordering::Relaxed);
            let tick = self.tick.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == before {
                return ContextRequest {
                    point: unpack_point(point),
                    tick,
                };
            }
        }
    }
}

struct SnapshotMailbox {
    sequence: AtomicU64,
    valid: AtomicBool,
    generation: AtomicU64,
    binding_set: AtomicU64,
    target: AtomicU64,
    point: AtomicU64,
    tick: AtomicU32,
    pid: AtomicI32,
    started_seconds: AtomicU64,
    started_microseconds: AtomicU64,
    window_hash: AtomicU64,
}

impl SnapshotMailbox {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            valid: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            binding_set: AtomicU64::new(0),
            target: AtomicU64::new(0),
            point: AtomicU64::new(0),
            tick: AtomicU32::new(0),
            pid: AtomicI32::new(0),
            started_seconds: AtomicU64::new(0),
            started_microseconds: AtomicU64::new(0),
            window_hash: AtomicU64::new(0),
        }
    }

    fn publish(&self, context: Option<ResolvedContext>) {
        let writing = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        debug_assert_eq!(writing & 1, 1);
        if let Some(context) = context {
            let view = context.view;
            let identity = context.identity;
            self.generation.store(view.generation, Ordering::Relaxed);
            self.binding_set
                .store(view.binding_set.index() as u64, Ordering::Relaxed);
            self.target.store(view.target.0, Ordering::Relaxed);
            self.point.store(pack_point(view.point), Ordering::Relaxed);
            self.tick.store(view.updated_tick, Ordering::Relaxed);
            self.pid.store(identity.process.pid, Ordering::Relaxed);
            self.started_seconds
                .store(identity.process.started_seconds, Ordering::Relaxed);
            self.started_microseconds
                .store(identity.process.started_microseconds, Ordering::Relaxed);
            self.window_hash
                .store(identity.window_hash, Ordering::Relaxed);
            self.valid.store(true, Ordering::Relaxed);
        } else {
            self.valid.store(false, Ordering::Relaxed);
        }
        self.sequence.store(writing + 1, Ordering::Release);
    }

    fn latest(&self, point: Point, tick: u32) -> Option<ResolvedContext> {
        for _ in 0..4 {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let valid = self.valid.load(Ordering::Relaxed);
            let generation = self.generation.load(Ordering::Relaxed);
            let binding_set = self.binding_set.load(Ordering::Relaxed);
            let target = self.target.load(Ordering::Relaxed);
            let cached_point = unpack_point(self.point.load(Ordering::Relaxed));
            let updated_tick = self.tick.load(Ordering::Relaxed);
            let pid = self.pid.load(Ordering::Relaxed);
            let started_seconds = self.started_seconds.load(Ordering::Relaxed);
            let started_microseconds = self.started_microseconds.load(Ordering::Relaxed);
            let window_hash = self.window_hash.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) != before {
                continue;
            }
            let binding_set = BindingSetId::from_index(binding_set as usize)?;
            if !valid
                || cached_point != point
                || tick.wrapping_sub(updated_tick) > CONTEXT_MAX_AGE_MS
            {
                return None;
            }
            return Some(ResolvedContext {
                view: ContextView {
                    generation,
                    binding_set,
                    target: TargetToken(target),
                    point: cached_point,
                    updated_tick,
                },
                identity: ContextIdentity {
                    process: ProcessIdentity {
                        pid,
                        started_seconds,
                        started_microseconds,
                    },
                    window_hash,
                },
            });
        }
        None
    }
}

struct WorkerExit {
    snapshots: Arc<SnapshotMailbox>,
}

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.snapshots.publish(None);
    }
}

pub(super) struct ContextWorker {
    requests: Arc<RequestMailbox>,
    snapshots: Arc<SnapshotMailbox>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    last_request_tick: Option<u32>,
}

impl ContextWorker {
    #[cfg(target_os = "macos")]
    pub(super) fn spawn(reader: ConfigSnapshotReader) -> Result<Self, HookFailure> {
        Self::spawn_with(reader, accessibility_preflight, resolve_native)
    }

    fn spawn_with(
        reader: ConfigSnapshotReader,
        capability: CapabilityCheck,
        resolver: Resolver,
    ) -> Result<Self, HookFailure> {
        let (requests, receiver) = RequestMailbox::new();
        let snapshots = Arc::new(SnapshotMailbox::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, readiness) = bounded(1);
        let thread_requests = Arc::clone(&requests);
        let thread_snapshots = Arc::clone(&snapshots);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("macos-context".to_string())
            .spawn(move || {
                worker_loop(
                    WorkerResources {
                        reader,
                        requests: thread_requests,
                        receiver,
                        snapshots: thread_snapshots,
                        stop: thread_stop,
                    },
                    ready,
                    capability,
                    resolver,
                );
            })
            .map_err(|_| HookFailure::new("macOS context", "failed to start"))?;
        if readiness.recv_timeout(READY_TIMEOUT).is_err() {
            stop.store(true, Ordering::Release);
            let _ = requests.wake.try_send(());
            let _ = handle.join();
            return Err(HookFailure::new("macOS context", "readiness timed out"));
        }
        Ok(Self {
            requests,
            snapshots,
            stop,
            handle: Some(handle),
            last_request_tick: None,
        })
    }

    pub(super) fn observe(&mut self, event: MouseEvent, point: Point, timestamp_ns: u64) {
        let tick = (timestamp_ns / 1_000_000) as u32;
        if request_due(self.last_request_tick, event, tick) {
            self.requests.publish(ContextRequest { point, tick });
            self.last_request_tick = Some(tick);
        }
    }

    #[cfg(test)]
    fn latest(&self, point: Point, tick: u32) -> Option<ResolvedContext> {
        self.snapshots.latest(point, tick)
    }

    pub(super) fn shutdown(self) {}
}

impl Drop for ContextWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.wake.try_send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(
    resources: WorkerResources,
    ready: Sender<()>,
    capability: CapabilityCheck,
    resolver: Resolver,
) {
    let _exit = WorkerExit {
        snapshots: Arc::clone(&resources.snapshots),
    };
    let trusted = capability();
    let _ = ready.send(());
    let mut resolved = 0_u64;
    let mut unknown = 0_u64;
    while !resources.stop.load(Ordering::Acquire) {
        match resources.receiver.recv_timeout(WORKER_POLL) {
            Ok(()) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
        if resources.stop.load(Ordering::Acquire) {
            break;
        }
        while matches!(resources.receiver.try_recv(), Ok(())) {}
        let started = Instant::now();
        let resolution = if trusted {
            let resolution = resolver(&resources.reader, resources.requests.latest());
            discard_slow_resolution(resolution, started.elapsed())
        } else {
            Err(ResolveFailure::PermissionDenied)
        };
        match resolution {
            Ok(context) => {
                resources.snapshots.publish(Some(context));
                resolved += 1;
            }
            Err(_) => {
                resources.snapshots.publish(None);
                unknown += 1;
            }
        }
    }
    #[cfg(target_os = "macos")]
    log::info!("macOS context worker stopped (resolved={resolved}, unknown={unknown})");
    #[cfg(not(target_os = "macos"))]
    let _ = (resolved, unknown);
}

fn publish_resolution(mailbox: &SnapshotMailbox, resolution: Resolution) {
    mailbox.publish(resolution.ok());
}

fn discard_slow_resolution(resolution: Resolution, elapsed: Duration) -> Resolution {
    if elapsed > Duration::from_millis(u64::from(CONTEXT_MAX_AGE_MS)) {
        Err(ResolveFailure::Timeout)
    } else {
        resolution
    }
}

fn request_due(last_tick: Option<u32>, event: MouseEvent, tick: u32) -> bool {
    matches!(event, MouseEvent::ButtonDown(_))
        || last_tick.is_none_or(|last| tick.wrapping_sub(last) >= REQUEST_PERIOD_MS)
}

fn pack_point(point: Point) -> u64 {
    ((point.x as u32 as u64) << 32) | u64::from(point.y as u32)
}

fn unpack_point(value: u64) -> Point {
    Point::new((value >> 32) as u32 as i32, value as u32 as i32)
}

fn target_token(identity: ContextIdentity) -> TargetToken {
    let process = identity.process;
    let mut value = process.pid as u32 as u64;
    value ^= process.started_seconds.rotate_left(13);
    value ^= process.started_microseconds.rotate_left(29);
    value ^= identity.window_hash.rotate_left(47);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    TargetToken(value.wrapping_mul(0x94d0_49bb_1331_11eb) ^ (value >> 31))
}

fn bounded_utf8(
    bytes: &[u8],
    converted_units: usize,
    expected_units: usize,
) -> Result<String, ResolveFailure> {
    if expected_units > MAX_CONTEXT_UTF16_UNITS || bytes.len() > MAX_CONTEXT_UTF8_BYTES {
        return Err(ResolveFailure::Oversized);
    }
    if converted_units != expected_units {
        return Err(ResolveFailure::InvalidData);
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| ResolveFailure::InvalidData)
}

#[cfg(target_os = "macos")]
fn accessibility_preflight_options() -> *const std::ffi::c_void {
    std::ptr::null()
}

#[cfg(all(test, not(target_os = "macos")))]
fn accessibility_preflight_options() -> *const std::ffi::c_void {
    std::ptr::null()
}

#[cfg(target_os = "macos")]
fn accessibility_preflight() -> bool {
    unsafe { AXIsProcessTrustedWithOptions(accessibility_preflight_options()) != 0 }
}

#[cfg(target_os = "macos")]
fn resolve_native(reader: &ConfigSnapshotReader, request: ContextRequest) -> Resolution {
    let snapshot = reader.read().ok_or(ResolveFailure::Unavailable)?;
    if !snapshot.enabled {
        return Err(ResolveFailure::Unavailable);
    }
    let observation = unsafe { native_observation()? };
    let info = crate::window_info::ForegroundWindowInfo {
        process_name: Some(observation.process_name),
        window_class: None,
        title: Some(observation.title),
        bundle_identifier: observation.bundle_identifier,
    };
    let binding_set = snapshot
        .match_macos_app(&info)
        .unwrap_or_else(|| snapshot.default_binding_set());
    let identity = ContextIdentity {
        process: observation.process,
        window_hash: observation.window_hash,
    };
    Ok(ResolvedContext {
        view: ContextView {
            generation: snapshot.generation(),
            binding_set,
            target: target_token(identity),
            point: request.point,
            updated_tick: request.tick,
        },
        identity,
    })
}

#[cfg(target_os = "macos")]
struct NativeObservation {
    process: ProcessIdentity,
    window_hash: u64,
    process_name: String,
    bundle_identifier: Option<String>,
    title: String,
}

#[cfg(target_os = "macos")]
struct OwnedCf(*const std::ffi::c_void);

#[cfg(target_os = "macos")]
impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

#[cfg(target_os = "macos")]
struct AutoreleasePool(*mut std::ffi::c_void);

#[cfg(target_os = "macos")]
impl AutoreleasePool {
    unsafe fn new() -> Result<Self, ResolveFailure> {
        let class = objc_getClass(c"NSAutoreleasePool".as_ptr());
        let allocated = send_id(class, selector(b"alloc\0"));
        let pool = send_id(allocated, selector(b"init\0"));
        if pool.is_null() {
            Err(ResolveFailure::Unavailable)
        } else {
            Ok(Self(pool))
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            send_void(self.0, selector(b"drain\0"));
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn native_observation() -> Result<NativeObservation, ResolveFailure> {
    let _pool = AutoreleasePool::new()?;
    let (pid, process, process_name, bundle_identifier) = frontmost_process()?;
    let window = focused_window(pid)?;
    let title_value =
        copy_ax_attribute(window.0.cast_mut(), kAXTitleAttribute, CFStringGetTypeID())?;
    let title = copy_cf_string(title_value.0)?;
    let window_hash = CFHash(window.0) as u64;
    verify_frontmost_identity(pid, process)?;
    Ok(NativeObservation {
        process,
        window_hash,
        process_name,
        bundle_identifier,
        title,
    })
}

#[cfg(target_os = "macos")]
unsafe fn frontmost_process(
) -> Result<(i32, ProcessIdentity, String, Option<String>), ResolveFailure> {
    let application = frontmost_application()?;
    let pid = send_pid(application, selector(b"processIdentifier\0"));
    if pid <= 0 {
        return Err(ResolveFailure::TargetExited);
    }
    let process = process_identity(pid)?;
    let process_name = process_name(pid)?;
    let bundle = send_id(application, selector(b"bundleIdentifier\0"));
    let bundle_identifier = if bundle.is_null() {
        None
    } else {
        Some(copy_cf_string(bundle.cast_const())?)
    };
    Ok((pid, process, process_name, bundle_identifier))
}

#[cfg(target_os = "macos")]
unsafe fn focused_window(pid: i32) -> Result<OwnedCf, ResolveFailure> {
    let ax_application = OwnedCf(AXUIElementCreateApplication(pid).cast_const());
    if ax_application.0.is_null() {
        return Err(ResolveFailure::TargetExited);
    }
    require_ax(AXUIElementSetMessagingTimeout(
        ax_application.0.cast_mut(),
        AX_MESSAGING_TIMEOUT_SECONDS,
    ))?;
    let window = copy_ax_attribute(
        ax_application.0.cast_mut(),
        kAXFocusedWindowAttribute,
        AXUIElementGetTypeID(),
    )?;
    require_ax(AXUIElementSetMessagingTimeout(
        window.0.cast_mut(),
        AX_MESSAGING_TIMEOUT_SECONDS,
    ))?;
    let mut window_pid = 0;
    require_ax(AXUIElementGetPid(window.0.cast_mut(), &mut window_pid))?;
    if window_pid != pid {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(window)
}

#[cfg(target_os = "macos")]
unsafe fn verify_frontmost_identity(
    pid: i32,
    process: ProcessIdentity,
) -> Result<(), ResolveFailure> {
    if process_identity(pid)? != process
        || send_pid(frontmost_application()?, selector(b"processIdentifier\0")) != pid
    {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe fn frontmost_application() -> Result<*mut std::ffi::c_void, ResolveFailure> {
    let workspace_class = objc_getClass(c"NSWorkspace".as_ptr());
    let workspace = send_id(workspace_class, selector(b"sharedWorkspace\0"));
    let application = send_id(workspace, selector(b"frontmostApplication\0"));
    if application.is_null() {
        Err(ResolveFailure::Unavailable)
    } else {
        Ok(application)
    }
}

#[cfg(target_os = "macos")]
unsafe fn process_identity(pid: i32) -> Result<ProcessIdentity, ResolveFailure> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let expected = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let actual = libc::proc_pidinfo(
        pid,
        libc::PROC_PIDTBSDINFO,
        0,
        info.as_mut_ptr().cast(),
        expected,
    );
    if actual != expected {
        return Err(ResolveFailure::TargetExited);
    }
    let info = info.assume_init();
    if info.pbi_pid != pid as u32 {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(ProcessIdentity {
        pid,
        started_seconds: info.pbi_start_tvsec,
        started_microseconds: info.pbi_start_tvusec,
    })
}

#[cfg(target_os = "macos")]
unsafe fn process_name(pid: i32) -> Result<String, ResolveFailure> {
    let mut bytes = [0_u8; MAX_CONTEXT_UTF8_BYTES + 1];
    let length = libc::proc_name(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32);
    if length <= 0 {
        return Err(ResolveFailure::TargetExited);
    }
    let length = length as usize;
    let bytes = bytes
        .get(..length)
        .ok_or(ResolveFailure::Oversized)?
        .split(|byte| *byte == 0)
        .next()
        .ok_or(ResolveFailure::InvalidData)?;
    bounded_utf8(bytes, bytes.len(), bytes.len()).map(|name| name.to_lowercase())
}

#[cfg(target_os = "macos")]
unsafe fn copy_ax_attribute(
    element: *mut std::ffi::c_void,
    attribute: *const std::ffi::c_void,
    expected_type: usize,
) -> Result<OwnedCf, ResolveFailure> {
    let mut value = std::ptr::null();
    require_ax(AXUIElementCopyAttributeValue(
        element, attribute, &mut value,
    ))?;
    if value.is_null() {
        return Err(ResolveFailure::InvalidData);
    }
    let value = OwnedCf(value);
    if CFGetTypeID(value.0) != expected_type {
        return Err(ResolveFailure::InvalidData);
    }
    Ok(value)
}

#[cfg(target_os = "macos")]
unsafe fn copy_cf_string(value: *const std::ffi::c_void) -> Result<String, ResolveFailure> {
    if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
        return Err(ResolveFailure::InvalidData);
    }
    let length = CFStringGetLength(value);
    if length < 0 || length as usize > MAX_CONTEXT_UTF16_UNITS {
        return Err(ResolveFailure::Oversized);
    }
    let mut bytes = [0_u8; MAX_CONTEXT_UTF8_BYTES];
    let mut used = 0_isize;
    let converted = CFStringGetBytes(
        value,
        CFRange {
            location: 0,
            length,
        },
        0x0800_0100,
        0,
        0,
        bytes.as_mut_ptr(),
        bytes.len() as isize,
        &mut used,
    );
    if used < 0 {
        return Err(ResolveFailure::InvalidData);
    }
    bounded_utf8(
        bytes
            .get(..used as usize)
            .ok_or(ResolveFailure::InvalidData)?,
        converted as usize,
        length as usize,
    )
}

#[cfg(target_os = "macos")]
fn require_ax(error: i32) -> Result<(), ResolveFailure> {
    match error {
        0 => Ok(()),
        AX_ERROR_CANNOT_COMPLETE => Err(ResolveFailure::Timeout),
        _ => Err(ResolveFailure::Accessibility),
    }
}

#[cfg(target_os = "macos")]
unsafe fn selector(name: &[u8]) -> *const std::ffi::c_void {
    sel_registerName(name.as_ptr().cast())
}

#[cfg(target_os = "macos")]
unsafe fn send_id(
    receiver: *mut std::ffi::c_void,
    selector: *const std::ffi::c_void,
) -> *mut std::ffi::c_void {
    let send: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void = std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector)
}

#[cfg(target_os = "macos")]
unsafe fn send_pid(receiver: *mut std::ffi::c_void, selector: *const std::ffi::c_void) -> i32 {
    let send: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> i32 =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector)
}

#[cfg(target_os = "macos")]
unsafe fn send_void(receiver: *mut std::ffi::c_void, selector: *const std::ffi::c_void) {
    let send: unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) =
        std::mem::transmute(objc_msgSend as *const ());
    send(receiver, selector);
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
    fn objc_msgSend();
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXFocusedWindowAttribute: *const std::ffi::c_void;
    static kAXTitleAttribute: *const std::ffi::c_void;

    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *mut std::ffi::c_void;
    fn AXUIElementSetMessagingTimeout(element: *mut std::ffi::c_void, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *mut std::ffi::c_void,
        attribute: *const std::ffi::c_void,
        value: *mut *const std::ffi::c_void,
    ) -> i32;
    fn AXUIElementGetPid(element: *mut std::ffi::c_void, pid: *mut i32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const std::ffi::c_void);
    fn CFGetTypeID(value: *const std::ffi::c_void) -> usize;
    fn CFHash(value: *const std::ffi::c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: *const std::ffi::c_void) -> isize;
    fn CFStringGetBytes(
        value: *const std::ffi::c_void,
        range: CFRange,
        encoding: u32,
        loss_byte: u8,
        external_representation: u8,
        buffer: *mut u8,
        max_buffer_length: isize,
        used_buffer_length: *mut isize,
    ) -> isize;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crossbeam_channel::TryRecvError;

    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};

    fn identity(pid: i32, started_seconds: u64, window_hash: u64) -> ContextIdentity {
        ContextIdentity {
            process: ProcessIdentity {
                pid,
                started_seconds,
                started_microseconds: 7,
            },
            window_hash,
        }
    }

    fn resolved(identity: ContextIdentity, point: Point, tick: u32) -> ResolvedContext {
        ResolvedContext {
            view: ContextView {
                generation: 3,
                binding_set: BindingSetId::from_index(1).unwrap(),
                target: target_token(identity),
                point,
                updated_tick: tick,
            },
            identity,
        }
    }

    fn mailbox_with_value() -> SnapshotMailbox {
        let mailbox = SnapshotMailbox::new();
        mailbox.publish(Some(resolved(identity(41, 100, 9), Point::new(1, 2), 10)));
        mailbox
    }

    fn assert_failure_clears(failure: ResolveFailure) {
        let mailbox = mailbox_with_value();
        publish_resolution(&mailbox, Err(failure));
        assert!(mailbox.latest(Point::new(1, 2), 10).is_none());
    }

    fn allowed() -> bool {
        true
    }

    fn fake_resolver(_: &ConfigSnapshotReader, request: ContextRequest) -> Resolution {
        Ok(resolved(identity(71, 200, 11), request.point, request.tick))
    }

    fn reader() -> (tempfile::TempDir, ConfigSnapshotReader) {
        let directory = tempfile::tempdir().unwrap();
        config::save_atomic(
            &config::ActiveConfig::from_document(ConfigDocument::default()).unwrap(),
            directory.path(),
        )
        .unwrap();
        let (owner, _) = ConfigOwner::startup(directory.path());
        (directory, owner.reader())
    }

    #[test]
    fn accessibility_preflight_uses_no_prompt_options_dictionary() {
        assert!(accessibility_preflight_options().is_null());
    }

    #[test]
    fn request_mailbox_is_bounded_and_coalesces_to_the_latest_request() {
        let (mailbox, wake) = RequestMailbox::new();
        mailbox.publish(ContextRequest {
            point: Point::new(1, 1),
            tick: 1,
        });
        mailbox.publish(ContextRequest {
            point: Point::new(2, 2),
            tick: 2,
        });
        mailbox.publish(ContextRequest {
            point: Point::new(3, 3),
            tick: 3,
        });

        assert_eq!(wake.try_recv(), Ok(()));
        assert_eq!(wake.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(
            mailbox.latest(),
            ContextRequest {
                point: Point::new(3, 3),
                tick: 3,
            }
        );
    }

    #[test]
    fn permission_denial_clears_the_latest_context() {
        assert_failure_clears(ResolveFailure::PermissionDenied);
    }

    #[test]
    fn accessibility_error_clears_the_latest_context() {
        #[cfg(target_os = "macos")]
        assert_eq!(require_ax(-25201), Err(ResolveFailure::Accessibility));
        assert_failure_clears(ResolveFailure::Accessibility);
    }

    #[test]
    fn accessibility_timeout_clears_the_latest_context() {
        #[cfg(target_os = "macos")]
        {
            let timeout = std::hint::black_box(AX_MESSAGING_TIMEOUT_SECONDS);
            assert!(timeout.is_sign_positive());
            assert!(timeout <= 0.05);
            assert_eq!(
                require_ax(AX_ERROR_CANNOT_COMPLETE),
                Err(ResolveFailure::Timeout)
            );
        }
        assert_failure_clears(ResolveFailure::Timeout);
    }

    #[test]
    fn target_exit_clears_the_latest_context() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            unsafe { process_identity(i32::MAX) },
            Err(ResolveFailure::TargetExited)
        );
        assert_failure_clears(ResolveFailure::TargetExited);
    }

    #[test]
    fn oversized_context_string_is_rejected_before_publication() {
        let bytes = vec![b'a'; MAX_CONTEXT_UTF8_BYTES + 1];
        assert_eq!(
            bounded_utf8(&bytes, bytes.len(), bytes.len()),
            Err(ResolveFailure::Oversized)
        );
        assert_failure_clears(ResolveFailure::Oversized);
    }

    #[test]
    fn malformed_context_string_is_rejected_before_publication() {
        assert_eq!(
            bounded_utf8(&[0xff], 1, 1),
            Err(ResolveFailure::InvalidData)
        );
        assert_failure_clears(ResolveFailure::InvalidData);
    }

    #[test]
    fn mouse_move_requests_are_rate_limited_before_worker_submission() {
        assert!(request_due(None, MouseEvent::MouseMove, 1_000));
        assert!(!request_due(Some(1_000), MouseEvent::MouseMove, 1_024));
        assert!(request_due(Some(1_000), MouseEvent::MouseMove, 1_025));
        assert!(request_due(
            Some(1_024),
            MouseEvent::ButtonDown(crate::domain::TriggerButton::Right),
            1_025
        ));
    }

    #[test]
    fn same_pid_with_a_new_process_identity_replaces_the_cached_window() {
        let mailbox = SnapshotMailbox::new();
        let point = Point::new(8, 9);
        let first = resolved(identity(51, 100, 5), point, 20);
        let second = resolved(identity(51, 101, 6), point, 21);
        mailbox.publish(Some(first));
        mailbox.publish(Some(second));

        let current = mailbox.latest(point, 21).unwrap();
        assert_eq!(current.identity, second.identity);
        assert_ne!(current.view.target, first.view.target);
    }

    #[test]
    fn cache_requires_exact_point_and_bounded_freshness() {
        let mailbox = mailbox_with_value();
        assert!(mailbox.latest(Point::new(1, 2), 110).is_some());
        assert!(mailbox.latest(Point::new(1, 2), 111).is_none());
        assert!(mailbox.latest(Point::new(2, 2), 10).is_none());
    }

    #[test]
    fn worker_delay_beyond_freshness_invalidates_the_latest_context() {
        let mailbox = mailbox_with_value();
        let next = resolved(identity(41, 100, 10), Point::new(1, 2), 20);
        publish_resolution(
            &mailbox,
            discard_slow_resolution(
                Ok(next),
                Duration::from_millis(u64::from(CONTEXT_MAX_AGE_MS) + 1),
            ),
        );

        assert!(mailbox.latest(Point::new(1, 2), 20).is_none());
    }

    #[test]
    fn resolver_worker_stops_deterministically_and_invalidates_its_snapshot() {
        let (_directory, reader) = reader();
        let mut worker = ContextWorker::spawn_with(reader, allowed, fake_resolver).unwrap();
        let point = Point::new(12, 34);
        worker.observe(MouseEvent::MouseMove, point, 25_000_000);
        let deadline = Instant::now() + Duration::from_millis(200);
        while worker.latest(point, 25).is_none() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(worker.latest(point, 25).is_some());
        let snapshots = Arc::clone(&worker.snapshots);

        let started = Instant::now();
        worker.shutdown();

        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(snapshots.latest(point, 25).is_none());
    }
}
