//! Native macOS context observation behind the `ContextWorker` seam.
//!
//! Generated objc2 bindings own AppKit, Accessibility, and Core Foundation
//! values. Only process identity and executable-path lookup remain on libc.

use std::ffi::CStr;
use std::ptr::NonNull;

use objc2::rc::{autoreleasepool, Retained};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_application_services::{AXError, AXIsProcessTrustedWithOptions, AXUIElement};
use objc2_core_foundation::{
    CFDictionary, CFEqual, CFHash, CFRange, CFRetained, CFString, CFStringBuiltInEncodings, CFType,
    ConcreteType,
};

use super::{
    bounded_utf8, executable_name_from_path, resolve_consistent_window, target_token,
    ContextIdentity, ContextRequest, ContextView, ProcessIdentity, Resolution, ResolveFailure,
    ResolvedContext, MAX_CONTEXT_UTF16_UNITS, MAX_CONTEXT_UTF8_BYTES,
};
use crate::config::ConfigSnapshotReader;

pub(super) const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 0.05;
pub(super) const FOCUSED_WINDOW_ATTRIBUTE: &str = "AXFocusedWindow";
pub(super) const WINDOW_TITLE_ATTRIBUTE: &str = "AXTitle";

struct NativeObservation {
    process: ProcessIdentity,
    window_fingerprint: u64,
    process_name: String,
    bundle_identifier: Option<String>,
    title: String,
}

fn accessibility_preflight_options() -> Option<&'static CFDictionary> {
    None
}

pub(super) fn accessibility_preflight() -> bool {
    // SAFETY: `None` is the documented no-options form and cannot request the
    // Accessibility trust prompt.
    unsafe { AXIsProcessTrustedWithOptions(accessibility_preflight_options()) }
}

