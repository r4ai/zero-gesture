use super::platform;
use super::protocol::{
    self, EngineStatus, Envelope, ErrorCode, ProtocolError, Request, Response, WindowCaptureInfo,
    WindowCaptureResult, AUTH_SECRET_BYTES, CAPABILITIES, MAX_REQUESTS_PER_CONNECTION,
};
use crate::capture::{CaptureError, CapturePoll, WindowCapture};
use crate::config::{self, ConfigOwner, ConfigOwnerError, ConfigOwnerStatus, PreparedToken};
use crate::domain::Point;
use log::{debug, warn};
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

pub(super) const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(super) const CONFIG_SCHEMA_VERSION: u16 = 2;
pub(super) const IO_TIMEOUT: Duration = Duration::from_millis(750);
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const RETRY_INTERVAL: Duration = Duration::from_millis(40);
pub(super) const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(2);
pub(super) const TERMINAL_RESPONSE_GRACE: Duration = Duration::from_millis(100);
pub(crate) type CaptureResolver = fn(Point) -> Result<crate::window_info::ForegroundWindowInfo, ()>;

#[derive(Debug)]
pub enum ControlError {
    Unavailable,
    EndpointBusy,
    Timeout,
    SpawnFailed(io::Error),
    Security(String),
    Protocol(ProtocolError),
    Rejected(ErrorCode),
    Io(io::Error),
    ProjectionFailed(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("Engine is unavailable"),
            Self::EndpointBusy => formatter.write_str("Engine endpoint is busy"),
            Self::Timeout => formatter.write_str("Engine connection timed out"),
            Self::SpawnFailed(error) => write!(formatter, "failed to start Engine: {error}"),
            Self::Security(error) => write!(formatter, "IPC security setup failed: {error}"),
            Self::Protocol(error) => write!(formatter, "IPC protocol failed: {error}"),
            Self::Rejected(code) => write!(formatter, "Engine rejected the request: {code:?}"),
            Self::Io(error) => write!(formatter, "IPC I/O failed: {error}"),
            Self::ProjectionFailed(error) => {
                write!(formatter, "Engine live projection failed: {error}")
            }
        }
    }
}

impl std::error::Error for ControlError {}

impl From<ProtocolError> for ControlError {
    fn from(error: ProtocolError) -> Self {
        match &error {
            ProtocolError::Io(io_error) if io_error.kind() == io::ErrorKind::TimedOut => {
                Self::Timeout
            }
            _ => Self::Protocol(error),
        }
    }
}

impl From<io::Error> for ControlError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl ControlError {
    pub(crate) fn projection(error: impl fmt::Display) -> Self {
        Self::ProjectionFailed(error.to_string())
    }
}

