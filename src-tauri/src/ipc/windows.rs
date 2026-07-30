use super::protocol::{
    self, EngineStatus, Envelope, ErrorCode, ProcessRole, ProtocolError, Request, Response,
    AUTH_SECRET_BYTES, CAPABILITIES, MAX_REQUESTS_PER_CONNECTION,
};
use crate::config::{self, ConfigOwner, ConfigOwnerError, ConfigOwnerStatus, PreparedToken};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE,
    ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, CREATE_ALWAYS, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_MODE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState, PIPE_NOWAIT,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetProcessHandleCount, OpenProcessToken, ReleaseMutex,
    WaitForSingleObject,
};

const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_SCHEMA_VERSION: u16 = 2;
const IO_TIMEOUT: Duration = Duration::from_millis(750);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_INTERVAL: Duration = Duration::from_millis(40);
const PIPE_POLL_INTERVAL: Duration = Duration::from_millis(2);
const TERMINAL_RESPONSE_GRACE: Duration = Duration::from_millis(100);
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const PIPE_MODE: u32 =
    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS;
#[cfg(debug_assertions)]
const TEST_FAIL_FIRST_PIPE_ENV: &str = "ZG_P03_TEST_FAIL_FIRST_PIPE";

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

#[derive(Clone)]
pub struct EngineControl {
    endpoint: Endpoint,
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

impl EngineControl {
    pub fn connect_or_start(executable: &Path, config_dir: &Path) -> Result<Self, ControlError> {
        let control = Self {
            endpoint: Endpoint::current_user(config_dir, "")?,
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
    fn ping(&self) -> Result<(), ControlError> {
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
    ) -> Result<ConfigApplyResult, ControlError> {
        let bytes = serde_json::to_vec(&document)
            .map_err(|_| ControlError::Rejected(ErrorCode::ConfigValidationFailed))?;
        self.apply_config_bytes(bytes, expected_revision)
    }

    pub(crate) fn apply_config_bytes(
        &self,
        bytes: Vec<u8>,
        expected_revision: u64,
    ) -> Result<ConfigApplyResult, ControlError> {
        let mut session = Session::connect(&self.endpoint)?;
        let prepared = match session.exchange(Request::PrepareConfig {
            expected_revision,
            config_bytes: bytes,
        })? {
            Response::Prepared {
                token,
                base_revision,
                base_generation,
            } => (token, base_revision, base_generation),
            Response::Error(code) => return Err(ControlError::Rejected(code)),
            _ => return Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        };
        let durability_warning = match session.exchange(Request::CommitConfig {
            token: prepared.0,
            base_revision: prepared.1,
            base_generation: prepared.2,
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
        let mut session = Session::connect(&self.endpoint)?;
        match session.exchange(Request::Shutdown)? {
            Response::Shutdown { already_requested } => Ok(already_requested),
            Response::Error(code) => Err(ControlError::Rejected(code)),
            _ => Err(ControlError::Protocol(ProtocolError::InvalidMessage)),
        }
    }

    fn connect_or_start_with(
        &self,
        spawn: impl FnOnce() -> Result<(), ControlError>,
    ) -> Result<(), ControlError> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        match self.ping_before(deadline) {
            Ok(()) => return Ok(()),
            Err(ControlError::Unavailable) => {}
            Err(error) => return Err(error),
        }

        let security = SecurityDescriptor::for_sid(&self.endpoint.sid)?;
        let _launch = LaunchLock::acquire(&self.endpoint.launch_mutex_name, &security, deadline)?;
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
    fn for_test(config_dir: &Path, suffix: &str) -> Result<Self, ControlError> {
        Ok(Self {
            endpoint: Endpoint::current_user(config_dir, suffix)?,
        })
    }

    pub(crate) fn for_prepared_server(server: &EngineServer) -> Self {
        Self {
            endpoint: server.endpoint.clone(),
        }
    }
}

pub struct EngineServer {
    endpoint: Endpoint,
    _singleton: Singleton,
    secret: [u8; AUTH_SECRET_BYTES],
    _secret_file: SecretFile,
    first_pipe: Pipe,
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
    fn for_test(config_dir: &Path, suffix: &str) -> Result<Option<Self>, ControlError> {
        Self::with_suffix(config_dir, suffix)
    }

    fn with_suffix(config_dir: &Path, suffix: &str) -> Result<Option<Self>, ControlError> {
        let endpoint = Endpoint::current_user(config_dir, suffix)?;
        let security = SecurityDescriptor::for_sid(&endpoint.sid)?;
        let Some(singleton) = Singleton::acquire(&endpoint.mutex_name, &security)? else {
            return Ok(None);
        };
        fs::create_dir_all(&endpoint.config_dir)?;
        let secret = generate_secret()?;
        let secret_file = SecretFile::create(&endpoint.secret_path, &secret, &security)?;
        let first_pipe = Pipe::server(&endpoint.pipe_name, &security)?;
        Ok(Some(Self {
            endpoint,
            _singleton: singleton,
            secret,
            _secret_file: secret_file,
            first_pipe,
        }))
    }

    pub fn run<F>(
        self,
        stop: Arc<AtomicBool>,
        mut config_owner: ConfigOwner,
        mut on_applied: F,
    ) -> Result<ServerExit, ControlError>
    where
        F: FnMut(&config::ActiveConfig, u64),
    {
        let Self {
            endpoint: _,
            _singleton,
            secret,
            _secret_file,
            mut first_pipe,
        } = self;
        let started_at = Instant::now();
        let mut next_session = 1_u64;

        while !stop.load(Ordering::Acquire) {
            if !first_pipe.wait_for_client(&stop)? {
                return Ok(ServerExit::Stopped);
            }
            let session = next_session;
            next_session = next_session.checked_add(1).unwrap_or(1);
            if serve_connection(
                &mut first_pipe,
                &secret,
                started_at,
                &mut config_owner,
                session,
                &mut on_applied,
            ) {
                return Ok(ServerExit::Shutdown);
            }
        }
        Ok(ServerExit::Stopped)
    }
}

fn serve_connection(
    pipe: &mut Pipe,
    secret: &[u8; AUTH_SECRET_BYTES],
    started_at: Instant,
    config_owner: &mut ConfigOwner,
    session: u64,
    on_applied: &mut impl FnMut(&config::ActiveConfig, u64),
) -> bool {
    let result =
        serve_connection_inner(pipe, secret, started_at, config_owner, session, on_applied);
    config_owner.disconnect(session);
    pipe.disconnect_client();
    result.unwrap_or(false)
}

fn serve_connection_inner(
    pipe: &mut Pipe,
    secret: &[u8; AUTH_SECRET_BYTES],
    started_at: Instant,
    config_owner: &mut ConfigOwner,
    session: u64,
    on_applied: &mut impl FnMut(&config::ActiveConfig, u64),
) -> Result<bool, ControlError> {
    let mut authenticated = false;
    let mut version_matches = false;
    let mut shutdown_requested = false;
    let mut request_ids = [0_u64; MAX_REQUESTS_PER_CONNECTION];
    let mut request_count = 0;

    loop {
        pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        let body = match protocol::read_frame(&mut *pipe) {
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
                reject_decode_error(pipe, &body_request_id_placeholder(&error), &error);
                return Ok(shutdown_requested);
            }
        };
        let request = match protocol::decode_request(&body) {
            Ok(request) => request,
            Err(error) => {
                let request_id = protocol::request_id_from_body(&body).unwrap_or(1);
                reject_decode_error(pipe, &request_id, &error);
                return Ok(shutdown_requested);
            }
        };

        if request_count == MAX_REQUESTS_PER_CONNECTION {
            send_terminal_response(
                pipe,
                request.request_id,
                Response::Error(ErrorCode::RequestLimit),
            )?;
            return Ok(shutdown_requested);
        }
        if request_ids[..request_count].contains(&request.request_id) {
            send_terminal_response(
                pipe,
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
                        pipe,
                        request.request_id,
                        Response::Error(ErrorCode::AuthenticationFailed),
                    )?;
                    return Ok(shutdown_requested);
                }
                _ => {
                    send_terminal_response(
                        pipe,
                        request.request_id,
                        Response::Error(ErrorCode::HelloRequired),
                    )?;
                    return Ok(shutdown_requested);
                }
            }
        } else {
            match request.message {
                Request::Hello { .. } => Response::Error(ErrorCode::InvalidMessage),
                Request::Ping => Response::Pong,
                Request::GetStatus => {
                    let config = config_owner.status(Instant::now());
                    Response::Status(process_status(started_at, config)?)
                }
                Request::Shutdown if version_matches => {
                    let already_requested = shutdown_requested;
                    shutdown_requested = true;
                    Response::Shutdown { already_requested }
                }
                Request::Shutdown => Response::Error(ErrorCode::ExecutableVersionMismatch),
                Request::GetConfig => match config_owner.current_bytes(Instant::now()) {
                    Ok((revision, generation, config_bytes)) => Response::Config {
                        revision,
                        generation,
                        config_bytes,
                    },
                    Err(error) => Response::Error(config_error_code(error)),
                },
                Request::PrepareConfig { .. } | Request::CommitConfig { .. }
                    if !version_matches =>
                {
                    Response::Error(ErrorCode::ExecutableVersionMismatch)
                }
                Request::PrepareConfig {
                    expected_revision,
                    config_bytes,
                } => match config_owner.prepare(
                    session,
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
                },
                Request::CommitConfig {
                    token,
                    base_revision,
                    base_generation,
                } => match config_owner.commit(
                    session,
                    PreparedToken(token),
                    base_revision,
                    base_generation,
                    Instant::now(),
                ) {
                    Ok(applied) => {
                        on_applied(
                            config_owner
                                .active()
                                .expect("successful commit must install active config"),
                            applied.generation,
                        );
                        Response::Applied {
                            revision: applied.revision,
                            generation: applied.generation,
                            durability_warning: applied.durability_warning,
                        }
                    }
                    Err(error) => Response::Error(config_error_code(error)),
                },
            }
        };
        send_response(pipe, request.request_id, response)?;
    }
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

fn body_request_id_placeholder(_error: &ProtocolError) -> u64 {
    1
}

fn reject_decode_error(pipe: &mut Pipe, request_id: &u64, error: &ProtocolError) {
    let code = match error {
        ProtocolError::WrongVersion(_) => ErrorCode::WrongVersion,
        _ => ErrorCode::InvalidMessage,
    };
    let _ = send_terminal_response(pipe, *request_id, Response::Error(code));
}

fn send_response(pipe: &mut Pipe, request_id: u64, response: Response) -> Result<(), ControlError> {
    let body = protocol::encode_response(&Envelope::current(request_id, response))?;
    pipe.set_deadline(Instant::now() + IO_TIMEOUT);
    protocol::write_frame(pipe, &body)?;
    Ok(())
}

fn send_terminal_response(
    pipe: &mut Pipe,
    request_id: u64,
    response: Response,
) -> Result<(), ControlError> {
    send_response(pipe, request_id, response)?;
    pipe.set_deadline(Instant::now() + TERMINAL_RESPONSE_GRACE);
    let mut ignored = [0_u8; 1];
    let _ = pipe.read(&mut ignored);
    Ok(())
}

struct Session {
    pipe: Pipe,
    next_request_id: u64,
}

impl Session {
    fn connect(endpoint: &Endpoint) -> Result<Self, ControlError> {
        Self::connect_before(endpoint, Instant::now() + CONNECT_TIMEOUT)
    }

    fn connect_before(endpoint: &Endpoint, deadline: Instant) -> Result<Self, ControlError> {
        let pipe = connect_pipe_before(endpoint, deadline)?;
        let secret_bytes = fs::read(&endpoint.secret_path).map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                ControlError::Security(format!("cannot read Engine secret file: {error}"))
            } else {
                ControlError::Io(error)
            }
        })?;
        let auth_secret: [u8; AUTH_SECRET_BYTES] = secret_bytes
            .try_into()
            .map_err(|_| ControlError::Security("invalid Engine secret file".to_string()))?;
        let mut session = Self {
            pipe,
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

    fn exchange(&mut self, request: Request) -> Result<Response, ControlError> {
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
    fn exchange_with_id(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> Result<Response, ControlError> {
        self.exchange_with_id_before(request_id, request, Instant::now() + IO_TIMEOUT)
    }

    #[cfg(test)]
    fn send_then_disconnect(mut self, request: Request) -> Result<(), ControlError> {
        let request_id = self.next_request_id;
        let body = protocol::encode_request(&Envelope::current(request_id, request))?;
        self.pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        protocol::write_frame(&mut self.pipe, &body)?;
        Ok(())
    }

    fn exchange_with_id_before(
        &mut self,
        request_id: u64,
        request: Request,
        deadline: Instant,
    ) -> Result<Response, ControlError> {
        let body = protocol::encode_request(&Envelope::current(request_id, request))?;
        self.pipe.set_deadline(deadline);
        protocol::write_frame(&mut self.pipe, &body)?;
        self.pipe.set_deadline(deadline);
        let response_body = protocol::read_frame(&mut self.pipe)?;
        Ok(protocol::decode_response(&response_body, request_id)?.message)
    }

    fn current_config(&mut self) -> Result<ConfigObservation, ControlError> {
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

#[cfg(test)]
fn connect_pipe(endpoint: &Endpoint) -> Result<Pipe, ControlError> {
    connect_pipe_before(endpoint, Instant::now() + IO_TIMEOUT)
}

fn connect_pipe_before(endpoint: &Endpoint, deadline: Instant) -> Result<Pipe, ControlError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ControlError::Timeout);
        }
        match Pipe::client(&endpoint.pipe_name, remaining.min(IO_TIMEOUT)) {
            Ok(pipe) => return Ok(pipe),
            Err(ControlError::Unavailable) => return Err(ControlError::Unavailable),
            Err(ControlError::EndpointBusy) if Instant::now() < deadline => {
                thread::sleep(
                    PIPE_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(ControlError::EndpointBusy) => return Err(ControlError::Timeout),
            Err(error) => return Err(error),
        }
    }
}

#[derive(Clone)]
struct Endpoint {
    pipe_name: Vec<u16>,
    mutex_name: Vec<u16>,
    launch_mutex_name: Vec<u16>,
    secret_path: PathBuf,
    config_dir: PathBuf,
    sid: String,
}

impl Endpoint {
    fn current_user(config_dir: &Path, suffix: &str) -> Result<Self, ControlError> {
        if !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ControlError::Security(
                "invalid internal endpoint suffix".to_string(),
            ));
        }
        let sid = current_user_sid()?;
        let suffix = if suffix.is_empty() {
            String::new()
        } else {
            format!(".{suffix}")
        };
        Ok(Self {
            pipe_name: wide(format!(
                r"\\.\pipe\dev.r4ai.zero-gesture.engine.{sid}{suffix}"
            )),
            mutex_name: wide(format!(r"Local\dev.r4ai.zero-gesture.engine.{sid}{suffix}")),
            launch_mutex_name: wide(format!(
                r"Local\dev.r4ai.zero-gesture.engine-launch.{sid}{suffix}"
            )),
            secret_path: config_dir.join(format!("engine-control{suffix}.secret")),
            config_dir: config_dir.to_path_buf(),
            sid,
        })
    }
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn for_sid(sid: &str) -> Result<Self, ControlError> {
        let sddl = wide(current_user_only_sddl(sid));
        let mut descriptor = null_mut();
        let success = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if success == 0 {
            return Err(last_security_error("build current-user DACL"));
        }
        Ok(Self(descriptor))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

fn current_user_only_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct Singleton {
    _handle: OwnedHandle,
}

impl Singleton {
    fn acquire(name: &[u16], security: &SecurityDescriptor) -> Result<Option<Self>, ControlError> {
        let attributes = security.attributes();
        let handle = unsafe { CreateMutexW(&attributes, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_security_error("create per-user Engine mutex"));
        }
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle = OwnedHandle(handle);
        if already_exists {
            Ok(None)
        } else {
            Ok(Some(Self { _handle: handle }))
        }
    }
}

struct LaunchLock {
    handle: OwnedHandle,
}

impl LaunchLock {
    fn acquire(
        name: &[u16],
        security: &SecurityDescriptor,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let attributes = security.attributes();
        let handle = unsafe { CreateMutexW(&attributes, 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_security_error("create per-user Engine launch mutex"));
        }
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle = OwnedHandle(handle);
        if already_exists {
            let remaining = u32::try_from(
                deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u32::MAX);
            match unsafe { WaitForSingleObject(handle.0, remaining) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => {}
                WAIT_TIMEOUT => return Err(ControlError::Timeout),
                _ => {
                    return Err(ControlError::Io(io::Error::last_os_error()));
                }
            }
        }
        Ok(Self { handle })
    }
}

impl Drop for LaunchLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle.0);
        }
    }
}

