//! Bounded macOS overlay delivery scheduled onto Tauri's main thread.

#[cfg(target_os = "macos")]
mod native;

#[cfg(any(target_os = "macos", test))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

use super::OverlayConfig;
#[cfg(target_os = "macos")]
use crate::config::RuntimeConfig;
use crate::domain::Point;

const QUEUE_CAPACITY: usize = 64;

#[derive(Debug)]
#[cfg_attr(all(test, not(target_os = "macos")), allow(dead_code))]
pub(super) enum Command {
    Start(OverlayConfig),
    Point(Point),
    Label(Option<String>),
    End,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    Accepted,
    Full,
    Fault,
}

struct QueueGate {
    state: QueueState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum QueueState {
    #[default]
    Unused,
    Idle,
    Active,
    Shutdown,
}

impl Default for QueueGate {
    fn default() -> Self {
        Self {
            state: QueueState::Unused,
        }
    }
}

#[derive(Clone, Copy)]
enum GateTransition {
    Start,
    End,
    Shutdown,
    None,
}

impl From<&Command> for GateTransition {
    fn from(command: &Command) -> Self {
        match command {
            Command::Start(_) => Self::Start,
            Command::End => Self::End,
            Command::Shutdown => Self::Shutdown,
            Command::Point(_) | Command::Label(_) => Self::None,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
struct WakeGate(AtomicBool);

#[cfg(any(target_os = "macos", test))]
impl WakeGate {
    fn new() -> Self {
        Self(AtomicBool::new(false))
    }
    fn request(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
    fn cancel(&self) {
        self.0.store(false, Ordering::Release);
    }
    #[cfg(test)]
    fn pending(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl QueueGate {
    fn admits(&self, command: &Command, queued: usize) -> Delivery {
        let active = self.state == QueueState::Active;
        let shutdown = self.state == QueueState::Shutdown;
        let reserved = 1 + usize::from(active);
        match command {
            Command::Start(_) if active || shutdown => Delivery::Fault,
            Command::Start(_) if queued.saturating_add(3) > QUEUE_CAPACITY => Delivery::Full,
            Command::Point(_) | Command::Label(_) if !active || shutdown => Delivery::Fault,
            Command::Point(_) | Command::Label(_)
                if queued.saturating_add(reserved) >= QUEUE_CAPACITY =>
            {
                Delivery::Full
            }
            Command::End if !active || shutdown => Delivery::Fault,
            Command::Shutdown if shutdown => Delivery::Fault,
            _ => Delivery::Accepted,
        }
    }

    fn accepted(&mut self, transition: GateTransition) {
        match transition {
            GateTransition::Start => self.state = QueueState::Active,
            GateTransition::End => self.state = QueueState::Idle,
            GateTransition::Shutdown => self.state = QueueState::Shutdown,
            GateTransition::None => {}
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) struct MacosOverlayClient {
    app: tauri::AppHandle,
    sender: Sender<Command>,
    shared: Arc<Shared>,
    shutdown_ack: Receiver<()>,
    gate: QueueGate,
}

#[cfg(target_os = "macos")]
pub(super) struct Shared {
    receiver: Receiver<Command>,
    wake: WakeGate,
    failed: AtomicBool,
    shutdown_ack: Sender<()>,
}

#[cfg(target_os = "macos")]
impl MacosOverlayClient {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        let (sender, receiver) = bounded(QUEUE_CAPACITY);
        let (shutdown_tx, shutdown_ack) = bounded(1);
        Self {
            app,
            sender,
            shared: Arc::new(Shared {
                receiver,
                wake: WakeGate::new(),
                failed: AtomicBool::new(false),
                shutdown_ack: shutdown_tx,
            }),
            shutdown_ack,
            gate: QueueGate::default(),
        }
    }

    pub(crate) fn start(&mut self, runtime: &RuntimeConfig) -> Delivery {
        self.send(Command::Start(OverlayConfig::from_runtime(runtime)))
    }
    pub(crate) fn point(&mut self, point: Point) -> Delivery {
        self.send(Command::Point(point))
    }
    pub(crate) fn label(&mut self, label: Option<String>) -> Delivery {
        self.send(Command::Label(label))
    }
    pub(crate) fn end(&mut self) -> Delivery {
        self.send(Command::End)
    }
    pub(crate) fn has_failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    pub(crate) fn shutdown(&mut self, timeout: Duration) -> bool {
        if self.gate.state == QueueState::Unused {
            self.gate.accepted(GateTransition::Shutdown);
            return true;
        }
        if self.has_failed() || self.send(Command::Shutdown) != Delivery::Accepted {
            return false;
        }
        self.shutdown_ack.recv_timeout(timeout).is_ok()
    }

    fn send(&mut self, command: Command) -> Delivery {
        if self.has_failed() {
            return Delivery::Fault;
        }
        let admission = self.gate.admits(&command, self.sender.len());
        if admission != Delivery::Accepted {
            return admission;
        }
        let transition = GateTransition::from(&command);
        match self.sender.try_send(command) {
            Ok(()) => self.gate.accepted(transition),
            Err(TrySendError::Full(_)) => return Delivery::Full,
            Err(TrySendError::Disconnected(_)) => return Delivery::Fault,
        }
        self.schedule()
    }

    fn schedule(&self) -> Delivery {
        if !self.shared.wake.request() {
            return Delivery::Accepted;
        }
        let shared = Arc::clone(&self.shared);
        if self
            .app
            .run_on_main_thread(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    native::drain(Arc::clone(&shared));
                }));
                if result.is_err() {
                    shared.failed.store(true, Ordering::Release);
                    shared.wake.cancel();
                }
            })
            .is_err()
        {
            self.shared.wake.cancel();
            self.shared.failed.store(true, Ordering::Release);
            return Delivery::Fault;
        }
        Delivery::Accepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OverlayConfig {
        OverlayConfig {
            color: (0, 1, 2),
            pen_width: 2,
            label_font_family: String::new(),
            label_font_size: 12,
            label_font_weight: 400,
            label_padding: 4,
        }
    }

    #[test]
    fn macos_overlay_queue_reserves_terminal_capacity_under_lossy_overload() {
        let mut gate = QueueGate::default();
        let start = Command::Start(config());
        assert_eq!(gate.admits(&start, QUEUE_CAPACITY - 3), Delivery::Accepted);
        gate.accepted(GateTransition::from(&start));
        assert_eq!(
            gate.admits(&Command::Point(Point::new(1, 2)), QUEUE_CAPACITY - 2),
            Delivery::Full
        );
        assert_eq!(
            gate.admits(&Command::End, QUEUE_CAPACITY - 2),
            Delivery::Accepted
        );
    }

    #[test]
    fn overlay_startup_is_lazy_and_requires_no_main_thread_ready_ack() {
        let mut gate = QueueGate::default();
        let wake = WakeGate::new();
        assert_eq!(gate.state, QueueState::Unused);
        assert!(!wake.pending());
        assert_eq!(gate.admits(&Command::End, 0), Delivery::Fault);
        let start = Command::Start(config());
        gate.accepted(GateTransition::from(&start));
        assert_eq!(gate.admits(&start, 1), Delivery::Fault);
        gate.accepted(GateTransition::End);
        gate.accepted(GateTransition::Shutdown);
        assert_eq!(gate.admits(&Command::Start(config()), 0), Delivery::Fault);
    }

    #[test]
    fn main_thread_wakeup_is_coalesced_without_stranding_work() {
        let (sender, receiver) = crossbeam_channel::bounded(2);
        let wake = WakeGate::new();
        assert!(wake.request());
        sender.try_send(Command::End).unwrap();
        assert!(!wake.request());
        wake.cancel();
        assert!(!receiver.is_empty());
        assert!(wake.request());
        receiver.try_recv().unwrap();
        wake.cancel();
        sender.try_send(Command::Shutdown).unwrap();
        assert!(wake.request());
        assert!(!wake.request());
        receiver.try_recv().unwrap();
        wake.cancel();
        assert!(!wake.pending());
    }
}