#[derive(Clone)]
pub struct EngineControl {
    pub(super) endpoint: platform::Endpoint,
    capture_session: Arc<Mutex<Option<Session>>>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct ConfigObservation {
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) config: Option<config::ConfigDocument>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub(crate) struct ConfigApplyResult {
    pub(crate) current: ConfigObservation,
    pub(crate) durability_warning: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigApplyPhase {
    Prepare,
    Commit,
    Query,
}

#[derive(Debug)]
pub(crate) struct ConfigApplyError {
    pub(crate) phase: ConfigApplyPhase,
    pub(crate) source: ControlError,
}

impl ConfigApplyError {
    fn new(phase: ConfigApplyPhase, source: ControlError) -> Self {
        Self { phase, source }
    }
}

impl fmt::Display for ConfigApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for ConfigApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct WindowCaptureStarted {
    pub(crate) capture_id: u64,
    pub(crate) epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub(crate) enum WindowCaptureObservation {
    Pending,
    Captured {
        info: crate::window_info::ForegroundWindowInfo,
    },
}

impl EngineControl {
    pub fn connect_or_start(executable: &Path, config_dir: &Path) -> Result<Self, ControlError> {
        #[cfg(debug_assertions)]
        let suffix = std::env::var_os("ZG_P03_TEST_NAMESPACE")
            .map(|value| {
                value.to_str().map(str::to_owned).ok_or_else(|| {
                    ControlError::Security(
                        "internal endpoint namespace must be valid Unicode".to_string(),
                    )
                })
            })
            .transpose()?
            .unwrap_or_default();
        #[cfg(not(debug_assertions))]
        let suffix = String::new();
        let control = Self {
            endpoint: platform::Endpoint::current_user(config_dir, &suffix)?,
            capture_session: Arc::new(Mutex::new(None)),
        };
        control.connect_or_start_with(|| {
            Command::new(executable)
                .arg("--engine")
                .spawn()
                .map(|_| ())
                .map_err(ControlError::SpawnFailed)
        })?;
        Ok(control)
    }

    #[cfg(test)]
    pub(super) fn ping(&self) -> Result<(), ControlError> {
        self.ping_before(Instant::now() + IO_TIMEOUT)
    }

    fn ping_before(&self, deadline: Instant) -> Result<(), ControlError> {
        let mut session = Session::connect_before(&self.endpoint, deadline)?;
        match session.exchange_before(Request::Ping, deadline)? {
            Response::Pong => Ok(()),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub fn status(&self) -> Result<EngineStatus, ControlError> {
        let mut session = Session::connect(&self.endpoint)?;
        match session.exchange(Request::GetStatus)? {
            Response::Status(status) => Ok(status),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub(crate) fn current_config(&self) -> Result<ConfigObservation, ControlError> {
        let mut session = Session::connect(&self.endpoint)?;
        session.current_config()
    }

    pub(crate) fn apply_config(
        &self,
        document: config::ConfigDocument,
        expected_revision: u64,
    ) -> Result<ConfigApplyResult, ConfigApplyError> {
        let bytes = serde_json::to_vec(&document).map_err(|_| {
            ConfigApplyError::new(
                ConfigApplyPhase::Prepare,
                ControlError::Rejected(ErrorCode::ConfigValidationFailed),
            )
        })?;
        self.apply_config_bytes(bytes, expected_revision)
    }

    pub(crate) fn apply_config_bytes(
        &self,
        bytes: Vec<u8>,
        expected_revision: u64,
    ) -> Result<ConfigApplyResult, ConfigApplyError> {
        let mut session = Session::connect(&self.endpoint)
            .map_err(|error| ConfigApplyError::new(ConfigApplyPhase::Prepare, error))?;
        let prepared = match session
            .exchange(Request::PrepareConfig {
                expected_revision,
                config_bytes: bytes,
            })
            .map_err(|error| ConfigApplyError::new(ConfigApplyPhase::Prepare, error))?
        {
            Response::Prepared {
                token,
                base_revision,
                base_generation,
            } => (token, base_revision, base_generation),
            Response::Error(code) => {
                return Err(ConfigApplyError::new(
                    ConfigApplyPhase::Prepare,
                    ControlError::Rejected(code),
                ))
            }
            _ => {
                return Err(ConfigApplyError::new(
                    ConfigApplyPhase::Prepare,
                    ControlError::Protocol(ProtocolError::InvalidMessage),
                ))
            }
        };
        let durability_warning = match session
            .exchange(Request::CommitConfig {
                token: prepared.0,
                base_revision: prepared.1,
                base_generation: prepared.2,
            })
            .map_err(|error| ConfigApplyError::new(ConfigApplyPhase::Commit, error))?
        {
            Response::Applied {
                durability_warning, ..
            } => durability_warning,
            Response::Error(code) => {
                return Err(ConfigApplyError::new(
                    ConfigApplyPhase::Commit,
                    ControlError::Rejected(code),
                ))
            }
            _ => {
                return Err(ConfigApplyError::new(
                    ConfigApplyPhase::Commit,
                    ControlError::Protocol(ProtocolError::InvalidMessage),
                ))
            }
        };
        Ok(ConfigApplyResult {
            current: session
                .current_config()
                .map_err(|error| ConfigApplyError::new(ConfigApplyPhase::Query, error))?,
            durability_warning,
        })
    }

    pub(crate) fn set_enabled(
        &self,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<ConfigApplyResult, ControlError> {
        let mut session = Session::connect(&self.endpoint)?;
        let durability_warning = match session.exchange(Request::SetEnabled {
            expected_revision,
            enabled,
        })? {
            Response::Applied {
                durability_warning, ..
            } => durability_warning,
            Response::Error(code) => return Err(ControlError::Rejected(code)),
            _ => return Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        };
        Ok(ConfigApplyResult {
            current: session.current_config()?,
            durability_warning,
        })
    }

    pub fn shutdown(&self) -> Result<bool, ControlError> {
        self.drop_capture_session()?;
        let mut session = Session::connect(&self.endpoint)?;
        match session.exchange(Request::Shutdown)? {
            Response::Shutdown { already_requested } => Ok(already_requested),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub(crate) fn begin_window_capture(
        &self,
        capture_id: u64,
    ) -> Result<WindowCaptureStarted, ControlError> {
        let mut sessions = self
            .capture_session
            .lock()
            .map_err(|_| ControlError::Protocol(ProtocolError::InvalidMessage))?;
        if sessions.is_none() {
            *sessions = Some(Session::connect(&self.endpoint)?);
        }
        let session = sessions
            .as_mut()
            .expect("capture session was inserted above");
        match session.exchange(Request::BeginWindowCapture { capture_id })? {
            Response::WindowCaptureStarted { capture_id, epoch } => {
                Ok(WindowCaptureStarted { capture_id, epoch })
            }
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub(crate) fn poll_window_capture(
        &self,
        capture_id: u64,
        epoch: u64,
    ) -> Result<WindowCaptureObservation, ControlError> {
        let mut sessions = self
            .capture_session
            .lock()
            .map_err(|_| ControlError::Protocol(ProtocolError::InvalidMessage))?;
        let session = sessions
            .as_mut()
            .ok_or(ControlError::Rejected(ErrorCode::CaptureStale))?;
        match session.exchange(Request::PollWindowCapture { capture_id, epoch })? {
            Response::WindowCapture {
                capture_id: actual_id,
                epoch: actual_epoch,
                result,
            } if actual_id == capture_id && actual_epoch == epoch => Ok(match result {
                WindowCaptureResult::Pending => WindowCaptureObservation::Pending,
                WindowCaptureResult::Captured(info) => WindowCaptureObservation::Captured {
                    info: crate::window_info::ForegroundWindowInfo {
                        process_name: info.process_name,
                        window_class: info.window_class,
                        title: info.title,
                    },
                },
            }),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub(crate) fn cancel_window_capture(
        &self,
        capture_id: u64,
        epoch: u64,
    ) -> Result<(), ControlError> {
        let mut sessions = self
            .capture_session
            .lock()
            .map_err(|_| ControlError::Protocol(ProtocolError::InvalidMessage))?;
        let mut session = sessions
            .take()
            .ok_or(ControlError::Rejected(ErrorCode::CaptureStale))?;
        match session.exchange(Request::CancelWindowCapture { capture_id, epoch })? {
            Response::WindowCaptureCancelled {
                capture_id: actual_id,
                epoch: actual_epoch,
            } if actual_id == capture_id && actual_epoch == epoch => Ok(()),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    fn drop_capture_session(&self) -> Result<(), ControlError> {
        self.capture_session
            .lock()
            .map_err(|_| ControlError::Protocol(ProtocolError::InvalidMessage))?
            .take();
        Ok(())
    }

    pub(super) fn connect_or_start_with(
        &self,
        spawn: impl FnOnce() -> Result<(), ControlError>,
    ) -> Result<(), ControlError> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        match self.ping_before(deadline) {
            Ok(()) => return Ok(()),
            Err(ControlError::Unavailable) => {}
            Err(error) => return Err(error),
        }

        let _launch = self.endpoint.acquire_launch_lock(deadline)?;
        match self.ping_before(deadline) {
            Ok(()) => return Ok(()),
            Err(ControlError::Unavailable) => {}
            Err(error) => return Err(error),
        }
        spawn()?;
        loop {
            match self.ping_before(deadline) {
                Ok(()) => return Ok(()),
                Err(ControlError::Unavailable) if Instant::now() < deadline => {
                    thread::sleep(
                        RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(ControlError::Unavailable) => return Err(ControlError::Timeout),
                Err(error) => return Err(error),
            }
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(config_dir: &Path, suffix: &str) -> Result<Self, ControlError> {
        Ok(Self {
            endpoint: platform::Endpoint::current_user(config_dir, suffix)?,
            capture_session: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn for_prepared_server(server: &EngineServer) -> Self {
        Self {
            endpoint: server.endpoint.clone(),
            capture_session: Arc::new(Mutex::new(None)),
        }
    }
}

pub struct EngineServer {
    pub(super) endpoint: platform::Endpoint,
    secret: [u8; AUTH_SECRET_BYTES],
    pub(super) transport: platform::ServerTransport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerExit {
    Shutdown,
    Stopped,
}

impl EngineServer {
    pub fn new(config_dir: &Path) -> Result<Option<Self>, ControlError> {
        Self::with_suffix(config_dir, "")
    }

    #[cfg(debug_assertions)]
    pub(crate) fn for_debug_namespace(
        config_dir: &Path,
        suffix: &str,
    ) -> Result<Option<Self>, ControlError> {
        Self::with_suffix(config_dir, suffix)
    }

    #[cfg(test)]
    pub(super) fn for_test(config_dir: &Path, suffix: &str) -> Result<Option<Self>, ControlError> {
        Self::with_suffix(config_dir, suffix)
    }

    fn with_suffix(config_dir: &Path, suffix: &str) -> Result<Option<Self>, ControlError> {
        let endpoint = platform::Endpoint::current_user(config_dir, suffix)?;
        let Some((secret, transport)) = endpoint.prepare_server()? else {
            return Ok(None);
        };
        Ok(Some(Self {
            endpoint,
            secret,
            transport,
        }))
    }

    pub fn run<F>(
        self,
        stop: Arc<AtomicBool>,
        mut config_owner: ConfigOwner,
        capture: Arc<WindowCapture>,
        resolve_capture: CaptureResolver,
        mut on_applied: F,
    ) -> Result<ServerExit, ControlError>
    where
        F: FnMut(&config::ActiveConfig, u64) -> Result<(), ControlError>,
    {
        let Self {
            endpoint: _,
            secret,
            mut transport,
        } = self;
        let started_at = Instant::now();
        let mut next_session = 1_u64;

        while !stop.load(Ordering::Acquire) {
            let Some(mut connection) = transport.accept(&stop)? else {
                return Ok(ServerExit::Stopped);
            };
            let session = next_session;
            next_session = next_session.checked_add(1).unwrap_or(1);
            let mut context = ConnectionContext {
                started_at,
                config_owner: &mut config_owner,
                capture: &capture,
                resolve_capture,
                session,
                on_applied: &mut on_applied,
            };
            if serve_connection(&mut connection, &secret, &mut context)? {
                return Ok(ServerExit::Shutdown);
            }
        }
        Ok(ServerExit::Stopped)
    }
}

struct ConnectionContext<'a> {
    started_at: Instant,
    config_owner: &'a mut ConfigOwner,
    capture: &'a WindowCapture,
    resolve_capture: CaptureResolver,
    session: u64,
    on_applied: &'a mut dyn FnMut(&config::ActiveConfig, u64) -> Result<(), ControlError>,
}

fn serve_connection(
    connection: &mut platform::AcceptedConnection<'_>,
    secret: &[u8; AUTH_SECRET_BYTES],
    context: &mut ConnectionContext<'_>,
) -> Result<bool, ControlError> {
    let result = serve_connection_inner(connection, secret, context);
    context.config_owner.disconnect(context.session);
    context.capture.disconnect(context.session);
    match result {
        Err(error @ ControlError::ProjectionFailed(_)) => Err(error),
        Err(_) => Ok(false),
        Ok(shutdown_requested) => Ok(shutdown_requested),
    }
}

fn serve_connection_inner(
    connection: &mut platform::AcceptedConnection<'_>,
    secret: &[u8; AUTH_SECRET_BYTES],
    context: &mut ConnectionContext<'_>,
) -> Result<bool, ControlError> {
    let mut authenticated = false;
    let mut version_matches = false;
    let mut shutdown_requested = false;
    let mut request_ids = [0_u64; MAX_REQUESTS_PER_CONNECTION];
    let mut request_count = 0;

    loop {
        connection.set_deadline(Instant::now() + IO_TIMEOUT)?;
        let body = match protocol::read_frame(&mut *connection) {
            Ok(body) => body,
            Err(ProtocolError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::UnexpectedEof
                        | io::ErrorKind::BrokenPipe
                        | io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(shutdown_requested);
            }
            Err(error) => {
                reject_decode_error(connection, 1, &error);
                return Ok(shutdown_requested);
            }
        };
        let request = match protocol::decode_request(&body) {
            Ok(request) => request,
            Err(error) => {
                let request_id = protocol::request_id_from_body(&body).unwrap_or(1);
                reject_decode_error(connection, request_id, &error);
                return Ok(shutdown_requested);
            }
        };

        if request_count == MAX_REQUESTS_PER_CONNECTION {
            send_terminal_response(
                connection,
                request.request_id,
                Response::Error(ErrorCode::RequestLimit),
            )?;
            return Ok(shutdown_requested);
        }
        if request_ids[..request_count].contains(&request.request_id) {
            send_terminal_response(
                connection,
                request.request_id,
                Response::Error(ErrorCode::DuplicateRequestId),
            )?;
            return Ok(shutdown_requested);
        }
        request_ids[request_count] = request.request_id;
        request_count += 1;

        let response = if !authenticated {
            match request.message {
                Request::Hello {
                    auth_secret,
                    executable_version,
                } if secrets_equal(secret, &auth_secret) => {
                    authenticated = true;
                    version_matches = executable_version == ENGINE_VERSION;
                    Response::Hello {
                        engine_version: ENGINE_VERSION.to_string(),
                        config_schema_version: CONFIG_SCHEMA_VERSION,
                        capabilities: CAPABILITIES,
                    }
                }
                Request::Hello { .. } => {
                    send_terminal_response(
                        connection,
                        request.request_id,
                        Response::Error(ErrorCode::AuthenticationFailed),
                    )?;
                    return Ok(shutdown_requested);
                }
                _ => {
                    send_terminal_response(
                        connection,
                        request.request_id,
                        Response::Error(ErrorCode::HelloRequired),
                    )?;
                    return Ok(shutdown_requested);
                }
            }
        } else {
            dispatch_request(
                request.message,
                version_matches,
                context,
                &mut shutdown_requested,
            )?
        };
        send_response(connection, request.request_id, response)?;
    }
}

fn dispatch_request(
    request: Request,
    version_matches: bool,
    context: &mut ConnectionContext<'_>,
    shutdown_requested: &mut bool,
) -> Result<Response, ControlError> {
    let response = match request {
        Request::Hello { .. } => Response::Error(ErrorCode::InvalidMessage),
        Request::Ping => Response::Pong,
        Request::GetStatus => {
            let config = context.config_owner.status(Instant::now());
            Response::Status(process_status(context.started_at, config)?)
        }
        Request::Shutdown if version_matches => {
            let already_requested = *shutdown_requested;
            *shutdown_requested = true;
            Response::Shutdown { already_requested }
        }
        Request::Shutdown => Response::Error(ErrorCode::ExecutableVersionMismatch),
        Request::GetConfig => match context.config_owner.current_bytes(Instant::now()) {
            Ok((revision, generation, config_bytes)) => Response::Config {
                revision,
                generation,
                config_bytes,
            },
            Err(error) => Response::Error(config_error_code(error)),
        },
        Request::PrepareConfig { .. }
        | Request::CommitConfig { .. }
        | Request::SetEnabled { .. }
        | Request::BeginWindowCapture { .. }
        | Request::PollWindowCapture { .. }
        | Request::CancelWindowCapture { .. }
            if !version_matches =>
        {
            Response::Error(ErrorCode::ExecutableVersionMismatch)
        }
        Request::PrepareConfig {
            expected_revision,
            config_bytes,
        } => {
            match context.config_owner.prepare(
                context.session,
                expected_revision,
                &config_bytes,
                Instant::now(),
            ) {
                Ok(prepared) => Response::Prepared {
                    token: prepared.token.0,
                    base_revision: prepared.base_revision,
                    base_generation: prepared.base_generation,
                },
                Err(error) => Response::Error(config_error_code(error)),
            }
        }
        Request::CommitConfig {
            token,
            base_revision,
            base_generation,
        } => match context.config_owner.commit(
            context.session,
            PreparedToken(token),
            base_revision,
            base_generation,
            Instant::now(),
        ) {
            Ok(applied) => applied_response(context.config_owner, applied, context.on_applied)?,
            Err(error) => Response::Error(config_error_code(error)),
        },
        Request::SetEnabled {
            expected_revision,
            enabled,
        } => match context.config_owner.set_enabled(
            context.session,
            expected_revision,
            enabled,
            Instant::now(),
        ) {
            Ok(applied) => applied_response(context.config_owner, applied, context.on_applied)?,
            Err(error) => Response::Error(config_error_code(error)),
        },
        Request::BeginWindowCapture { capture_id } => {
            match context.capture.begin(context.session, capture_id) {
                Ok(epoch) => {
                    debug!("window capture id={capture_id} epoch={epoch} phase=begin");
                    Response::WindowCaptureStarted { capture_id, epoch }
                }
                Err(error) => {
                    let code = capture_error_code(error);
                    warn!("window capture id={capture_id} phase=begin error={code:?}");
                    Response::Error(code)
                }
            }
        }
        Request::PollWindowCapture { capture_id, epoch } => {
            match context.capture.poll(context.session, capture_id, epoch) {
                Ok(CapturePoll::Pending) => Response::WindowCapture {
                    capture_id,
                    epoch,
                    result: WindowCaptureResult::Pending,
                },
                Ok(CapturePoll::Captured(point)) => match (context.resolve_capture)(point) {
                    Ok(info) => {
                        debug!("window capture id={capture_id} epoch={epoch} phase=captured");
                        Response::WindowCapture {
                            capture_id,
                            epoch,
                            result: WindowCaptureResult::Captured(WindowCaptureInfo {
                                process_name: info.process_name,
                                window_class: info.window_class,
                                title: info.title,
                            }),
                        }
                    }
                    Err(()) => {
                        warn!(
                            "window capture id={capture_id} epoch={epoch} phase=resolve error={:?}",
                            ErrorCode::CaptureBackendFailed
                        );
                        Response::Error(ErrorCode::CaptureBackendFailed)
                    }
                },
                Err(error) => {
                    let code = capture_error_code(error);
                    warn!("window capture id={capture_id} epoch={epoch} phase=poll error={code:?}");
                    Response::Error(code)
                }
            }
        }
        Request::CancelWindowCapture { capture_id, epoch } => {
            match context.capture.cancel(context.session, capture_id, epoch) {
                Ok(()) => {
                    debug!("window capture id={capture_id} epoch={epoch} phase=cancel");
                    Response::WindowCaptureCancelled { capture_id, epoch }
                }
                Err(error) => {
                    let code = capture_error_code(error);
                    warn!(
                        "window capture id={capture_id} epoch={epoch} phase=cancel error={code:?}"
                    );
                    Response::Error(code)
                }
            }
        }
    };
    Ok(response)
}

fn applied_response(
    config_owner: &ConfigOwner,
    applied: config::AppliedConfig,
    on_applied: &mut dyn FnMut(&config::ActiveConfig, u64) -> Result<(), ControlError>,
) -> Result<Response, ControlError> {
    on_applied(
        config_owner
            .active()
            .ok_or_else(|| ControlError::projection("successful commit has no active config"))?,
        applied.generation,
    )?;
    Ok(Response::Applied {
        revision: applied.revision,
        generation: applied.generation,
        durability_warning: applied.durability_warning,
    })
}

fn config_error_code(error: ConfigOwnerError) -> ErrorCode {
    match error {
        ConfigOwnerError::PayloadTooLarge => ErrorCode::ConfigPayloadTooLarge,
        ConfigOwnerError::Busy => ErrorCode::ConfigBusy,
        ConfigOwnerError::RevisionConflict => ErrorCode::ConfigRevisionConflict,
        ConfigOwnerError::ValidationFailed => ErrorCode::ConfigValidationFailed,
        ConfigOwnerError::TokenMismatch => ErrorCode::ConfigTokenMismatch,
        ConfigOwnerError::NoPreparedConfig => ErrorCode::NoPreparedConfig,
        ConfigOwnerError::GenerationExhausted => ErrorCode::ConfigGenerationExhausted,
        ConfigOwnerError::PersistenceFailed => ErrorCode::ConfigPersistenceFailed,
    }
}

fn capture_error_code(error: CaptureError) -> ErrorCode {
    match error {
        CaptureError::Stale => ErrorCode::CaptureStale,
        CaptureError::Unavailable => ErrorCode::CaptureUnavailable,
    }
}

fn reject_decode_error(
    connection: &mut platform::AcceptedConnection<'_>,
    request_id: u64,
    error: &ProtocolError,
) {
    let code = match error {
        ProtocolError::WrongVersion(_) => ErrorCode::WrongVersion,
        _ => ErrorCode::InvalidMessage,
    };
    let _ = send_terminal_response(connection, request_id, Response::Error(code));
}

fn send_response(
    connection: &mut platform::AcceptedConnection<'_>,
    request_id: u64,
    response: Response,
) -> Result<(), ControlError> {
    let body = protocol::encode_response(&Envelope::current(request_id, response))?;
    connection.set_deadline(Instant::now() + IO_TIMEOUT)?;
    protocol::write_frame(connection, &body)?;
    Ok(())
}

fn send_terminal_response(
    connection: &mut platform::AcceptedConnection<'_>,
    request_id: u64,
    response: Response,
) -> Result<(), ControlError> {
    send_response(connection, request_id, response)?;
    connection.set_deadline(Instant::now() + TERMINAL_RESPONSE_GRACE)?;
    let mut ignored = [0_u8; 1];
    let _ = connection.read(&mut ignored);
    Ok(())
}

pub(super) struct Session {
    connection: platform::ClientConnection,
    next_request_id: u64,
}

impl Session {
    pub(super) fn connect(endpoint: &platform::Endpoint) -> Result<Self, ControlError> {
        Self::connect_before(endpoint, Instant::now() + CONNECT_TIMEOUT)
    }

    pub(super) fn connect_before(
        endpoint: &platform::Endpoint,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let connection = endpoint.connect_before(deadline)?;
        let auth_secret = endpoint.read_secret()?;
        let mut session = Self {
            connection,
            next_request_id: 1,
        };
        match session.exchange_before(
            Request::Hello {
                auth_secret,
                executable_version: ENGINE_VERSION.to_string(),
            },
            deadline,
        )? {
            Response::Hello { capabilities, .. } if capabilities == CAPABILITIES => Ok(session),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    pub(super) fn exchange(&mut self, request: Request) -> Result<Response, ControlError> {
        self.exchange_before(request, Instant::now() + IO_TIMEOUT)
    }

    fn exchange_before(
        &mut self,
        request: Request,
        deadline: Instant,
    ) -> Result<Response, ControlError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(ControlError::Protocol(ProtocolError::InvalidMessage))?;
        self.exchange_with_id_before(request_id, request, deadline)
    }

    #[cfg(test)]
    pub(super) fn exchange_with_id(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> Result<Response, ControlError> {
        self.exchange_with_id_before(request_id, request, Instant::now() + IO_TIMEOUT)
    }

    #[cfg(test)]
    pub(super) fn send_then_disconnect(mut self, request: Request) -> Result<(), ControlError> {
        let request_id = self.next_request_id;
        let body = protocol::encode_request(&Envelope::current(request_id, request))?;
        self.connection.set_deadline(Instant::now() + IO_TIMEOUT)?;
        protocol::write_frame(&mut self.connection, &body)?;
        Ok(())
    }

    fn exchange_with_id_before(
        &mut self,
        request_id: u64,
        request: Request,
        deadline: Instant,
    ) -> Result<Response, ControlError> {
        let body = protocol::encode_request(&Envelope::current(request_id, request))?;
        self.connection.set_deadline(deadline)?;
        protocol::write_frame(&mut self.connection, &body)?;
        self.connection.set_deadline(deadline)?;
        let response_body = protocol::read_frame(&mut self.connection)?;
        Ok(protocol::decode_response(&response_body, request_id)?.message)
    }

    pub(super) fn current_config(&mut self) -> Result<ConfigObservation, ControlError> {
        match self.exchange(Request::GetConfig)? {
            Response::Config {
                revision,
                generation,
                config_bytes,
            } => {
                let config = config_bytes
                    .map(|bytes| {
                        config::decode_and_compile(&bytes).map(|active| active.document().clone())
                    })
                    .transpose()
                    .map_err(|_| ControlError::Rejected(ErrorCode::ConfigValidationFailed))?;
                Ok(ConfigObservation {
                    revision,
                    generation,
                    config,
                })
            }
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }
}

fn process_status(
    started_at: Instant,
    config: ConfigOwnerStatus,
) -> Result<EngineStatus, ControlError> {
    let (thread_count, handle_count, working_set_bytes) = platform::process_resources()?;
    Ok(EngineStatus {
        role: protocol::ProcessRole::Engine,
        webview_count: 0,
        process_id: std::process::id(),
        uptime_ms: started_at
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        thread_count,
        handle_count,
        working_set_bytes,
        config_available: config.available,
        config_revision: config.revision,
        config_generation: config.generation,
        config_candidate_prepared: config.candidate_prepared,
    })
}

pub(super) fn secrets_equal(
    expected: &[u8; AUTH_SECRET_BYTES],
    actual: &[u8; AUTH_SECRET_BYTES],
) -> bool {
    expected
        .iter()
        .zip(actual)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