struct SecretFile {
    path: PathBuf,
}

impl SecretFile {
    fn create(
        path: &Path,
        secret: &[u8; AUTH_SECRET_BYTES],
        security: &SecurityDescriptor,
    ) -> Result<Self, ControlError> {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ControlError::Io(error)),
        }
        let path_wide = wide(path.as_os_str());
        let attributes = security.attributes();
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_MODE::default(),
                &attributes,
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_HIDDEN,
                null_mut(),
            )
        };
        let handle = OwnedHandle::from_file(handle, "create Engine secret file")?;
        let mut written = 0;
        let success = unsafe {
            WriteFile(
                handle.0,
                secret.as_ptr(),
                AUTH_SECRET_BYTES as u32,
                &mut written,
                null_mut(),
            )
        };
        if success == 0 || written != AUTH_SECRET_BYTES as u32 {
            return Err(ControlError::Io(io::Error::last_os_error()));
        }
        if unsafe { FlushFileBuffers(handle.0) } == 0 {
            return Err(ControlError::Io(io::Error::last_os_error()));
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct Pipe {
    handle: OwnedHandle,
    deadline: Instant,
    server: bool,
}

impl Pipe {
    fn server(name: &[u16], security: &SecurityDescriptor) -> Result<Self, ControlError> {
        #[cfg(debug_assertions)]
        if std::env::var_os(TEST_FAIL_FIRST_PIPE_ENV).is_some() {
            return Err(ControlError::Security(
                "injected first named-pipe bind failure".to_string(),
            ));
        }
        let attributes = security.attributes();
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_MODE,
                1,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                IO_TIMEOUT.as_millis() as u32,
                &attributes,
            )
        };
        Ok(Self {
            handle: OwnedHandle::from_file(handle, "create Engine named pipe")?,
            deadline: Instant::now() + IO_TIMEOUT,
            server: true,
        })
    }

    fn client(name: &[u16], timeout: Duration) -> Result<Self, ControlError> {
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_MODE::default(),
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND => Err(ControlError::Unavailable),
                ERROR_PIPE_BUSY => Err(ControlError::EndpointBusy),
                _ => Err(ControlError::Io(io::Error::last_os_error())),
            };
        }
        let handle = OwnedHandle::from_file(handle, "connect to Engine named pipe")?;
        let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
        if unsafe { SetNamedPipeHandleState(handle.0, &mode, null(), null()) } == 0 {
            return Err(ControlError::Io(io::Error::last_os_error()));
        }
        Ok(Self {
            handle,
            deadline: Instant::now() + timeout,
            server: false,
        })
    }

    fn wait_for_client(&self, stop: &AtomicBool) -> Result<bool, ControlError> {
        while !stop.load(Ordering::Acquire) {
            if unsafe { ConnectNamedPipe(self.handle.0, null_mut()) } != 0 {
                return Ok(true);
            }
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => return Ok(true),
                ERROR_PIPE_LISTENING | ERROR_NO_DATA => thread::sleep(RETRY_INTERVAL),
                _ => return Err(ControlError::Io(io::Error::last_os_error())),
            }
        }
        Ok(false)
    }

    fn disconnect_client(&self) {
        debug_assert!(self.server);
        unsafe {
            DisconnectNamedPipe(self.handle.0);
        }
    }

    fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    fn retry_or_timeout(&self, error: u32) -> io::Result<usize> {
        match error {
            ERROR_NO_DATA | ERROR_PIPE_LISTENING | ERROR_PIPE_BUSY => {
                if Instant::now() >= self.deadline {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "named pipe deadline",
                    ))
                } else {
                    thread::sleep(PIPE_POLL_INTERVAL);
                    Ok(0)
                }
            }
            ERROR_BROKEN_PIPE => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "named pipe closed",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Read for Pipe {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut read = 0;
            let success = unsafe {
                ReadFile(
                    self.handle.0,
                    output.as_mut_ptr(),
                    u32::try_from(output.len()).unwrap_or(u32::MAX),
                    &mut read,
                    null_mut(),
                )
            };
            if success != 0 {
                return Ok(read as usize);
            }
            let result = self.retry_or_timeout(unsafe { GetLastError() })?;
            if result != 0 {
                return Ok(result);
            }
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        loop {
            let mut written = 0;
            let success = unsafe {
                WriteFile(
                    self.handle.0,
                    input.as_ptr(),
                    u32::try_from(input.len()).unwrap_or(u32::MAX),
                    &mut written,
                    null_mut(),
                )
            };
            if success != 0 {
                return Ok(written as usize);
            }
            let result = self.retry_or_timeout(unsafe { GetLastError() })?;
            if result != 0 {
                return Ok(result);
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        if self.server {
            self.disconnect_client();
        }
    }
}

struct OwnedHandle(HANDLE);

// Windows kernel handles are process-wide values and may be closed from a
// different thread than the one that created them. Ownership remains unique.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    fn from_file(handle: HANDLE, operation: &str) -> Result<Self, ControlError> {
        if handle == INVALID_HANDLE_VALUE {
            Err(ControlError::Security(format!(
                "{operation}: {}",
                io::Error::last_os_error()
            )))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn current_user_sid() -> Result<String, ControlError> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_security_error("open current process token"));
    }
    let token = OwnedHandle(token);
    let mut required = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
    }
    if required == 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(last_security_error("measure current user SID"));
    }
    let mut buffer = vec![0_u8; required as usize];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(last_security_error("read current user SID"));
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_string = null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) } == 0 {
        return Err(last_security_error("format current user SID"));
    }
    let length = (0..)
        .take_while(|index| unsafe { *sid_string.add(*index) } != 0)
        .count();
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_string, length) })
        .map_err(|_| ControlError::Security("current user SID is invalid UTF-16".to_string()))?;
    unsafe {
        LocalFree(sid_string.cast());
    }
    Ok(sid)
}

