//! Core Graphics Event Tap and Core Foundation run-loop ownership.

#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
#[cfg(target_os = "macos")]
use log::warn;
#[cfg(target_os = "macos")]
use objc2_core_foundation::{
    kCFRunLoopDefaultMode, CFMachPort, CFRetained, CFRunLoop, CFRunLoopMode, CFRunLoopSource,
};
#[cfg(target_os = "macos")]
use objc2_core_graphics::{
    CGEvent, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGPreflightListenEventAccess,
};

use super::super::{HookEvent, HookFailure};
#[cfg(target_os = "macos")]
use super::consumer::{drain_input, MacosInputConsumer, OwnerClock, MACOS_INPUT_STEP_FUNCTIONS};
#[cfg(target_os = "macos")]
use super::context::ContextWorker;
#[cfg(target_os = "macos")]
use super::event_tap_spec;
use super::TapState;
#[cfg(target_os = "macos")]
use crate::config::ConfigSnapshotReader;
#[cfg(target_os = "macos")]
use crate::executor::macos::{post_access_allowed, MacosActionExecutor};
use crate::hook::owner::NativeInputOwner;
#[cfg(target_os = "macos")]
use crate::overlay::macos::MacosOverlayClient;

#[cfg(target_os = "macos")]
const RUN_LOOP_SLICE_SECONDS: f64 = 0.01;
const DEGRADED_STOP_POLL: Duration = Duration::from_millis(10);
const RUN_LOOP_TIMED_OUT: i32 = 3;

#[cfg(target_os = "macos")]
extern "C" {
    fn arc4random_buf(buffer: *mut c_void, length: usize);
}

#[cfg(target_os = "macos")]
struct TapResources {
    run_loop: CFRetained<CFRunLoop>,
    source: CFRetained<CFRunLoopSource>,
    tap: CFRetained<CFMachPort>,
    mode: &'static CFRunLoopMode,
}

#[cfg(target_os = "macos")]
impl Drop for TapResources {
    fn drop(&mut self) {
        CGEvent::tap_enable(&self.tap, false);
        self.run_loop
            .remove_source(Some(&self.source), Some(self.mode));
        self.tap.invalidate();
    }
}

#[cfg(target_os = "macos")]
enum StartupMode {
    Active(TapResources),
    PermissionDenied,
    CreationFailed,
}

#[derive(Clone, Copy)]
pub(super) enum StartupFailure {
    PermissionDenied,
    CreationFailed,
}

#[cfg(target_os = "macos")]
pub(in crate::hook) fn run_loop_macos(
    reader: ConfigSnapshotReader,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
    overlay: MacosOverlayClient,
) -> Result<(), HookFailure> {
    // SAFETY: arc4random_buf accepts this initialized stack buffer and exact
    // byte length; no pointer escapes the call.
    let marker = unsafe { process_instance_marker() };
    let owner = NativeInputOwner::new(reader.clone());
    let (state, mode) = prepare_marked_state(marker, Some(owner), |state| {
        // SAFETY: the boxed state address remains stable until `resources`
        // disables and invalidates the tap before `state` is dropped.
        unsafe { start_event_tap(state) }
    });
    match mode {
        StartupMode::Active(resources) => {
            run_active(reader, stop, events, state, resources, overlay)
        }
        StartupMode::PermissionDenied => {
            dispatch_startup_failure(StartupFailure::PermissionDenied, stop, events)
        }
        StartupMode::CreationFailed => {
            dispatch_startup_failure(StartupFailure::CreationFailed, stop, events)
        }
    }
}

pub(super) fn prepare_marked_state<T>(
    marker: i64,
    owner: Option<NativeInputOwner>,
    install_tap: impl FnOnce(&TapState) -> T,
) -> (Box<TapState>, T) {
    let state = Box::new(TapState::with_owner(marker, owner));
    let tap = install_tap(state.as_ref());
    (state, tap)
}

pub(super) fn executor_marker(state: &TapState) -> i64 {
    state.marker
}