pub(super) fn resolve(reader: &ConfigSnapshotReader, request: ContextRequest) -> Resolution {
    let snapshot = reader.read().ok_or(ResolveFailure::Unavailable)?;
    if !snapshot.enabled {
        return Err(ResolveFailure::Unavailable);
    }
    let observation = autoreleasepool(|_| native_observation())?;
    let binding_set = snapshot
        .match_macos_app(
            &observation.process_name,
            observation.bundle_identifier.as_deref(),
            &observation.title,
        )
        .unwrap_or_else(|| snapshot.default_binding_set());
    let identity = ContextIdentity {
        process: observation.process,
        window_fingerprint: observation.window_fingerprint,
    };
    Ok(ResolvedContext {
        request_id: request.request_id,
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

fn native_observation() -> Result<NativeObservation, ResolveFailure> {
    let (pid, process, process_name, bundle_identifier) = frontmost_process()?;
    let (window, title) = resolve_consistent_window(
        || focused_window(pid),
        |window| window_title(window),
        |first, current| CFEqual(Some(as_cf_type(first)), Some(as_cf_type(current))),
    )?;
    let window_fingerprint = CFHash(Some(as_cf_type(&window))) as u64;
    verify_frontmost_identity(pid, process)?;
    Ok(NativeObservation {
        process,
        window_fingerprint,
        process_name,
        bundle_identifier,
        title,
    })
}

fn frontmost_process() -> Result<(i32, ProcessIdentity, String, Option<String>), ResolveFailure> {
    let application = frontmost_application()?;
    let pid = application.processIdentifier();
    if pid <= 0 {
        return Err(ResolveFailure::TargetExited);
    }
    let process = process_identity(pid)?;
    let process_name = process_name(pid)?;
    let bundle_identifier = application
        .bundleIdentifier()
        .map(|bundle| copy_cf_string(AsRef::<CFString>::as_ref(&*bundle)))
        .transpose()?;
    Ok((pid, process, process_name, bundle_identifier))
}

fn focused_window(pid: i32) -> Result<CFRetained<AXUIElement>, ResolveFailure> {
    let application = create_application(pid)?;
    let window = copy_timed_ax_attribute::<AXUIElement>(&application, FOCUSED_WINDOW_ATTRIBUTE)?;
    let mut window_pid = 0;
    require_ax(unsafe { window.pid(NonNull::from(&mut window_pid)) })?;
    if window_pid != pid {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(window)
}

fn window_title(window: &AXUIElement) -> Result<String, ResolveFailure> {
    let title = copy_timed_ax_attribute::<CFString>(window, WINDOW_TITLE_ATTRIBUTE)?;
    copy_cf_string(&title)
}

fn copy_timed_ax_attribute<T: ConcreteType>(
    element: &AXUIElement,
    attribute: &'static str,
) -> Result<CFRetained<T>, ResolveFailure> {
    timed_ax_query(
        attribute,
        |timeout| require_ax(unsafe { element.set_messaging_timeout(timeout) }),
        |attribute| {
            let attribute = create_cf_string(attribute.as_bytes())?;
            copy_ax_attribute(element, &attribute)
        },
    )
}

pub(super) fn timed_ax_query<T>(
    attribute: &'static str,
    set_timeout: impl FnOnce(f32) -> Result<(), ResolveFailure>,
    copy_attribute: impl FnOnce(&'static str) -> Result<T, ResolveFailure>,
) -> Result<T, ResolveFailure> {
    set_timeout(AX_MESSAGING_TIMEOUT_SECONDS)?;
    copy_attribute(attribute)
}

fn copy_ax_attribute<T: ConcreteType>(
    element: &AXUIElement,
    attribute: &CFString,
) -> Result<CFRetained<T>, ResolveFailure> {
    let mut value: *const CFType = std::ptr::null();
    require_ax(unsafe { element.copy_attribute_value(attribute, NonNull::from(&mut value)) })?;
    let value = NonNull::new(value.cast_mut()).ok_or(ResolveFailure::InvalidData)?;
    // SAFETY: A successful Copy call returns one owned CF reference. The null
    // case was rejected above before constructing the owner.
    let value = unsafe { CFRetained::<CFType>::from_raw(value) };
    value
        .downcast::<T>()
        .map_err(|_| ResolveFailure::InvalidData)
}

fn create_cf_string(bytes: &[u8]) -> Result<CFRetained<CFString>, ResolveFailure> {
    // SAFETY: `bytes` remains valid for the call and Core Foundation copies it.
    unsafe {
        CFString::with_bytes(
            None,
            bytes.as_ptr(),
            bytes.len() as isize,
            CFStringBuiltInEncodings::EncodingUTF8.0,
            false,
        )
    }
    .ok_or(ResolveFailure::InvalidData)
}

fn copy_cf_string(value: &CFString) -> Result<String, ResolveFailure> {
    let length = value.length();
    if length < 0 || length as usize > MAX_CONTEXT_UTF16_UNITS {
        return Err(ResolveFailure::Oversized);
    }
    let mut bytes = [0_u8; MAX_CONTEXT_UTF8_BYTES];
    let mut used = 0_isize;
    // SAFETY: The fixed output buffer and `used` pointer are valid for the
    // call. A lossy conversion byte is not supplied.
    let converted = unsafe {
        value.bytes(
            CFRange::new(0, length),
            CFStringBuiltInEncodings::EncodingUTF8.0,
            0,
            false,
            bytes.as_mut_ptr(),
            bytes.len() as isize,
            &mut used,
        )
    };
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

fn frontmost_application() -> Result<Retained<NSRunningApplication>, ResolveFailure> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or(ResolveFailure::Unavailable)
}

fn verify_frontmost_identity(pid: i32, process: ProcessIdentity) -> Result<(), ResolveFailure> {
    if process_identity(pid)? != process || frontmost_application()?.processIdentifier() != pid {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(())
}

pub(super) fn process_identity(pid: i32) -> Result<ProcessIdentity, ResolveFailure> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let expected = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: `info` has the exact kernel structure size and is initialized
    // only when `proc_pidinfo` reports that complete size.
    let actual = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            expected,
        )
    };
    if actual != expected {
        return Err(ResolveFailure::TargetExited);
    }
    // SAFETY: The complete-size check above proves initialization.
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid as u32 {
        return Err(ResolveFailure::TargetExited);
    }
    Ok(ProcessIdentity {
        pid,
        started_seconds: info.pbi_start_tvsec,
        started_microseconds: info.pbi_start_tvusec,
    })
}

pub(super) fn process_name(pid: i32) -> Result<String, ResolveFailure> {
    let mut bytes = [0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: The fixed buffer is writable for the supplied length.
    let length = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if length <= 0 {
        return Err(ResolveFailure::TargetExited);
    }
    if length as usize >= bytes.len() {
        return Err(ResolveFailure::Oversized);
    }
    // SAFETY: A successful `proc_pidpath` call writes a NUL-terminated path.
    let path = unsafe { CStr::from_ptr(bytes.as_ptr().cast()) }.to_bytes();
    executable_name_from_path(path)
}

fn require_ax(error: AXError) -> Result<(), ResolveFailure> {
    match error {
        AXError::Success => Ok(()),
        AXError::CannotComplete => Err(ResolveFailure::Timeout),
        _ => Err(ResolveFailure::Accessibility),
    }
}

fn as_cf_type<T: AsRef<CFType> + ?Sized>(value: &T) -> &CFType {
    value.as_ref()
}

fn retained_application(
    value: Option<NonNull<AXUIElement>>,
) -> Result<CFRetained<AXUIElement>, ResolveFailure> {
    let value = value.ok_or(ResolveFailure::TargetExited)?;
    // SAFETY: The raw Create function returns one owned AX reference.
    Ok(unsafe { CFRetained::from_raw(value) })
}

fn create_application(pid: i32) -> Result<CFRetained<AXUIElement>, ResolveFailure> {
    // objc2 0.3.2 models this Create function as non-null and panics if the
    // framework returns NULL. The existing fail-open contract requires NULL
    // to become TargetExited, so this is the one retained typed raw leaf.
    retained_application(unsafe { ax_ui_element_create_application(pid) })
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C-unwind" {
    #[link_name = "AXUIElementCreateApplication"]
    fn ax_ui_element_create_application(pid: libc::pid_t) -> Option<NonNull<AXUIElement>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_preflight_uses_no_prompt_options_dictionary() {
        assert!(accessibility_preflight_options().is_none());
    }

    #[test]
    fn nullable_cf_create_is_rejected_before_owned_drop() {
        assert!(matches!(
            retained_application(None),
            Err(ResolveFailure::TargetExited)
        ));
    }

    #[test]
    fn ax_error_classes_preserve_failure_mapping() {
        assert_eq!(require_ax(AXError::Success), Ok(()));
        assert_eq!(
            require_ax(AXError::CannotComplete),
            Err(ResolveFailure::Timeout)
        );
        assert_eq!(
            require_ax(AXError::IllegalArgument),
            Err(ResolveFailure::Accessibility)
        );
    }
}