fn generate_secret() -> Result<[u8; AUTH_SECRET_BYTES], ControlError> {
    let mut secret = [0_u8; AUTH_SECRET_BYTES];
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            secret.as_mut_ptr(),
            secret.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        Err(ControlError::Security(format!(
            "generate Engine authentication secret: NTSTATUS {status:#x}"
        )))
    } else {
        Ok(secret)
    }
}

fn process_status(
    started_at: Instant,
    config: ConfigOwnerStatus,
) -> Result<EngineStatus, ControlError> {
    let process = unsafe { GetCurrentProcess() };
    let mut handle_count = 0;
    if unsafe { GetProcessHandleCount(process, &mut handle_count) } == 0 {
        return Err(ControlError::Io(io::Error::last_os_error()));
    }
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    if unsafe { GetProcessMemoryInfo(process, &mut memory, memory.cb) } == 0 {
        return Err(ControlError::Io(io::Error::last_os_error()));
    }
    Ok(EngineStatus {
        role: ProcessRole::Engine,
        webview_count: 0,
        process_id: std::process::id(),
        uptime_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        thread_count: current_process_thread_count()?,
        handle_count,
        working_set_bytes: u64::try_from(memory.WorkingSetSize).unwrap_or(u64::MAX),
        config_available: config.available,
        config_revision: config.revision,
        config_generation: config.generation,
        config_candidate_prepared: config.candidate_prepared,
    })
}