pub(super) fn dispatch_startup_failure(
    failure: StartupFailure,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    #[cfg(target_os = "macos")]
    match failure {
        StartupFailure::PermissionDenied => {
            warn!("macOS input owner is pass-through: Listen Event access is unavailable");
        }
        StartupFailure::CreationFailed => {
            warn!("macOS input owner is pass-through: event tap creation failed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = failure;
    run_degraded(stop, events)
}

fn run_degraded(stop: Arc<AtomicBool>, events: Sender<HookEvent>) -> Result<(), HookFailure> {
    publish_ready(&events)?;
    wait_for_stop(&stop);
    Ok(())
}

fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        thread::sleep(DEGRADED_STOP_POLL);
    }
}

pub(super) fn run_non_timeout_owner_step<R>(stop: &AtomicBool, resources: R) {
    drop(resources);
    wait_for_stop(stop);
}

#[cfg(target_os = "macos")]
fn run_active(
    reader: ConfigSnapshotReader,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
    state: Box<TapState>,
    resources: TapResources,
    overlay: MacosOverlayClient,
) -> Result<(), HookFailure> {
    let post_access = match post_access_ready(&state, &events, post_access_allowed) {
        Ok(available) => available,
        Err(failure) => {
            drop(resources);
            state.detach_owner();
            return Err(failure);
        }
    };
    if !post_access {
        warn!("macOS input owner is pass-through: Post Event access is unavailable");
        drop(resources);
        state.detach_owner();
        wait_for_stop(&stop);
        return Ok(());
    }
    let mut context = match ContextWorker::spawn(reader.clone()) {
        Ok(context) => context,
        Err(failure) => {
            drop(resources);
            return Err(failure);
        }
    };
    let executor = match MacosActionExecutor::spawn(executor_marker(&state)) {
        Ok(executor) => executor,
        Err(_) => {
            drop(resources);
            context.shutdown();
            return Err(HookFailure::new("macOS action", "failed to start"));
        }
    };
    let mut consumer = MacosInputConsumer::new(executor, overlay);
    let mut clock = OwnerClock::new();
    if let Err(failure) = publish_ready(&events) {
        consumer.prepare_shutdown(&state, &mut context, clock.current());
        drop(resources);
        state.detach_owner();
        consumer.finish_shutdown();
        context.shutdown();
        return Err(failure);
    }
    state.enable_active_input();
    while !stop.load(Ordering::Acquire) {
        let result = CFRunLoop::run_in_mode(Some(resources.mode), RUN_LOOP_SLICE_SECONDS, false);
        if classify_run_loop_result(result.0) == RunLoopDisposition::Degrade {
            drain_input(
                &state,
                &mut context,
                &mut consumer,
                &mut clock,
                &MACOS_INPUT_STEP_FUNCTIONS,
            );
            consumer.prepare_shutdown(&state, &mut context, clock.current());
            let renderer_failed = consumer.renderer_failed();
            drop(resources);
            state.detach_owner();
            consumer.finish_shutdown();
            context.shutdown();
            if renderer_failed {
                return Err(HookFailure::new(
                    "macOS overlay",
                    "failed during owner degrade",
                ));
            }
            wait_for_stop(&stop);
            return Ok(());
        }
        drain_input(
            &state,
            &mut context,
            &mut consumer,
            &mut clock,
            &MACOS_INPUT_STEP_FUNCTIONS,
        );
        consumer.safety_timer(&state, &mut context, clock.current());
        if consumer.renderer_failed() {
            consumer.prepare_shutdown(&state, &mut context, clock.current());
            drop(resources);
            state.detach_owner();
            consumer.finish_shutdown();
            context.shutdown();
            return Err(HookFailure::new("macOS overlay", "terminated unexpectedly"));
        }
        reenable_if_requested(&state, &resources.tap);
    }
    drain_input(
        &state,
        &mut context,
        &mut consumer,
        &mut clock,
        &MACOS_INPUT_STEP_FUNCTIONS,
    );
    consumer.prepare_shutdown(&state, &mut context, clock.current());
    let renderer_failed = consumer.renderer_failed();
    drop(resources);
    state.detach_owner();
    consumer.finish_shutdown();
    context.shutdown();
    if renderer_failed {
        Err(HookFailure::new("macOS overlay", "failed to shut down"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn reenable_if_requested(state: &TapState, tap: &CFMachPort) {
    if !state.take_reenable_request() {
        return;
    }
    state.reenable_attempts.fetch_add(1, Ordering::Relaxed);
    CGEvent::tap_enable(tap, true);
    if !CGEvent::tap_is_enabled(tap) {
        state.reenable_failures.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
unsafe fn process_instance_marker() -> i64 {
    let mut marker = 0_u64;
    // SAFETY: the pointer names the initialized local `u64` and the requested
    // length is exactly that object's size.
    unsafe {
        arc4random_buf(
            std::ptr::from_mut(&mut marker).cast::<c_void>(),
            std::mem::size_of::<u64>(),
        );
    }
    if marker == 0 {
        1
    } else {
        marker as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunLoopDisposition {
    Continue,
    Degrade,
}

pub(super) fn classify_run_loop_result(result: i32) -> RunLoopDisposition {
    if result == RUN_LOOP_TIMED_OUT {
        RunLoopDisposition::Continue
    } else {
        RunLoopDisposition::Degrade
    }
}

pub(super) fn publish_ready(events: &Sender<HookEvent>) -> Result<(), HookFailure> {
    events
        .send(HookEvent::Ready(1))
        .map_err(|_| HookFailure::new("event tap", "readiness receiver disappeared"))
}

#[cfg(target_os = "macos")]
pub(super) fn post_access_ready(
    state: &TapState,
    events: &Sender<HookEvent>,
    preflight: impl FnOnce() -> bool,
) -> Result<bool, HookFailure> {
    if preflight() {
        return Ok(true);
    }
    state.disable_active_input();
    publish_ready(events)?;
    Ok(false)
}

#[cfg(target_os = "macos")]
unsafe fn start_event_tap(state: &TapState) -> StartupMode {
    if !CGPreflightListenEventAccess() {
        return StartupMode::PermissionDenied;
    }
    let spec = event_tap_spec();
    // SAFETY: callback ABI is the generated `CGEventTapCallBack` type and
    // `state` remains at a stable boxed address through tap invalidation.
    let Some(tap) = (unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions(spec.options),
            spec.mask,
            Some(super::callback::event_tap_callback),
            std::ptr::from_ref(state).cast_mut().cast(),
        )
    }) else {
        return StartupMode::CreationFailed;
    };
    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
        tap.invalidate();
        return StartupMode::CreationFailed;
    };
    let Some(run_loop) = CFRunLoop::current() else {
        tap.invalidate();
        return StartupMode::CreationFailed;
    };
    // SAFETY: Core Foundation exports this process-lifetime mode singleton.
    let Some(mode) = (unsafe { kCFRunLoopDefaultMode }) else {
        tap.invalidate();
        return StartupMode::CreationFailed;
    };
    run_loop.add_source(Some(&source), Some(mode));
    CGEvent::tap_enable(&tap, true);
    if !CGEvent::tap_is_enabled(&tap) {
        run_loop.remove_source(Some(&source), Some(mode));
        tap.invalidate();
        return StartupMode::CreationFailed;
    }
    StartupMode::Active(TapResources {
        run_loop,
        source,
        tap,
        mode,
    })
}
