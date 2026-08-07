use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventSource, CGEventTapLocation, CGKeyCode, CGPreflightPostEventAccess,
};

type CreateKeyboardEvent =
    extern "C-unwind" fn(Option<&CGEventSource>, CGKeyCode, bool) -> Option<CFRetained<CGEvent>>;
type SetIntegerValue = extern "C-unwind" fn(Option<&CGEvent>, CGEventField, i64);
type PostEvent = extern "C-unwind" fn(CGEventTapLocation, Option<&CGEvent>);

#[derive(Clone, Copy)]
struct CgFunctions {
    create_keyboard_event: CreateKeyboardEvent,
    set_integer_value: SetIntegerValue,
    post_event: PostEvent,
}

extern "C-unwind" fn create_keyboard_event(
    source: Option<&CGEventSource>,
    virtual_key: CGKeyCode,
    key_down: bool,
) -> Option<CFRetained<CGEvent>> {
    CGEvent::new_keyboard_event(source, virtual_key, key_down)
}

extern "C-unwind" fn set_integer_value(event: Option<&CGEvent>, field: CGEventField, value: i64) {
    CGEvent::set_integer_value_field(event, field, value);
}

extern "C-unwind" fn post_event(tap: CGEventTapLocation, event: Option<&CGEvent>) {
    CGEvent::post(tap, event);
}

const SYSTEM_CG_FUNCTIONS: CgFunctions = CgFunctions {
    create_keyboard_event,
    set_integer_value,
    post_event,
};

pub(super) fn post_access_allowed() -> bool {
    CGPreflightPostEventAccess()
}

pub(super) fn create_tag_and_post_repeat(key_codes: &[u16], marker: i64) -> bool {
    create_tag_and_post_repeat_with(key_codes, marker, SYSTEM_CG_FUNCTIONS)
}

fn create_tag_and_post_repeat_with(key_codes: &[u16], marker: i64, functions: CgFunctions) -> bool {
    let Some(events) = create_tagged_events(key_codes, marker, functions) else {
        return false;
    };
    for event in &events {
        (functions.post_event)(CGEventTapLocation::SessionEventTap, Some(event));
    }
    true
}

fn create_tagged_events(
    key_codes: &[u16],
    marker: i64,
    functions: CgFunctions,
) -> Option<Vec<CFRetained<CGEvent>>> {
    let mut events = Vec::with_capacity(key_codes.len() * 2);
    for (&key_code, key_down) in key_codes
        .iter()
        .map(|key| (key, true))
        .chain(key_codes.iter().rev().map(|key| (key, false)))
    {
        let event = (functions.create_keyboard_event)(None, key_code, key_down)?;
        (functions.set_integer_value)(Some(&event), CGEventField::EventSourceUserData, marker);
        events.push(event);
    }
    Some(events)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use objc2_core_foundation::CFType;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RecordedCall {
        Create(u16, bool, usize),
        Tag(usize, CGEventField, i64),
        Post(CGEventTapLocation, usize),
    }

    thread_local! {
        static CALLS: RefCell<Vec<RecordedCall>> = const { RefCell::new(Vec::new()) };
        static CREATED_EVENTS: RefCell<Vec<CFRetained<CGEvent>>> = const { RefCell::new(Vec::new()) };
        static CREATE_COUNT: Cell<usize> = const { Cell::new(0) };
        static NULL_AT: Cell<usize> = const { Cell::new(usize::MAX) };
    }

    fn event_id(event: &CGEvent) -> usize {
        std::ptr::from_ref(event).addr()
    }

    extern "C-unwind" fn record_create(
        _: Option<&CGEventSource>,
        key: CGKeyCode,
        down: bool,
    ) -> Option<CFRetained<CGEvent>> {
        let call = CREATE_COUNT.get();
        CREATE_COUNT.set(call + 1);
        if call == NULL_AT.get() {
            return None;
        }
        let event = CGEvent::new_keyboard_event(None, key, down)?;
        let id = event_id(&event);
        CREATED_EVENTS.with_borrow_mut(|events| events.push(event.clone()));
        CALLS.with_borrow_mut(|calls| calls.push(RecordedCall::Create(key, down, id)));
        Some(event)
    }

    extern "C-unwind" fn record_tag(event: Option<&CGEvent>, field: CGEventField, marker: i64) {
        let id = event_id(event.expect("tag requires an event"));
        CALLS.with_borrow_mut(|calls| calls.push(RecordedCall::Tag(id, field, marker)));
    }

    extern "C-unwind" fn record_post(tap: CGEventTapLocation, event: Option<&CGEvent>) {
        let id = event_id(event.expect("post requires an event"));
        CALLS.with_borrow_mut(|calls| calls.push(RecordedCall::Post(tap, id)));
    }

    const RECORDING_FUNCTIONS: CgFunctions = CgFunctions {
        create_keyboard_event: record_create,
        set_integer_value: record_tag,
        post_event: record_post,
    };

    fn reset_calls(null_at: usize) {
        CREATE_COUNT.set(0);
        NULL_AT.set(null_at);
        CALLS.with_borrow_mut(Vec::clear);
        CREATED_EVENTS.with_borrow_mut(Vec::clear);
    }

    #[test]
    fn generated_keyboard_events_all_carry_process_marker() {
        reset_calls(usize::MAX);

        assert!(create_tag_and_post_repeat_with(
            &[0x37, 0x00],
            0x1234,
            RECORDING_FUNCTIONS,
        ));

        let calls = CALLS.with_borrow(Clone::clone);
        let posts = calls
            .iter()
            .enumerate()
            .filter_map(|(index, call)| match call {
                RecordedCall::Post(CGEventTapLocation::SessionEventTap, event_id) => {
                    Some((index, *event_id))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(posts.len(), 4);

        for (post_index, event_id) in posts {
            let tags = calls
                .iter()
                .enumerate()
                .filter_map(|(index, call)| match call {
                    RecordedCall::Tag(
                        tagged_event_id,
                        CGEventField::EventSourceUserData,
                        0x1234,
                    ) if *tagged_event_id == event_id => Some(index),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(tags.len(), 1, "event {event_id} must be tagged once");
            assert!(
                tags[0] < post_index,
                "event {event_id} must be tagged before posting"
            );
        }
    }

    #[test]
    fn keyboard_action_posts_key_downs_then_reverse_key_ups() {
        reset_calls(usize::MAX);

        assert!(create_tag_and_post_repeat_with(
            &[0x37, 0x00],
            7,
            RECORDING_FUNCTIONS,
        ));

        let created = CALLS.with_borrow(|calls| {
            calls
                .iter()
                .filter_map(|call| match call {
                    RecordedCall::Create(key, down, _) => Some((*key, *down)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(
            created,
            [(0x37, true), (0x00, true), (0x00, false), (0x37, false)]
        );
    }

    #[test]
    fn nullable_event_creation_releases_only_owned_events_and_posts_nothing() {
        reset_calls(1);

        assert!(!create_tag_and_post_repeat_with(
            &[0x37, 0x00],
            7,
            RECORDING_FUNCTIONS,
        ));

        let calls = CALLS.with_borrow(Clone::clone);
        assert!(!calls
            .iter()
            .any(|call| matches!(call, RecordedCall::Post(_, _))));
        CREATED_EVENTS.with_borrow(|events| {
            assert_eq!(events.len(), 1);
            let event_as_cf: &CFType = events[0].as_ref();
            assert_eq!(
                event_as_cf.retain_count(),
                1,
                "the generated CFRetained owner must release the failed batch"
            );
        });
    }
}
