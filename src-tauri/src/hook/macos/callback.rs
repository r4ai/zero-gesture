//! Allocation-free Core Graphics callback leaf.

#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;

#[cfg(target_os = "macos")]
use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapProxy, CGEventType};

use super::{RawInput, TapState};
#[cfg(target_os = "macos")]
use super::{EVENT_OTHER_MOUSE_DOWN, EVENT_OTHER_MOUSE_UP, EVENT_SCROLL_WHEEL};
#[cfg(target_os = "macos")]
use crate::executor::macos::EVENT_FIELD_SOURCE_USER_DATA;

pub(super) fn capture_callback_event(
    state: &TapState,
    source_marker: i64,
    raw: impl FnOnce() -> RawInput,
) {
    if source_marker != state.marker {
        state.capture_raw(raw());
    }
}

#[cfg(target_os = "macos")]
pub(super) unsafe extern "C-unwind" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: `start_event_tap` keeps the boxed state alive until after the tap
    // is disabled and invalidated. Core Graphics supplies a non-null event for
    // the callback duration, as encoded by the generated callback type.
    let state = unsafe { &*user_info.cast::<TapState>() };
    if matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        state.note_disabled();
        return event.as_ptr();
    }

    // SAFETY: the generated callback type guarantees this event reference is
    // valid for the callback duration; it is never retained or stored.
    let event_ref = unsafe { event.as_ref() };
    let source_marker =
        CGEvent::integer_value_field(Some(event_ref), CGEventField(EVENT_FIELD_SOURCE_USER_DATA));
    capture_callback_event(state, source_marker, || {
        read_raw_event(event_type.0, event_ref)
    });
    event.as_ptr()
}

#[cfg(target_os = "macos")]
fn read_raw_event(event_type: u32, event: &CGEvent) -> RawInput {
    let location = CGEvent::location(Some(event));
    let button = if matches!(event_type, EVENT_OTHER_MOUSE_DOWN | EVENT_OTHER_MOUSE_UP) {
        CGEvent::integer_value_field(Some(event), CGEventField::MouseEventButtonNumber)
    } else {
        0
    };
    let scroll = if event_type == EVENT_SCROLL_WHEEL {
        CGEvent::integer_value_field(Some(event), CGEventField::ScrollWheelEventDeltaAxis1)
    } else {
        0
    };
    RawInput {
        event_type,
        button,
        scroll,
        x: location.x,
        y: location.y,
        timestamp_ns: CGEvent::timestamp(Some(event)),
    }
}
