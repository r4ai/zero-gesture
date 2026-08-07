//! macOS Accessibility context resolver.
//!
//! The Event Tap callback cannot reach this module. P04b3b starts the concrete
//! P04b3a worker/cache only beside its run-loop consumer, so context OS queries
//! exist solely for action routing and never delay the callback.

use std::sync::atomic::{fence, AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

#[cfg(target_os = "macos")]
mod native;

use super::super::owner::{ContextView, CONTEXT_MAX_AGE_MS};
use super::super::HookFailure;
use crate::config::ConfigSnapshotReader;
use crate::domain::input::TargetToken;
use crate::domain::{BindingSetId, MouseEvent, Point};

const REQUEST_PERIOD_MS: u32 = 25;
const WORKER_POLL: Duration = Duration::from_millis(10);
const READY_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_CONTEXT_UTF16_UNITS: usize = 512;
const MAX_CONTEXT_UTF8_BYTES: usize = MAX_CONTEXT_UTF16_UNITS * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextRequest {
    request_id: u64,
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
    window_fingerprint: u64,
}

#[derive(Clone, Copy)]
struct ResolvedContext {
    request_id: u64,
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

struct WorkerResources {
    reader: ConfigSnapshotReader,
    requests: Arc<RequestMailbox>,
    receiver: Receiver<()>,
    snapshots: Arc<SnapshotMailbox>,
    stop: Arc<AtomicBool>,
    preflight_complete: Arc<AtomicBool>,
}

struct RequestMailbox {
    sequence: AtomicU64,
    request_id: AtomicU64,
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
                request_id: AtomicU64::new(0),
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
        self.request_id.store(request.request_id, Ordering::Relaxed);
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
            let request_id = self.request_id.load(Ordering::Relaxed);
            let point = self.point.load(Ordering::Relaxed);
            let tick = self.tick.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == before {
                return ContextRequest {
                    request_id,
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
    request_id: AtomicU64,
    generation: AtomicU64,
    binding_set: AtomicU64,
    target: AtomicU64,
    point: AtomicU64,
    tick: AtomicU32,
    pid: AtomicI32,
    started_seconds: AtomicU64,
    started_microseconds: AtomicU64,
    window_fingerprint: AtomicU64,
}

impl SnapshotMailbox {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            valid: AtomicBool::new(false),
            request_id: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            binding_set: AtomicU64::new(0),
            target: AtomicU64::new(0),
            point: AtomicU64::new(0),
            tick: AtomicU32::new(0),
            pid: AtomicI32::new(0),
            started_seconds: AtomicU64::new(0),
            started_microseconds: AtomicU64::new(0),
            window_fingerprint: AtomicU64::new(0),
        }
    }

    fn publish(&self, context: Option<ResolvedContext>) {
        let writing = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        debug_assert_eq!(writing & 1, 1);
        if let Some(context) = context {
            self.request_id.store(context.request_id, Ordering::Relaxed);
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
            self.window_fingerprint
                .store(identity.window_fingerprint, Ordering::Relaxed);
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
            let request_id = self.request_id.load(Ordering::Relaxed);
            let generation = self.generation.load(Ordering::Relaxed);
            let binding_set = self.binding_set.load(Ordering::Relaxed);
            let target = self.target.load(Ordering::Relaxed);
            let cached_point = unpack_point(self.point.load(Ordering::Relaxed));
            let updated_tick = self.tick.load(Ordering::Relaxed);
            let pid = self.pid.load(Ordering::Relaxed);
            let started_seconds = self.started_seconds.load(Ordering::Relaxed);
            let started_microseconds = self.started_microseconds.load(Ordering::Relaxed);
            let window_fingerprint = self.window_fingerprint.load(Ordering::Relaxed);
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
                request_id,
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
                    window_fingerprint,
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
    preflight_complete: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    last_request_tick: Option<u32>,
    next_request_id: u64,
    minimum_request_id: u64,
    needed: bool,
}

impl ContextWorker {
    #[cfg(target_os = "macos")]
    pub(super) fn spawn(reader: ConfigSnapshotReader) -> Result<Self, HookFailure> {
        Self::spawn_with(reader, native::accessibility_preflight, native::resolve)
    }

    fn spawn_with<R>(
        reader: ConfigSnapshotReader,
        capability: CapabilityCheck,
        resolver: R,
    ) -> Result<Self, HookFailure>
    where
        R: Fn(&ConfigSnapshotReader, ContextRequest) -> Resolution + Send + 'static,
    {
        let (requests, receiver) = RequestMailbox::new();
        let snapshots = Arc::new(SnapshotMailbox::new());
        let stop = Arc::new(AtomicBool::new(false));
        let preflight_complete = Arc::new(AtomicBool::new(false));
        let (ready, readiness) = bounded(1);
        let thread_requests = Arc::clone(&requests);
        let thread_snapshots = Arc::clone(&snapshots);
        let thread_stop = Arc::clone(&stop);
        let thread_preflight_complete = Arc::clone(&preflight_complete);
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
                        preflight_complete: thread_preflight_complete,
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
            drop(handle);
            return Err(HookFailure::new("macOS context", "readiness timed out"));
        }
        Ok(Self {
            requests,
            snapshots,
            stop,
            preflight_complete,
            handle: Some(handle),
            last_request_tick: None,
            next_request_id: 0,
            minimum_request_id: 0,
            needed: true,
        })
    }

    pub(super) fn set_needed(&mut self, needed: bool) {
        if self.needed == needed {
            return;
        }
        self.needed = needed;
        self.last_request_tick = None;
        if !needed {
            self.minimum_request_id = self.next_request_id;
        }
    }

    pub(super) fn observe(&mut self, event: MouseEvent, point: Point, timestamp_ns: u64) {
        if !self.needed {
            return;
        }
        let tick = (timestamp_ns / 1_000_000) as u32;
        if request_due(self.last_request_tick, event, tick) {
            if self.next_request_id == u64::MAX {
                self.minimum_request_id = u64::MAX;
                return;
            }
            self.next_request_id += 1;
            self.requests.publish(ContextRequest {
                request_id: self.next_request_id,
                point,
                tick,
            });
            self.last_request_tick = Some(tick);
        }
    }

    pub(super) fn latest(&mut self, point: Point, tick: u32) -> Option<ContextView> {
        if !self.needed {
            return None;
        }
        self.snapshots
            .latest(point, tick)
            .filter(|context| context.request_id > self.minimum_request_id)
            .map(|context| context.view)
    }

    pub(super) fn shutdown(self) {}
}

impl Drop for ContextWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.wake.try_send(());
        if let Some(handle) = self.handle.take() {
            if self.preflight_complete.load(Ordering::Acquire) {
                let _ = handle.join();
            } else {
                drop(handle);
            }
        }
    }
}

fn worker_loop<R>(
    resources: WorkerResources,
    ready: Sender<()>,
    capability: CapabilityCheck,
    resolver: R,
) where
    R: Fn(&ConfigSnapshotReader, ContextRequest) -> Resolution,
{
    let _exit = WorkerExit {
        snapshots: Arc::clone(&resources.snapshots),
    };
    let _ = ready.send(());
    if resources.stop.load(Ordering::Acquire) {
        return;
    }
    let trusted = capability();
    resources.preflight_complete.store(true, Ordering::Release);
    if resources.stop.load(Ordering::Acquire) {
        return;
    }
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
        if resolution.is_ok() {
            resolved += 1;
        } else {
            unknown += 1;
        }
        publish_resolution(&resources.snapshots, resolution);
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
    match event {
        MouseEvent::ButtonDown(_) => true,
        MouseEvent::MouseMove => {
            last_tick.is_none_or(|last| tick.wrapping_sub(last) >= REQUEST_PERIOD_MS)
        }
        MouseEvent::ButtonUp(_)
        | MouseEvent::WheelUp(_)
        | MouseEvent::WheelDown(_)
        | MouseEvent::Other => false,
    }
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

fn executable_name_from_path(path: &[u8]) -> Result<String, ResolveFailure> {
    let name = path
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or(ResolveFailure::InvalidData)?;
    bounded_utf8(name, name.len(), name.len()).map(|name| name.to_lowercase())
}

fn resolve_consistent_window<W, T>(
    mut focused_window: impl FnMut() -> Result<W, ResolveFailure>,
    window_title: impl FnOnce(&W) -> Result<T, ResolveFailure>,
    equal: impl FnOnce(&W, &W) -> bool,
) -> Result<(W, T), ResolveFailure> {
    let window = focused_window()?;
    let title = window_title(&window)?;
    let current = focused_window()?;
    if !equal(&window, &current) {
        return Err(ResolveFailure::TargetExited);
    }
    Ok((window, title))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    #[cfg(target_os = "macos")]
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use crossbeam_channel::TryRecvError;

    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};
    use crate::domain::TriggerButton;

    static RELEASE_CAPABILITY: AtomicBool = AtomicBool::new(false);
    static BLOCKED_CAPABILITY_ENTERED: AtomicBool = AtomicBool::new(false);
    static RELEASE_BLOCKED_CAPABILITY: AtomicBool = AtomicBool::new(false);

    #[cfg(target_os = "macos")]
    #[derive(Debug, PartialEq)]
    enum QueryStep {
        Timeout(u32),
        Attribute(&'static str),
        Equal,
    }

    fn identity(pid: i32, started_seconds: u64, window_fingerprint: u64) -> ContextIdentity {
        ContextIdentity {
            process: ProcessIdentity {
                pid,
                started_seconds,
                started_microseconds: 7,
            },
            window_fingerprint,
        }
    }

    fn resolved(identity: ContextIdentity, point: Point, tick: u32) -> ResolvedContext {
        ResolvedContext {
            request_id: 1,
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

    fn resolved_request(identity: ContextIdentity, request: ContextRequest) -> ResolvedContext {
        ResolvedContext {
            request_id: request.request_id,
            ..resolved(identity, request.point, request.tick)
        }
    }

    fn mailbox_with_value() -> SnapshotMailbox {
        let mailbox = SnapshotMailbox::new();
        mailbox.publish(Some(resolved(identity(41, 100, 9), Point::new(1, 2), 10)));
        mailbox
    }

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::yield_now();
        }
        condition()
    }

    fn assert_failure_clears(failure: ResolveFailure) {
        let (_directory, reader) = reader();
        let first_resolution = AtomicBool::new(true);
        let mut worker = ContextWorker::spawn_with(reader, allowed, move |_, request| {
            if first_resolution.swap(false, Ordering::AcqRel) {
                Ok(resolved_request(identity(41, 100, 9), request))
            } else {
                Err(failure)
            }
        })
        .unwrap();
        let first_point = Point::new(1, 2);
        worker.observe(
            MouseEvent::ButtonDown(TriggerButton::Right),
            first_point,
            1_000_000,
        );
        assert!(wait_until(Duration::from_millis(200), || {
            worker.latest(first_point, 1).is_some()
        }));

        worker.observe(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(3, 4),
            2_000_000,
        );
        assert!(wait_until(Duration::from_millis(200), || {
            worker.latest(first_point, 1).is_none()
        }));
        worker.shutdown();
    }

    fn allowed() -> bool {
        true
    }

    fn delayed_capability() -> bool {
        while !RELEASE_CAPABILITY.load(Ordering::Acquire) {
            thread::yield_now();
        }
        true
    }

    fn blocked_capability() -> bool {
        BLOCKED_CAPABILITY_ENTERED.store(true, Ordering::Release);
        while !RELEASE_BLOCKED_CAPABILITY.load(Ordering::Acquire) {
            thread::yield_now();
        }
        true
    }

    fn fake_resolver(_: &ConfigSnapshotReader, request: ContextRequest) -> Resolution {
        Ok(resolved_request(identity(71, 200, 11), request))
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
    fn worker_readiness_precedes_the_capability_ffi_boundary() {
        RELEASE_CAPABILITY.store(false, Ordering::Release);
        thread::spawn(|| {
            thread::sleep(Duration::from_millis(300));
            RELEASE_CAPABILITY.store(true, Ordering::Release);
        });
        let (_directory, reader) = reader();

        let started = Instant::now();
        let worker = ContextWorker::spawn_with(reader, delayed_capability, fake_resolver).unwrap();

        assert!(started.elapsed() < Duration::from_millis(150));
        RELEASE_CAPABILITY.store(true, Ordering::Release);
        worker.shutdown();
    }

    #[test]
    fn blocked_capability_does_not_block_shutdown_and_releases_resources_after_return() {
        BLOCKED_CAPABILITY_ENTERED.store(false, Ordering::Release);
        RELEASE_BLOCKED_CAPABILITY.store(false, Ordering::Release);
        let (_directory, reader) = reader();
        let worker = ContextWorker::spawn_with(reader, blocked_capability, fake_resolver).unwrap();
        assert!(wait_until(Duration::from_millis(200), || {
            BLOCKED_CAPABILITY_ENTERED.load(Ordering::Acquire)
        }));
        let snapshots = Arc::downgrade(&worker.snapshots);
        let release = thread::spawn(|| {
            thread::sleep(Duration::from_millis(300));
            RELEASE_BLOCKED_CAPABILITY.store(true, Ordering::Release);
        });

        let started = Instant::now();
        worker.shutdown();

        assert!(started.elapsed() < Duration::from_millis(150));
        release.join().unwrap();
        assert!(wait_until(Duration::from_millis(200), || {
            snapshots.upgrade().is_none()
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn consistent_window_query_applies_the_exact_timed_ax_sequence() {
        assert_eq!(native::AX_MESSAGING_TIMEOUT_SECONDS, 0.05);
        assert_eq!(native::FOCUSED_WINDOW_ATTRIBUTE, "AXFocusedWindow");
        assert_eq!(native::WINDOW_TITLE_ATTRIBUTE, "AXTitle");
        let calls = RefCell::new(Vec::new());
        let result = resolve_consistent_window(
            || {
                native::timed_ax_query(
                    native::FOCUSED_WINDOW_ATTRIBUTE,
                    |timeout| {
                        calls
                            .borrow_mut()
                            .push(QueryStep::Timeout(timeout.to_bits()));
                        Ok(())
                    },
                    |attribute| {
                        calls.borrow_mut().push(QueryStep::Attribute(attribute));
                        Ok(7_u64)
                    },
                )
            },
            |_| {
                native::timed_ax_query(
                    native::WINDOW_TITLE_ATTRIBUTE,
                    |timeout| {
                        calls
                            .borrow_mut()
                            .push(QueryStep::Timeout(timeout.to_bits()));
                        Ok(())
                    },
                    |attribute| {
                        calls.borrow_mut().push(QueryStep::Attribute(attribute));
                        Ok("title")
                    },
                )
            },
            |first, current| {
                calls.borrow_mut().push(QueryStep::Equal);
                first == current
            },
        );

        assert_eq!(result.map(|(_, title)| title), Ok("title"));
        assert_eq!(
            calls.into_inner(),
            [
                QueryStep::Timeout(0.05_f32.to_bits()),
                QueryStep::Attribute("AXFocusedWindow"),
                QueryStep::Timeout(0.05_f32.to_bits()),
                QueryStep::Attribute("AXTitle"),
                QueryStep::Timeout(0.05_f32.to_bits()),
                QueryStep::Attribute("AXFocusedWindow"),
                QueryStep::Equal,
            ]
        );
    }

    #[test]
    fn focus_change_during_resolution_degrades_to_unknown() {
        let index = Cell::new(0);
        let result = resolve_consistent_window(
            || {
                let window = [7_u64, 8][index.get()];
                index.set(index.get() + 1);
                Ok(window)
            },
            |_| Ok("title"),
            |left, right| left == right,
        );

        assert_eq!(result, Err(ResolveFailure::TargetExited));
    }

    #[test]
    fn request_mailbox_is_bounded_and_coalesces_to_the_latest_request() {
        let (mailbox, wake) = RequestMailbox::new();
        mailbox.publish(ContextRequest {
            request_id: 1,
            point: Point::new(1, 1),
            tick: 1,
        });
        mailbox.publish(ContextRequest {
            request_id: 2,
            point: Point::new(2, 2),
            tick: 2,
        });
        mailbox.publish(ContextRequest {
            request_id: 3,
            point: Point::new(3, 3),
            tick: 3,
        });

        assert_eq!(wake.try_recv(), Ok(()));
        assert_eq!(wake.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(
            mailbox.latest(),
            ContextRequest {
                request_id: 3,
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
        assert_failure_clears(ResolveFailure::Accessibility);
    }

    #[test]
    fn accessibility_timeout_clears_the_latest_context() {
        #[cfg(target_os = "macos")]
        {
            let timeout = std::hint::black_box(native::AX_MESSAGING_TIMEOUT_SECONDS);
            assert!(timeout.is_sign_positive());
            assert!(timeout <= 0.05);
        }
        assert_failure_clears(ResolveFailure::Timeout);
    }

    #[test]
    fn target_exit_clears_the_latest_context() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            native::process_identity(i32::MAX),
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
    fn executable_path_keeps_a_long_complete_basename() {
        assert_eq!(
            executable_name_from_path(
                b"/Applications/Long App.app/Contents/MacOS/long-executable-name"
            ),
            Ok("long-executable-name".to_string())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn process_name_matches_the_current_executable_basename() {
        let expected = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_lowercase();

        assert_eq!(
            native::process_name(std::process::id() as i32),
            Ok(expected)
        );
    }

    #[test]
    fn mouse_move_is_rate_limited_button_down_is_immediate_and_other_events_are_rejected() {
        assert!(request_due(None, MouseEvent::MouseMove, 1_000));
        assert!(!request_due(Some(1_000), MouseEvent::MouseMove, 1_024));
        assert!(request_due(Some(1_000), MouseEvent::MouseMove, 1_025));
        assert!(request_due(
            Some(1_024),
            MouseEvent::ButtonDown(crate::domain::TriggerButton::Right),
            1_025
        ));
        assert!(!request_due(
            None,
            MouseEvent::ButtonUp(TriggerButton::Right),
            1_025
        ));
        assert!(!request_due(None, MouseEvent::WheelUp(1), 1_025));
        assert!(!request_due(None, MouseEvent::WheelDown(1), 1_025));
        assert!(!request_due(None, MouseEvent::Other, 1_025));
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
    fn window_fingerprint_is_not_used_as_unique_target_identity() {
        let process = identity(51, 100, 5);
        let other_window = ContextIdentity {
            window_fingerprint: 6,
            ..process
        };

        assert_eq!(target_token(process), target_token(other_window));
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
    fn same_tick_pre_invalidation_result_is_rejected_until_new_request_completes() {
        let (_directory, reader) = reader();
        let first_call = Arc::new(AtomicBool::new(true));
        let resolver_first_call = Arc::clone(&first_call);
        let (started, started_rx) = bounded(2);
        let (release_old, release_old_rx) = bounded(1);
        let (release_new, release_new_rx) = bounded(1);
        let mut worker = ContextWorker::spawn_with(reader, allowed, move |_, request| {
            let old = resolver_first_call.swap(false, Ordering::AcqRel);
            started.send(old).unwrap();
            if old {
                release_old_rx.recv().unwrap();
                Ok(resolved_request(identity(41, 100, 9), request))
            } else {
                release_new_rx.recv().unwrap();
                Ok(resolved_request(identity(42, 100, 9), request))
            }
        })
        .unwrap();
        let point = Point::new(12, 34);
        worker.observe(MouseEvent::MouseMove, point, 25_000_000);
        assert_eq!(
            started_rx.recv_timeout(Duration::from_millis(200)),
            Ok(true)
        );

        worker.set_needed(false);
        worker.set_needed(true);
        worker.observe(MouseEvent::MouseMove, point, 25_000_000);
        release_old.send(()).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_millis(200)),
            Ok(false)
        );
        let old_result_was_rejected = worker.latest(point, 25).is_none();

        release_new.send(()).unwrap();
        assert!(wait_until(Duration::from_millis(200), || {
            worker
                .latest(point, 25)
                .is_some_and(|context| context.target == target_token(identity(42, 100, 9)))
        }));
        assert!(old_result_was_rejected);
        worker.shutdown();
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