fn current_process_thread_count() -> Result<u32, ControlError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, std::process::id()) };
    let snapshot = OwnedHandle::from_file(snapshot, "snapshot process threads")?;
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut count = 0_u32;
    if unsafe { Thread32First(snapshot.0, &mut entry) } != 0 {
        loop {
            if entry.th32OwnerProcessID == std::process::id() {
                count = count.saturating_add(1);
            }
            if unsafe { Thread32Next(snapshot.0, &mut entry) } == 0 {
                break;
            }
        }
    }
    Ok(count)
}

fn secrets_equal(expected: &[u8; AUTH_SECRET_BYTES], actual: &[u8; AUTH_SECRET_BYTES]) -> bool {
    expected
        .iter()
        .zip(actual)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn last_security_error(operation: &str) -> ControlError {
    ControlError::Security(format!("{operation}: {}", io::Error::last_os_error()))
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;
    use std::process::{Child, Stdio};
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::{Barrier, Mutex};
    use tempfile::TempDir;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSidToSidW, GetNamedSecurityInfoW, GetSecurityInfo, SE_FILE_OBJECT,
        SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::System::Pipes::{GetNamedPipeInfo, PIPE_SERVER_END};
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

    static NEXT_SUFFIX: AtomicU64 = AtomicU64::new(1);
    const HELPER_ENV: &str = "ZG_P03_PROCESS_HELPER";
    const HELPER_DIR_ENV: &str = "ZG_P03_PROCESS_HELPER_DIR";
    const HELPER_SUFFIX_ENV: &str = "ZG_P03_PROCESS_HELPER_SUFFIX";
    const HELPER_APPLIED_FILE: &str = "runtime-applied.json";

    fn fixture() -> (TempDir, String, EngineControl) {
        let directory = tempfile::tempdir().unwrap();
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            NEXT_SUFFIX.fetch_add(1, AtomicOrdering::Relaxed)
        );
        let control = EngineControl::for_test(directory.path(), &suffix).unwrap();
        (directory, suffix, control)
    }

    fn spawn_helper(directory: &Path, suffix: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ipc::windows::tests::process_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "1")
            .env(HELPER_DIR_ENV, directory)
            .env(HELPER_SUFFIX_ENV, suffix)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    struct ChildGuard {
        child: Child,
        control: EngineControl,
    }

    impl ChildGuard {
        fn new(child: Child, control: EngineControl) -> Self {
            Self { child, control }
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.control.shutdown();
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }

    #[test]
    fn process_helper() {
        if std::env::var_os(HELPER_ENV).is_none() {
            return;
        }
        let directory = PathBuf::from(std::env::var_os(HELPER_DIR_ENV).unwrap());
        let suffix = std::env::var(HELPER_SUFFIX_ENV).unwrap();
        let Some(server) = EngineServer::for_test(&directory, &suffix).unwrap() else {
            return;
        };
        let (owner, _) = ConfigOwner::startup(&directory);
        let applied_path = directory.join(HELPER_APPLIED_FILE);
        let exit = server
            .run(Arc::new(AtomicBool::new(false)), owner, move |active, _| {
                if let Ok(bytes) = config::encode(active) {
                    let _ = fs::write(&applied_path, bytes);
                }
            })
            .unwrap();
        assert_eq!(exit, ServerExit::Shutdown);
    }

    #[test]
    fn authenticated_client_rejects_wrong_secret() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control
            .connect_or_start_with(|| Ok(()))
            .expect("helper must become ready");
        let wrong_endpoint = control.endpoint.clone();
        fs::write(&wrong_endpoint.secret_path, [0_u8; AUTH_SECRET_BYTES]).unwrap();
        assert!(matches!(
            Session::connect(&wrong_endpoint),
            Err(ControlError::Rejected(ErrorCode::AuthenticationFailed))
        ));
    }

    #[test]
    fn existing_pipe_with_locked_secret_is_terminal_and_does_not_spawn() {
        let (directory, suffix, control) = fixture();
        let _server = EngineServer::for_test(directory.path(), &suffix)
            .unwrap()
            .unwrap();
        let path_wide = wide(control.endpoint.secret_path.as_os_str());
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_MODE::default(),
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        let _handle = OwnedHandle::from_file(handle, "hold Engine secret file").unwrap();
        let spawn_count = AtomicU64::new(0);
        let started = Instant::now();

        assert!(matches!(
            control.connect_or_start_with(|| {
                spawn_count.fetch_add(1, AtomicOrdering::Relaxed);
                Ok(())
            }),
            Err(ControlError::Io(_))
        ));
        assert_eq!(spawn_count.load(AtomicOrdering::Relaxed), 0);
        assert!(started.elapsed() < IO_TIMEOUT);
    }

    #[test]
    fn one_instance_race_converges_on_one_engine() {
        let (directory, suffix, control) = fixture();
        let first = spawn_helper(directory.path(), &suffix);
        let second = spawn_helper(directory.path(), &suffix);
        let _first_guard = ChildGuard::new(first, control.clone());
        let _second_guard = ChildGuard::new(second, control.clone());
        control
            .connect_or_start_with(|| Ok(()))
            .expect("one helper must own the endpoint");
        assert_eq!(control.status().unwrap().role, ProcessRole::Engine);
    }

    #[test]
    fn settings_config_mutation_has_engine_as_only_writer() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let observed = control.current_config().unwrap();
        let mut document = observed.config.unwrap();
        document.shared.appearance.trail_thickness = 9.0;
        let applied = control
            .apply_config(document.clone(), observed.revision)
            .unwrap();

        assert_eq!(applied.current.config, Some(document.clone()));
        assert_eq!(control.current_config().unwrap().config, Some(document));
        let status = control.status().unwrap();
        assert_eq!((status.config_revision, status.config_generation), (2, 2));
        assert!(directory.path().join("zero-gesture.config.json").exists());
    }

    #[test]
    fn normal_windows_commit_returns_durability_warning_through_control() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let observed = control.current_config().unwrap();
        let mut changed = observed.config.unwrap();
        changed.shared.appearance.trail_thickness = 9.5;
        let applied = control.apply_config(changed, observed.revision).unwrap();

        assert!(applied.durability_warning);
        assert_eq!(
            (applied.current.revision, applied.current.generation),
            (2, 2)
        );
    }

    #[test]
    fn prepared_candidate_is_cleaned_when_settings_disconnects() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let mut session = Session::connect(&control.endpoint).unwrap();
        let observed = session.current_config().unwrap();
        let revision = observed.revision;
        let mut bytes =
            config::encode(&config::ActiveConfig::from_document(observed.config.unwrap()).unwrap())
                .unwrap();
        bytes.push(b' ');
        assert!(matches!(
            session
                .exchange(Request::PrepareConfig {
                    expected_revision: revision,
                    config_bytes: bytes,
                })
                .unwrap(),
            Response::Prepared { .. }
        ));
        drop(session);

        let observed = control.current_config().unwrap();
        let mut document = observed.config.unwrap();
        document.shared.enabled = !document.shared.enabled;
        control.apply_config(document, observed.revision).unwrap();
        assert!(!control.status().unwrap().config_candidate_prepared);
    }

    #[test]
    fn stale_settings_draft_conflicts_after_another_client_applies() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let first = control.current_config().unwrap();
        let second = control.current_config().unwrap();
        let mut first_document = first.config.unwrap();
        first_document.shared.appearance.trail_thickness = 8.0;
        control
            .apply_config(first_document, first.revision)
            .unwrap();
        let mut stale_document = second.config.unwrap();
        stale_document.shared.appearance.trail_thickness = 10.0;

        let error = control
            .apply_config(stale_document, second.revision)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ControlError::Rejected(ErrorCode::ConfigRevisionConflict)
            ),
            "unexpected stale-draft result: {error:?}"
        );
        assert_eq!(control.current_config().unwrap().revision, 2);
    }

    #[test]
    fn revision_zero_recovers_an_invalid_startup_config() {
        let (directory, suffix, control) = fixture();
        fs::write(
            directory.path().join("zero-gesture.config.json"),
            b"{not valid config",
        )
        .unwrap();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let unavailable = control.current_config().unwrap();
        assert_eq!(
            unavailable,
            ConfigObservation {
                revision: 0,
                generation: 0,
                config: None,
            }
        );
        let repaired = config::ConfigDocument::default();
        let applied = control.apply_config(repaired.clone(), 0).unwrap();

        assert_eq!(
            (applied.current.revision, applied.current.generation),
            (1, 1)
        );
        assert_eq!(applied.current.config, Some(repaired));
        assert!(control.status().unwrap().config_available);
    }

    #[test]
    fn applied_projects_enabled_binding_and_appearance_to_the_live_runtime() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let observed = control.current_config().unwrap();
        let mut changed = observed.config.unwrap();
        changed.shared.enabled = !changed.shared.enabled;
        changed.shared.appearance.trail_thickness = 6.0;
        match &mut changed.bindings[0] {
            config::BindingRecord::Shared(binding)
            | config::BindingRecord::Windows(binding)
            | config::BindingRecord::Macos(binding) => {
                binding.label = Some("live-applied-binding".to_string());
            }
        }
        control
            .apply_config(changed.clone(), observed.revision)
            .unwrap();

        let path = directory.path().join(HELPER_APPLIED_FILE);
        let deadline = Instant::now() + IO_TIMEOUT;
        let projected = loop {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(active) = config::decode_and_compile(&bytes) {
                    break active.document().clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "live projection was not observed"
            );
            thread::sleep(PIPE_POLL_INTERVAL);
        };
        assert_eq!(projected, changed);
    }

    #[test]
    fn bounded_auto_start_connects_to_spawned_engine() {
        let (directory, suffix, control) = fixture();
        let child_slot = std::sync::Mutex::new(None);
        let started = Instant::now();
        control
            .connect_or_start_with(|| {
                *child_slot.lock().unwrap() = Some(spawn_helper(directory.path(), &suffix));
                Ok(())
            })
            .unwrap();
        let child = child_slot.lock().unwrap().take().unwrap();
        let _guard = ChildGuard::new(child, control.clone());
        assert!(started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(1));
    }

    #[test]
    fn concurrent_settings_launch_storm_spawns_one_engine() {
        const CALLERS: usize = 8;
        let (directory, suffix, control) = fixture();
        let barrier = Arc::new(Barrier::new(CALLERS));
        let spawn_count = Arc::new(AtomicU64::new(0));
        let child_slot = Arc::new(Mutex::new(None));

        thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..CALLERS {
                let barrier = Arc::clone(&barrier);
                let spawn_count = Arc::clone(&spawn_count);
                let child_slot = Arc::clone(&child_slot);
                let control = control.clone();
                let directory = directory.path().to_path_buf();
                let suffix = suffix.clone();
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    control.connect_or_start_with(|| {
                        spawn_count.fetch_add(1, AtomicOrdering::Relaxed);
                        let child = spawn_helper(&directory, &suffix);
                        assert!(child_slot.lock().unwrap().replace(child).is_none());
                        Ok(())
                    })
                }));
            }
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        });

        let child = child_slot.lock().unwrap().take().unwrap();
        let _guard = ChildGuard::new(child, control);
        assert_eq!(spawn_count.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn authentication_error_does_not_spawn_engine() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        fs::write(&control.endpoint.secret_path, [0_u8; AUTH_SECRET_BYTES]).unwrap();
        let spawn_count = AtomicU64::new(0);

        let error = control
            .connect_or_start_with(|| {
                spawn_count.fetch_add(1, AtomicOrdering::Relaxed);
                Ok(())
            })
            .unwrap_err();

        assert!(matches!(
            error,
            ControlError::Rejected(ErrorCode::AuthenticationFailed)
        ));
        assert_eq!(spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn protocol_error_does_not_spawn_engine() {
        let (directory, _suffix, control) = fixture();
        let security = SecurityDescriptor::for_sid(&control.endpoint.sid).unwrap();
        fs::create_dir_all(directory.path()).unwrap();
        let secret = generate_secret().unwrap();
        let _secret_file =
            SecretFile::create(&control.endpoint.secret_path, &secret, &security).unwrap();
        let pipe = Pipe::server(&control.endpoint.pipe_name, &security).unwrap();
        let responder = thread::spawn(move || {
            let stop = AtomicBool::new(false);
            assert!(pipe.wait_for_client(&stop).unwrap());
            let mut pipe = pipe;
            pipe.set_deadline(Instant::now() + IO_TIMEOUT);
            let body = protocol::read_frame(&mut pipe).unwrap();
            let hello = protocol::decode_request(&body).unwrap();
            send_response(&mut pipe, hello.request_id, Response::Pong).unwrap();
            thread::sleep(TERMINAL_RESPONSE_GRACE);
        });
        let spawn_count = AtomicU64::new(0);

        let error = control
            .connect_or_start_with(|| {
                spawn_count.fetch_add(1, AtomicOrdering::Relaxed);
                Ok(())
            })
            .unwrap_err();

        responder.join().unwrap();
        assert!(
            matches!(
                &error,
                ControlError::Protocol(ProtocolError::InvalidMessage)
            ),
            "unexpected error: {error:?}"
        );
        assert_eq!(spawn_count.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn client_disconnect_before_response_does_not_stop_engine() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        Session::connect(&control.endpoint)
            .unwrap()
            .send_then_disconnect(Request::GetStatus)
            .unwrap();
        let started = Instant::now();
        loop {
            if let Ok(status) = control.status() {
                assert_eq!(status.webview_count, 0);
                break;
            }
            assert!(started.elapsed() < CONNECT_TIMEOUT);
            thread::sleep(RETRY_INTERVAL);
        }
    }

    #[test]
    fn client_disconnect_does_not_stop_engine() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        drop(Session::connect(&control.endpoint).unwrap());

        assert_eq!(control.status().unwrap().role, ProcessRole::Engine);
    }

    #[test]
    fn client_disconnect_after_commit_keeps_applied_truth_and_engine_alive() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();

        let mut session = Session::connect(&control.endpoint).unwrap();
        let observed = session.current_config().unwrap();
        let mut changed = observed.config.unwrap();
        changed.shared.appearance.trail_thickness = 11.0;
        let bytes = serde_json::to_vec(&changed).unwrap();
        let prepared = match session
            .exchange(Request::PrepareConfig {
                expected_revision: observed.revision,
                config_bytes: bytes,
            })
            .unwrap()
        {
            Response::Prepared {
                token,
                base_revision,
                base_generation,
            } => (token, base_revision, base_generation),
            response => panic!("unexpected Prepare response: {response:?}"),
        };
        session
            .send_then_disconnect(Request::CommitConfig {
                token: prepared.0,
                base_revision: prepared.1,
                base_generation: prepared.2,
            })
            .unwrap();

        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let current = loop {
            if let Ok(current) = control.current_config() {
                if current.revision == 2 {
                    break current;
                }
            }
            assert!(Instant::now() < deadline, "commit did not become queryable");
            thread::sleep(PIPE_POLL_INTERVAL);
        };
        assert_eq!(current.config, Some(changed));
        assert!(!control.status().unwrap().config_candidate_prepared);
    }

    #[test]
    fn status_reports_engine_role_and_server_process_id() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let child_process_id = child.id();
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let status = control.status().unwrap();
        assert_eq!(status.role, ProcessRole::Engine);
        assert_eq!(status.process_id, child_process_id);
    }

    #[test]
    fn status_reports_idle_resource_snapshot() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let status = control.status().unwrap();
        assert!(status.process_id > 0);
        assert!(status.thread_count > 0);
        assert!(status.handle_count > 0);
        assert!(status.working_set_bytes > 0);
    }

    #[test]
    fn duplicate_request_id_is_rejected() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut session = Session::connect(&control.endpoint).unwrap();
        assert_eq!(
            session.exchange_with_id(7, Request::Ping).unwrap(),
            Response::Pong
        );
        assert_eq!(
            session.exchange_with_id(7, Request::Ping).unwrap(),
            Response::Error(ErrorCode::DuplicateRequestId)
        );
    }

    #[test]
    fn request_before_hello_is_rejected() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut pipe = connect_pipe(&control.endpoint).unwrap();
        send_raw_request(&mut pipe, Envelope::current(9, Request::Ping));
        assert_eq!(
            read_raw_response(&mut pipe, 9),
            Response::Error(ErrorCode::HelloRequired)
        );
    }

    #[test]
    fn wrong_protocol_version_is_rejected_by_server() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut pipe = connect_pipe(&control.endpoint).unwrap();
        let mut body = protocol::encode_request(&Envelope::current(11, Request::Ping)).unwrap();
        body[..2].copy_from_slice(&(protocol::PROTOCOL_VERSION + 1).to_le_bytes());
        pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        protocol::write_frame(&mut pipe, &body).unwrap();
        assert_eq!(
            read_raw_response(&mut pipe, 11),
            Response::Error(ErrorCode::WrongVersion)
        );
    }

    #[test]
    fn unknown_request_is_rejected_by_server() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut pipe = connect_pipe(&control.endpoint).unwrap();
        let mut body = protocol::encode_request(&Envelope::current(12, Request::Ping)).unwrap();
        body[2] = 99;
        pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        protocol::write_frame(&mut pipe, &body).unwrap();
        assert_eq!(
            read_raw_response(&mut pipe, 12),
            Response::Error(ErrorCode::InvalidMessage)
        );
    }

    #[test]
    fn connection_request_limit_is_enforced() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut session = Session::connect(&control.endpoint).unwrap();
        for request_id in 10..17 {
            assert_eq!(
                session.exchange_with_id(request_id, Request::Ping).unwrap(),
                Response::Pong
            );
        }
        assert_eq!(
            session.exchange_with_id(17, Request::Ping).unwrap(),
            Response::Error(ErrorCode::RequestLimit)
        );
    }

    #[test]
    fn executable_mismatch_allows_status() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut pipe = mismatched_version_session(&control);
        send_raw_request(&mut pipe, Envelope::current(21, Request::GetStatus));
        assert!(matches!(
            read_raw_response(&mut pipe, 21),
            Response::Status(_)
        ));
    }

    #[test]
    fn executable_mismatch_rejects_shutdown() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut pipe = mismatched_version_session(&control);
        send_raw_request(&mut pipe, Envelope::current(22, Request::Shutdown));
        assert_eq!(
            read_raw_response(&mut pipe, 22),
            Response::Error(ErrorCode::ExecutableVersionMismatch)
        );
    }

    #[test]
    fn shutdown_is_idempotent() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let _guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        let mut session = Session::connect(&control.endpoint).unwrap();
        assert_eq!(
            session.exchange(Request::Shutdown).unwrap(),
            Response::Shutdown {
                already_requested: false
            }
        );
        assert_eq!(
            session.exchange(Request::Shutdown).unwrap(),
            Response::Shutdown {
                already_requested: true
            }
        );
    }

    #[test]
    fn shutdown_cleans_endpoint() {
        let (directory, suffix, control) = fixture();
        let child = spawn_helper(directory.path(), &suffix);
        let mut guard = ChildGuard::new(child, control.clone());
        control.connect_or_start_with(|| Ok(())).unwrap();
        assert!(!control.shutdown().unwrap());
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        while guard.child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(RETRY_INTERVAL);
        }
        assert!(guard.child.try_wait().unwrap().is_some());
        assert!(!control.endpoint.secret_path.exists());
        assert!(matches!(control.ping(), Err(ControlError::Unavailable)));
    }

    #[test]
    fn actual_mutex_descriptor_is_current_user_only() {
        let (directory, suffix, control) = fixture();
        let server = EngineServer::for_test(directory.path(), &suffix)
            .unwrap()
            .unwrap();
        let descriptor = security_for_handle(server._singleton._handle.0);
        assert_current_user_only(descriptor.0, &control.endpoint.sid);
    }

    #[test]
    fn actual_launch_mutex_descriptor_is_current_user_only() {
        let (_directory, _suffix, control) = fixture();
        let security = SecurityDescriptor::for_sid(&control.endpoint.sid).unwrap();
        let launch = LaunchLock::acquire(
            &control.endpoint.launch_mutex_name,
            &security,
            Instant::now() + IO_TIMEOUT,
        )
        .unwrap();
        let descriptor = security_for_handle(launch.handle.0);
        assert_current_user_only(descriptor.0, &control.endpoint.sid);
    }

    #[test]
    fn actual_pipe_descriptor_is_current_user_only() {
        let (directory, suffix, control) = fixture();
        let server = EngineServer::for_test(directory.path(), &suffix)
            .unwrap()
            .unwrap();
        let descriptor = security_for_handle(server.first_pipe.handle.0);
        assert_current_user_only(descriptor.0, &control.endpoint.sid);
    }

    #[test]
    fn actual_secret_file_descriptor_is_current_user_only() {
        let (directory, suffix, control) = fixture();
        let server = EngineServer::for_test(directory.path(), &suffix)
            .unwrap()
            .unwrap();
        let descriptor = security_for_path(&server.endpoint.secret_path);
        assert_current_user_only(descriptor.0, &control.endpoint.sid);
    }

    #[test]
    fn actual_pipe_is_kernel_server_endpoint() {
        let (directory, suffix, control) = fixture();
        let server = EngineServer::for_test(directory.path(), &suffix)
            .unwrap()
            .unwrap();
        let mut flags = 0;
        assert_ne!(
            unsafe {
                GetNamedPipeInfo(
                    server.first_pipe.handle.0,
                    &mut flags,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            },
            0
        );
        assert_eq!(flags & PIPE_SERVER_END, PIPE_SERVER_END);
        assert!(control.endpoint.secret_path.starts_with(directory.path()));
    }

    fn mismatched_version_session(control: &EngineControl) -> Pipe {
        let auth_secret: [u8; AUTH_SECRET_BYTES] = fs::read(&control.endpoint.secret_path)
            .unwrap()
            .try_into()
            .unwrap();
        let mut pipe = connect_pipe(&control.endpoint).unwrap();
        send_raw_request(
            &mut pipe,
            Envelope::current(
                20,
                Request::Hello {
                    auth_secret,
                    executable_version: "different-version".to_string(),
                },
            ),
        );
        assert!(matches!(
            read_raw_response(&mut pipe, 20),
            Response::Hello { .. }
        ));
        pipe
    }

    fn send_raw_request(pipe: &mut Pipe, request: Envelope<Request>) {
        let body = protocol::encode_request(&request).unwrap();
        pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        protocol::write_frame(pipe, &body).unwrap();
    }

    fn read_raw_response(pipe: &mut Pipe, request_id: u64) -> Response {
        pipe.set_deadline(Instant::now() + IO_TIMEOUT);
        let body = protocol::read_frame(pipe).unwrap();
        protocol::decode_response(&body, request_id)
            .unwrap()
            .message
    }

    struct TestSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl Drop for TestSecurityDescriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }

    fn security_for_handle(handle: HANDLE) -> TestSecurityDescriptor {
        let mut descriptor = null_mut();
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0);
        TestSecurityDescriptor(descriptor)
    }

    fn security_for_path(path: &Path) -> TestSecurityDescriptor {
        let mut descriptor = null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide(path.as_os_str()).as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0);
        TestSecurityDescriptor(descriptor)
    }

    fn assert_current_user_only(descriptor: PSECURITY_DESCRIPTOR, sid: &str) {
        let mut control = 0;
        let mut revision = 0;
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
            0
        );
        assert_eq!(control & SE_DACL_PROTECTED, SE_DACL_PROTECTED);

        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
            },
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null());

        let mut information = ACL_SIZE_INFORMATION::default();
        assert_ne!(
            unsafe {
                GetAclInformation(
                    acl,
                    (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
                    size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                )
            },
            0
        );
        assert_eq!(information.AceCount, 1);

        let mut ace = null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(u32::from(ace.Header.AceType), ACCESS_ALLOWED_ACE_TYPE);

        let mut expected_sid = null_mut();
        assert_ne!(
            unsafe { ConvertStringSidToSidW(wide(sid).as_ptr(), &mut expected_sid) },
            0
        );
        assert_ne!(
            unsafe {
                EqualSid(
                    (&ace.SidStart as *const u32).cast_mut().cast(),
                    expected_sid,
                )
            },
            0
        );
        unsafe {
            LocalFree(expected_sid);
        }
    }
}
