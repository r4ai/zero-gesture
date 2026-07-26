//! Portable gesture recognition and session decisions.
//!
//! Platform code supplies normalized pointer input and an optional matched
//! application ID. The owned [`GestureMachine`] returns a closed [`Decision`]
//! that platform code applies after the transition completes.

mod recognition;
mod session;

pub(crate) use session::{
    AppBindingSet, Decision, Disposition, GestureConfig, GestureInput, GestureMachine,
    GestureTransition, HoldBinding, MouseEvent, Point, ReleaseBinding, RenderEffect, ReplayRequest,
    TriggerButton,
};
