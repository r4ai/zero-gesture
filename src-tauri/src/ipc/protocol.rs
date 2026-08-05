use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;

use crate::config::MAX_CONFIG_BYTES;

pub const PROTOCOL_VERSION: u16 = 3;
pub const MAX_ENCODED_FRAME: usize = 1024 * 1024 - 1;
#[cfg(test)]
pub const MAX_BODY: usize = MAX_ENCODED_FRAME - size_of::<u32>();
pub const AUTH_SECRET_BYTES: usize = 32;
pub const MAX_VERSION_BYTES: usize = 64;
pub const MAX_REQUESTS_PER_CONNECTION: usize = 1024;
pub const CAPABILITY_PING: u64 = 1 << 0;
pub const CAPABILITY_STATUS: u64 = 1 << 1;
pub const CAPABILITY_SHUTDOWN: u64 = 1 << 2;
pub const CAPABILITY_CONFIG_READ: u64 = 1 << 3;
pub const CAPABILITY_CONFIG_TRANSACTION: u64 = 1 << 4;
pub const CAPABILITY_WINDOW_CAPTURE: u64 = 1 << 5;
pub const CAPABILITIES: u64 = CAPABILITY_PING
    | CAPABILITY_STATUS
    | CAPABILITY_SHUTDOWN
    | CAPABILITY_CONFIG_READ
    | CAPABILITY_CONFIG_TRANSACTION
    | CAPABILITY_WINDOW_CAPTURE;
const MAX_CAPTURE_TEXT_BYTES: usize = 4 * 1024;

const HEADER_BYTES: usize = size_of::<u16>() + size_of::<u8>() + size_of::<u8>() + size_of::<u64>();

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessRole {
    Engine,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EngineStatus {
    pub role: ProcessRole,
    pub webview_count: u16,
    pub process_id: u32,
    pub uptime_ms: u64,
    pub thread_count: u32,
    pub handle_count: u32,
    pub working_set_bytes: u64,
    pub config_available: bool,
    pub config_revision: u64,
    pub config_generation: u64,
    pub config_candidate_prepared: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Hello {
        auth_secret: [u8; AUTH_SECRET_BYTES],
        executable_version: String,
    },
    Ping,
    GetStatus,
    Shutdown,
    GetConfig,
    PrepareConfig {
        expected_revision: u64,
        config_bytes: Vec<u8>,
    },
    CommitConfig {
        token: u64,
        base_revision: u64,
        base_generation: u64,
    },
    SetEnabled {
        expected_revision: u64,
        enabled: bool,
    },
    BeginWindowCapture {
        capture_id: u64,
    },
    PollWindowCapture {
        capture_id: u64,
        epoch: u64,
    },
    CancelWindowCapture {
        capture_id: u64,
        epoch: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    WrongVersion,
    ExecutableVersionMismatch,
    AuthenticationFailed,
    HelloRequired,
    DuplicateRequestId,
    RequestLimit,
    InvalidMessage,
    ConfigPayloadTooLarge,
    ConfigBusy,
    ConfigRevisionConflict,
    ConfigValidationFailed,
    ConfigTokenMismatch,
    NoPreparedConfig,
    ConfigGenerationExhausted,
    ConfigPersistenceFailed,
    CaptureStale,
    CaptureUnavailable,
    CaptureBackendFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowCaptureInfo {
    pub process_name: Option<String>,
    pub window_class: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowCaptureResult {
    Pending,
    Captured(WindowCaptureInfo),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Response {
    Hello {
        engine_version: String,
        config_schema_version: u16,
        capabilities: u64,
    },
    Pong,
    Status(EngineStatus),
    Shutdown {
        already_requested: bool,
    },
    Config {
        revision: u64,
        generation: u64,
        config_bytes: Option<Vec<u8>>,
    },
    Prepared {
        token: u64,
        base_revision: u64,
        base_generation: u64,
    },
    Applied {
        revision: u64,
        generation: u64,
        durability_warning: bool,
    },
    WindowCaptureStarted {
        capture_id: u64,
        epoch: u64,
    },
    WindowCapture {
        capture_id: u64,
        epoch: u64,
        result: WindowCaptureResult,
    },
    WindowCaptureCancelled {
        capture_id: u64,
        epoch: u64,
    },
    Error(ErrorCode),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope<T> {
    pub protocol_version: u16,
    pub request_id: u64,
    pub message: T,
}

impl<T> Envelope<T> {
    pub fn current(request_id: u64, message: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            message,
        }
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    ZeroLength,
    FrameTooLarge(u32),
    Truncated,
    WrongVersion(u16),
    InvalidMessage,
    InvalidUtf8,
    VersionTooLong,
    MismatchedResponse { expected: u64, actual: u64 },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "IPC I/O failed: {error}"),
            Self::ZeroLength => formatter.write_str("IPC frame body is empty"),
            Self::FrameTooLarge(length) => {
                write!(formatter, "IPC frame length {length} exceeds the limit")
            }
            Self::Truncated => formatter.write_str("IPC frame is truncated"),
            Self::WrongVersion(version) => {
                write!(formatter, "unsupported IPC protocol version {version}")
            }
            Self::InvalidMessage => formatter.write_str("invalid IPC message"),
            Self::InvalidUtf8 => formatter.write_str("invalid IPC UTF-8"),
            Self::VersionTooLong => formatter.write_str("executable version exceeds the limit"),
            Self::MismatchedResponse { expected, actual } => write!(
                formatter,
                "IPC response id mismatch: expected {expected}, received {actual}"
            ),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, ProtocolError> {
    let mut prefix = [0_u8; size_of::<u32>()];
    read_exact(reader, &mut prefix)?;
    let body_length = u32::from_le_bytes(prefix);
    if body_length == 0 {
        return Err(ProtocolError::ZeroLength);
    }
    let encoded_length = (body_length as usize)
        .checked_add(prefix.len())
        .ok_or(ProtocolError::FrameTooLarge(body_length))?;
    if encoded_length > MAX_ENCODED_FRAME {
        return Err(ProtocolError::FrameTooLarge(body_length));
    }
    let mut body = vec![0_u8; body_length as usize];
    read_exact(reader, &mut body)?;
    Ok(body)
}

pub fn write_frame(writer: &mut impl Write, body: &[u8]) -> Result<(), ProtocolError> {
    if body.is_empty() {
        return Err(ProtocolError::ZeroLength);
    }
    let encoded_length = body
        .len()
        .checked_add(size_of::<u32>())
        .ok_or(ProtocolError::FrameTooLarge(u32::MAX))?;
    if encoded_length > MAX_ENCODED_FRAME {
        return Err(ProtocolError::FrameTooLarge(
            u32::try_from(body.len()).unwrap_or(u32::MAX),
        ));
    }
    writer.write_all(&(body.len() as u32).to_le_bytes())?;
    writer.write_all(body)?;
    Ok(())
}

fn read_exact(reader: &mut impl Read, bytes: &mut [u8]) -> Result<(), ProtocolError> {
    reader.read_exact(bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            ProtocolError::Truncated
        } else {
            ProtocolError::Io(error)
        }
    })
}

pub fn encode_request(envelope: &Envelope<Request>) -> Result<Vec<u8>, ProtocolError> {
    let (tag, payload) = match &envelope.message {
        Request::Hello {
            auth_secret,
            executable_version,
        } => {
            let mut payload = Vec::with_capacity(AUTH_SECRET_BYTES + 1 + executable_version.len());
            payload.extend_from_slice(auth_secret);
            put_string(&mut payload, executable_version)?;
            (1, payload)
        }
        Request::Ping => (2, Vec::new()),
        Request::GetStatus => (3, Vec::new()),
        Request::Shutdown => (4, Vec::new()),
        Request::GetConfig => (5, Vec::new()),
        Request::PrepareConfig {
            expected_revision,
            config_bytes,
        } => {
            if config_bytes.len() > MAX_CONFIG_BYTES {
                return Err(ProtocolError::FrameTooLarge(
                    u32::try_from(config_bytes.len()).unwrap_or(u32::MAX),
                ));
            }
            let mut payload = Vec::with_capacity(12 + config_bytes.len());
            payload.extend_from_slice(&expected_revision.to_le_bytes());
            payload.extend_from_slice(&(config_bytes.len() as u32).to_le_bytes());
            payload.extend_from_slice(config_bytes);
            (6, payload)
        }
        Request::CommitConfig {
            token,
            base_revision,
            base_generation,
        } => {
            let mut payload = Vec::with_capacity(24);
            payload.extend_from_slice(&token.to_le_bytes());
            payload.extend_from_slice(&base_revision.to_le_bytes());
            payload.extend_from_slice(&base_generation.to_le_bytes());
            (7, payload)
        }
        Request::SetEnabled {
            expected_revision,
            enabled,
        } => {
            let mut payload = Vec::with_capacity(9);
            payload.extend_from_slice(&expected_revision.to_le_bytes());
            payload.push(u8::from(*enabled));
            (8, payload)
        }
        Request::BeginWindowCapture { capture_id } => (9, capture_id.to_le_bytes().to_vec()),
        Request::PollWindowCapture { capture_id, epoch } => {
            let mut payload = Vec::with_capacity(16);
            payload.extend_from_slice(&capture_id.to_le_bytes());
            payload.extend_from_slice(&epoch.to_le_bytes());
            (10, payload)
        }
        Request::CancelWindowCapture { capture_id, epoch } => {
            let mut payload = Vec::with_capacity(16);
            payload.extend_from_slice(&capture_id.to_le_bytes());
            payload.extend_from_slice(&epoch.to_le_bytes());
            (11, payload)
        }
    };
    encode_envelope(
        envelope.protocol_version,
        tag,
        envelope.request_id,
        &payload,
    )
}

pub fn decode_request(body: &[u8]) -> Result<Envelope<Request>, ProtocolError> {
    let (protocol_version, tag, request_id, payload) = decode_envelope(body)?;
    let message = match tag {
        1 => {
            if payload.len() < AUTH_SECRET_BYTES + 1 {
                return Err(ProtocolError::InvalidMessage);
            }
            let mut auth_secret = [0_u8; AUTH_SECRET_BYTES];
            auth_secret.copy_from_slice(&payload[..AUTH_SECRET_BYTES]);
            let mut cursor = Cursor::new(&payload[AUTH_SECRET_BYTES..]);
            let executable_version = cursor.string()?;
            cursor.finish()?;
            Request::Hello {
                auth_secret,
                executable_version,
            }
        }
        2 => empty_payload(payload, Request::Ping)?,
        3 => empty_payload(payload, Request::GetStatus)?,
        4 => empty_payload(payload, Request::Shutdown)?,
        5 => empty_payload(payload, Request::GetConfig)?,
        6 => {
            let mut cursor = Cursor::new(payload);
            let expected_revision = cursor.u64()?;
            let config_bytes = cursor.bounded_bytes(MAX_CONFIG_BYTES)?;
            cursor.finish()?;
            Request::PrepareConfig {
                expected_revision,
                config_bytes,
            }
        }
        7 => {
            let mut cursor = Cursor::new(payload);
            let request = Request::CommitConfig {
                token: cursor.u64()?,
                base_revision: cursor.u64()?,
                base_generation: cursor.u64()?,
            };
            cursor.finish()?;
            request
        }
        8 => {
            let mut cursor = Cursor::new(payload);
            let request = Request::SetEnabled {
                expected_revision: cursor.u64()?,
                enabled: cursor.boolean()?,
            };
            cursor.finish()?;
            request
        }
        9 => {
            let mut cursor = Cursor::new(payload);
            let request = Request::BeginWindowCapture {
                capture_id: cursor.u64()?,
            };
            cursor.finish()?;
            request
        }
        10 => {
            let mut cursor = Cursor::new(payload);
            let request = Request::PollWindowCapture {
                capture_id: cursor.u64()?,
                epoch: cursor.u64()?,
            };
            cursor.finish()?;
            request
        }
        11 => {
            let mut cursor = Cursor::new(payload);
            let request = Request::CancelWindowCapture {
                capture_id: cursor.u64()?,
                epoch: cursor.u64()?,
            };
            cursor.finish()?;
            request
        }
        _ => return Err(ProtocolError::InvalidMessage),
    };
    Ok(Envelope {
        protocol_version,
        request_id,
        message,
    })
}

pub fn encode_response(envelope: &Envelope<Response>) -> Result<Vec<u8>, ProtocolError> {
    let (tag, payload) = match &envelope.message {
        Response::Hello {
            engine_version,
            config_schema_version,
            capabilities,
        } => {
            let mut payload = Vec::with_capacity(1 + engine_version.len() + 10);
            put_string(&mut payload, engine_version)?;
            payload.extend_from_slice(&config_schema_version.to_le_bytes());
            payload.extend_from_slice(&capabilities.to_le_bytes());
            (129, payload)
        }
        Response::Pong => (130, Vec::new()),
        Response::Status(status) => {
            let mut payload = Vec::with_capacity(31);
            payload.push(match status.role {
                ProcessRole::Engine => 1,
            });
            payload.extend_from_slice(&status.webview_count.to_le_bytes());
            payload.extend_from_slice(&status.process_id.to_le_bytes());
            payload.extend_from_slice(&status.uptime_ms.to_le_bytes());
            payload.extend_from_slice(&status.thread_count.to_le_bytes());
            payload.extend_from_slice(&status.handle_count.to_le_bytes());
            payload.extend_from_slice(&status.working_set_bytes.to_le_bytes());
            payload.push(u8::from(status.config_available));
            payload.extend_from_slice(&status.config_revision.to_le_bytes());
            payload.extend_from_slice(&status.config_generation.to_le_bytes());
            payload.push(u8::from(status.config_candidate_prepared));
            (131, payload)
        }
        Response::Shutdown { already_requested } => (132, vec![u8::from(*already_requested)]),
        Response::Config {
            revision,
            generation,
            config_bytes,
        } => {
            if config_bytes
                .as_ref()
                .is_some_and(|bytes| bytes.len() > MAX_CONFIG_BYTES)
            {
                return Err(ProtocolError::FrameTooLarge(
                    u32::try_from(config_bytes.as_ref().map_or(0, Vec::len)).unwrap_or(u32::MAX),
                ));
            }
            let mut payload = Vec::with_capacity(21 + config_bytes.as_ref().map_or(0, Vec::len));
            payload.extend_from_slice(&revision.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.push(u8::from(config_bytes.is_some()));
            if let Some(config_bytes) = config_bytes {
                payload.extend_from_slice(&(config_bytes.len() as u32).to_le_bytes());
                payload.extend_from_slice(config_bytes);
            }
            (133, payload)
        }
        Response::Prepared {
            token,
            base_revision,
            base_generation,
        } => {
            let mut payload = Vec::with_capacity(24);
            payload.extend_from_slice(&token.to_le_bytes());
            payload.extend_from_slice(&base_revision.to_le_bytes());
            payload.extend_from_slice(&base_generation.to_le_bytes());
            (134, payload)
        }
        Response::Applied {
            revision,
            generation,
            durability_warning,
        } => {
            let mut payload = Vec::with_capacity(17);
            payload.extend_from_slice(&revision.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.push(u8::from(*durability_warning));
            (135, payload)
        }
        Response::WindowCaptureStarted { capture_id, epoch } => {
            let mut payload = Vec::with_capacity(16);
            payload.extend_from_slice(&capture_id.to_le_bytes());
            payload.extend_from_slice(&epoch.to_le_bytes());
            (136, payload)
        }
        Response::WindowCapture {
            capture_id,
            epoch,
            result,
        } => {
            let mut payload = Vec::new();
            payload.extend_from_slice(&capture_id.to_le_bytes());
            payload.extend_from_slice(&epoch.to_le_bytes());
            match result {
                WindowCaptureResult::Pending => payload.push(0),
                WindowCaptureResult::Captured(info) => {
                    payload.push(1);
                    put_optional_capture_text(&mut payload, info.process_name.as_deref())?;
                    put_optional_capture_text(&mut payload, info.window_class.as_deref())?;
                    put_optional_capture_text(&mut payload, info.title.as_deref())?;
                }
            }
            (137, payload)
        }
        Response::WindowCaptureCancelled { capture_id, epoch } => {
            let mut payload = Vec::with_capacity(16);
            payload.extend_from_slice(&capture_id.to_le_bytes());
            payload.extend_from_slice(&epoch.to_le_bytes());
            (138, payload)
        }
        Response::Error(code) => (255, vec![error_code_byte(*code)]),
    };
    encode_envelope(
        envelope.protocol_version,
        tag,
        envelope.request_id,
        &payload,
    )
}

pub fn decode_response(
    body: &[u8],
    expected_request_id: u64,
) -> Result<Envelope<Response>, ProtocolError> {
    let (protocol_version, tag, request_id, payload) = decode_envelope(body)?;
    if request_id != expected_request_id {
        return Err(ProtocolError::MismatchedResponse {
            expected: expected_request_id,
            actual: request_id,
        });
    }
    let message = match tag {
        129 => {
            let mut cursor = Cursor::new(payload);
            let engine_version = cursor.string()?;
            let config_schema_version = cursor.u16()?;
            let capabilities = cursor.u64()?;
            cursor.finish()?;
            Response::Hello {
                engine_version,
                config_schema_version,
                capabilities,
            }
        }
        130 => empty_payload(payload, Response::Pong)?,
        131 => {
            let mut cursor = Cursor::new(payload);
            let role = match cursor.u8()? {
                1 => ProcessRole::Engine,
                _ => return Err(ProtocolError::InvalidMessage),
            };
            let status = EngineStatus {
                role,
                webview_count: cursor.u16()?,
                process_id: cursor.u32()?,
                uptime_ms: cursor.u64()?,
                thread_count: cursor.u32()?,
                handle_count: cursor.u32()?,
                working_set_bytes: cursor.u64()?,
                config_available: cursor.boolean()?,
                config_revision: cursor.u64()?,
                config_generation: cursor.u64()?,
                config_candidate_prepared: cursor.boolean()?,
            };
            cursor.finish()?;
            Response::Status(status)
        }
        132 => {
            if payload.len() != 1 || payload[0] > 1 {
                return Err(ProtocolError::InvalidMessage);
            }
            Response::Shutdown {
                already_requested: payload[0] == 1,
            }
        }
        133 => {
            let mut cursor = Cursor::new(payload);
            let revision = cursor.u64()?;
            let generation = cursor.u64()?;
            let config_bytes = cursor
                .boolean()?
                .then(|| cursor.bounded_bytes(MAX_CONFIG_BYTES))
                .transpose()?;
            cursor.finish()?;
            Response::Config {
                revision,
                generation,
                config_bytes,
            }
        }
        134 => {
            let mut cursor = Cursor::new(payload);
            let response = Response::Prepared {
                token: cursor.u64()?,
                base_revision: cursor.u64()?,
                base_generation: cursor.u64()?,
            };
            cursor.finish()?;
            response
        }
        135 => {
            let mut cursor = Cursor::new(payload);
            let response = Response::Applied {
                revision: cursor.u64()?,
                generation: cursor.u64()?,
                durability_warning: cursor.boolean()?,
            };
            cursor.finish()?;
            response
        }
        136 => {
            let mut cursor = Cursor::new(payload);
            let response = Response::WindowCaptureStarted {
                capture_id: cursor.u64()?,
                epoch: cursor.u64()?,
            };
            cursor.finish()?;
            response
        }
        137 => {
            let mut cursor = Cursor::new(payload);
            let capture_id = cursor.u64()?;
            let epoch = cursor.u64()?;
            let result = match cursor.u8()? {
                0 => WindowCaptureResult::Pending,
                1 => WindowCaptureResult::Captured(WindowCaptureInfo {
                    process_name: cursor.optional_capture_text()?,
                    window_class: cursor.optional_capture_text()?,
                    title: cursor.optional_capture_text()?,
                }),
                _ => return Err(ProtocolError::InvalidMessage),
            };
            cursor.finish()?;
            Response::WindowCapture {
                capture_id,
                epoch,
                result,
            }
        }
        138 => {
            let mut cursor = Cursor::new(payload);
            let response = Response::WindowCaptureCancelled {
                capture_id: cursor.u64()?,
                epoch: cursor.u64()?,
            };
            cursor.finish()?;
            response
        }
        255 => {
            if payload.len() != 1 {
                return Err(ProtocolError::InvalidMessage);
            }
            Response::Error(byte_error_code(payload[0])?)
        }
        _ => return Err(ProtocolError::InvalidMessage),
    };
    Ok(Envelope {
        protocol_version,
        request_id,
        message,
    })
}

fn encode_envelope(
    protocol_version: u16,
    tag: u8,
    request_id: u64,
    payload: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let body_length = HEADER_BYTES
        .checked_add(payload.len())
        .ok_or(ProtocolError::FrameTooLarge(u32::MAX))?;
    if body_length + size_of::<u32>() > MAX_ENCODED_FRAME {
        return Err(ProtocolError::FrameTooLarge(
            u32::try_from(body_length).unwrap_or(u32::MAX),
        ));
    }
    let mut body = Vec::with_capacity(body_length);
    body.extend_from_slice(&protocol_version.to_le_bytes());
    body.push(tag);
    body.push(0);
    body.extend_from_slice(&request_id.to_le_bytes());
    body.extend_from_slice(payload);
    Ok(body)
}

fn decode_envelope(body: &[u8]) -> Result<(u16, u8, u64, &[u8]), ProtocolError> {
    if body.len() < HEADER_BYTES {
        return Err(ProtocolError::InvalidMessage);
    }
    let protocol_version = u16::from_le_bytes([body[0], body[1]]);
    if protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::WrongVersion(protocol_version));
    }
    if body[3] != 0 {
        return Err(ProtocolError::InvalidMessage);
    }
    let request_id = u64::from_le_bytes(
        body[4..12]
            .try_into()
            .map_err(|_| ProtocolError::InvalidMessage)?,
    );
    if request_id == 0 {
        return Err(ProtocolError::InvalidMessage);
    }
    Ok((protocol_version, body[2], request_id, &body[HEADER_BYTES..]))
}

fn put_string(output: &mut Vec<u8>, value: &str) -> Result<(), ProtocolError> {
    if value.len() > MAX_VERSION_BYTES {
        return Err(ProtocolError::VersionTooLong);
    }
    output.push(value.len() as u8);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_optional_capture_text(
    output: &mut Vec<u8>,
    value: Option<&str>,
) -> Result<(), ProtocolError> {
    let Some(value) = value else {
        output.extend_from_slice(&u16::MAX.to_le_bytes());
        return Ok(());
    };
    if value.len() > MAX_CAPTURE_TEXT_BYTES || value.len() >= u16::MAX as usize {
        return Err(ProtocolError::InvalidMessage);
    }
    output.extend_from_slice(&(value.len() as u16).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn empty_payload<T>(payload: &[u8], value: T) -> Result<T, ProtocolError> {
    if payload.is_empty() {
        Ok(value)
    } else {
        Err(ProtocolError::InvalidMessage)
    }
}

fn error_code_byte(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::WrongVersion => 1,
        ErrorCode::ExecutableVersionMismatch => 2,
        ErrorCode::AuthenticationFailed => 3,
        ErrorCode::HelloRequired => 4,
        ErrorCode::DuplicateRequestId => 5,
        ErrorCode::RequestLimit => 6,
        ErrorCode::InvalidMessage => 7,
        ErrorCode::ConfigPayloadTooLarge => 8,
        ErrorCode::ConfigBusy => 9,
        ErrorCode::ConfigRevisionConflict => 10,
        ErrorCode::ConfigValidationFailed => 11,
        ErrorCode::ConfigTokenMismatch => 12,
        ErrorCode::NoPreparedConfig => 13,
        ErrorCode::ConfigGenerationExhausted => 14,
        ErrorCode::ConfigPersistenceFailed => 15,
        ErrorCode::CaptureStale => 16,
        ErrorCode::CaptureUnavailable => 17,
        ErrorCode::CaptureBackendFailed => 18,
    }
}

fn byte_error_code(byte: u8) -> Result<ErrorCode, ProtocolError> {
    match byte {
        1 => Ok(ErrorCode::WrongVersion),
        2 => Ok(ErrorCode::ExecutableVersionMismatch),
        3 => Ok(ErrorCode::AuthenticationFailed),
        4 => Ok(ErrorCode::HelloRequired),
        5 => Ok(ErrorCode::DuplicateRequestId),
        6 => Ok(ErrorCode::RequestLimit),
        7 => Ok(ErrorCode::InvalidMessage),
        8 => Ok(ErrorCode::ConfigPayloadTooLarge),
        9 => Ok(ErrorCode::ConfigBusy),
        10 => Ok(ErrorCode::ConfigRevisionConflict),
        11 => Ok(ErrorCode::ConfigValidationFailed),
        12 => Ok(ErrorCode::ConfigTokenMismatch),
        13 => Ok(ErrorCode::NoPreparedConfig),
        14 => Ok(ErrorCode::ConfigGenerationExhausted),
        15 => Ok(ErrorCode::ConfigPersistenceFailed),
        16 => Ok(ErrorCode::CaptureStale),
        17 => Ok(ErrorCode::CaptureUnavailable),
        18 => Ok(ErrorCode::CaptureBackendFailed),
        _ => Err(ProtocolError::InvalidMessage),
    }
}

pub(super) fn request_id_from_body(body: &[u8]) -> Option<u64> {
    body.get(4..12)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .filter(|request_id| *request_id != 0)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(ProtocolError::InvalidMessage)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::InvalidMessage)?;
        self.position = end;
        bytes.try_into().map_err(|_| ProtocolError::InvalidMessage)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn boolean(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::InvalidMessage),
        }
    }

    fn bounded_bytes(&mut self, maximum: usize) -> Result<Vec<u8>, ProtocolError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(ProtocolError::InvalidMessage);
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::InvalidMessage)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::InvalidMessage)?;
        self.position = end;
        Ok(bytes.to_vec())
    }

    fn string(&mut self) -> Result<String, ProtocolError> {
        let length = self.u8()? as usize;
        if length > MAX_VERSION_BYTES {
            return Err(ProtocolError::VersionTooLong);
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::InvalidMessage)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::InvalidMessage)?;
        self.position = end;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ProtocolError::InvalidUtf8)
    }

    fn optional_capture_text(&mut self) -> Result<Option<String>, ProtocolError> {
        let length = self.u16()? as usize;
        if length == u16::MAX as usize {
            return Ok(None);
        }
        if length > MAX_CAPTURE_TEXT_BYTES {
            return Err(ProtocolError::InvalidMessage);
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::InvalidMessage)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::InvalidMessage)?;
        self.position = end;
        std::str::from_utf8(bytes)
            .map(|value| Some(value.to_owned()))
            .map_err(|_| ProtocolError::InvalidUtf8)
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(ProtocolError::InvalidMessage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor as IoCursor;

    fn secret() -> [u8; AUTH_SECRET_BYTES] {
        [0x5a; AUTH_SECRET_BYTES]
    }

    #[test]
    fn codec_roundtrips_hello() {
        let request = Envelope::current(
            1,
            Request::Hello {
                auth_secret: secret(),
                executable_version: "0.1.0".to_string(),
            },
        );
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );

        let response = Envelope::current(
            1,
            Response::Hello {
                engine_version: "0.1.0".to_string(),
                config_schema_version: 2,
                capabilities: CAPABILITIES,
            },
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 1).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_ping() {
        let request = Envelope::current(2, Request::Ping);
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
        let response = Envelope::current(2, Response::Pong);
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 2).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_status() {
        let response = Envelope::current(
            3,
            Response::Status(EngineStatus {
                role: ProcessRole::Engine,
                webview_count: 0,
                process_id: 7,
                uptime_ms: 11,
                thread_count: 2,
                handle_count: 9,
                working_set_bytes: 4096,
                config_available: true,
                config_revision: 1,
                config_generation: 1,
                config_candidate_prepared: false,
            }),
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 3).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_shutdown() {
        let request = Envelope::current(4, Request::Shutdown);
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
        let response = Envelope::current(
            4,
            Response::Shutdown {
                already_requested: true,
            },
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 4).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_prepare_and_prepared() {
        let prepare = Envelope::current(
            5,
            Request::PrepareConfig {
                expected_revision: 7,
                config_bytes: br#"{"schema_version":2}"#.to_vec(),
            },
        );
        assert_eq!(
            decode_request(&encode_request(&prepare).unwrap()).unwrap(),
            prepare
        );
        let prepared = Envelope::current(
            5,
            Response::Prepared {
                token: 9,
                base_revision: 7,
                base_generation: 4,
            },
        );
        assert_eq!(
            decode_response(&encode_response(&prepared).unwrap(), 5).unwrap(),
            prepared
        );
    }

    #[test]
    fn codec_roundtrips_commit_and_applied() {
        let commit = Envelope::current(
            6,
            Request::CommitConfig {
                token: 9,
                base_revision: 7,
                base_generation: 4,
            },
        );
        assert_eq!(
            decode_request(&encode_request(&commit).unwrap()).unwrap(),
            commit
        );
        let applied = Envelope::current(
            6,
            Response::Applied {
                revision: 8,
                generation: 5,
                durability_warning: true,
            },
        );
        assert_eq!(
            decode_response(&encode_response(&applied).unwrap(), 6).unwrap(),
            applied
        );
    }

    #[test]
    fn codec_roundtrips_set_enabled() {
        let request = Envelope::current(
            7,
            Request::SetEnabled {
                expected_revision: 4,
                enabled: true,
            },
        );
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn codec_roundtrips_window_capture_identity_and_result() {
        let begin = Envelope::current(30, Request::BeginWindowCapture { capture_id: 77 });
        assert_eq!(
            decode_request(&encode_request(&begin).unwrap()).unwrap(),
            begin
        );

        let poll = Envelope::current(
            31,
            Request::PollWindowCapture {
                capture_id: 77,
                epoch: 9,
            },
        );
        assert_eq!(
            decode_request(&encode_request(&poll).unwrap()).unwrap(),
            poll
        );

        let response = Envelope::current(
            31,
            Response::WindowCapture {
                capture_id: 77,
                epoch: 9,
                result: WindowCaptureResult::Captured(WindowCaptureInfo {
                    process_name: Some("explorer.exe".to_string()),
                    window_class: Some("CabinetWClass".to_string()),
                    title: Some("Downloads".to_string()),
                }),
            },
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 31).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_available_config_observation() {
        let request = Envelope::current(7, Request::GetConfig);
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
        let response = Envelope::current(
            7,
            Response::Config {
                revision: 3,
                generation: 2,
                config_bytes: Some(br#"{"schema_version":2}"#.to_vec()),
            },
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 7).unwrap(),
            response
        );
    }

    #[test]
    fn codec_roundtrips_unavailable_config_observation() {
        let response = Envelope::current(
            8,
            Response::Config {
                revision: 0,
                generation: 0,
                config_bytes: None,
            },
        );
        assert_eq!(
            decode_response(&encode_response(&response).unwrap(), 8).unwrap(),
            response
        );
    }

    #[test]
    fn config_payload_limit_is_enforced_by_the_closed_codec() {
        let oversized = Envelope::current(
            1,
            Request::PrepareConfig {
                expected_revision: 1,
                config_bytes: vec![0; MAX_CONFIG_BYTES + 1],
            },
        );
        assert!(matches!(
            encode_request(&oversized),
            Err(ProtocolError::FrameTooLarge(_))
        ));

        let mut payload = Vec::from(1_u64.to_le_bytes());
        payload.extend_from_slice(&u32::try_from(MAX_CONFIG_BYTES + 1).unwrap().to_le_bytes());
        let body = encode_envelope(PROTOCOL_VERSION, 6, 1, &payload).unwrap();
        assert!(matches!(
            decode_request(&body),
            Err(ProtocolError::InvalidMessage)
        ));
    }

    #[test]
    fn zero_length_frame_is_rejected() {
        let mut bytes = IoCursor::new(0_u32.to_le_bytes());
        assert!(matches!(
            read_frame(&mut bytes),
            Err(ProtocolError::ZeroLength)
        ));
    }

    #[test]
    fn oversized_frame_is_rejected_before_body_read() {
        let oversized = u32::try_from(MAX_BODY + 1).unwrap();
        let mut bytes = IoCursor::new(oversized.to_le_bytes());
        assert!(matches!(
            read_frame(&mut bytes),
            Err(ProtocolError::FrameTooLarge(length)) if length == oversized
        ));
    }

    #[test]
    fn maximum_frame_body_is_accepted() {
        let body = vec![0_u8; MAX_BODY];
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &body).unwrap();
        assert_eq!(read_frame(&mut IoCursor::new(encoded)).unwrap(), body);
    }

    #[test]
    fn truncated_prefix_is_rejected() {
        let mut bytes = IoCursor::new([1_u8, 0, 0]);
        assert!(matches!(
            read_frame(&mut bytes),
            Err(ProtocolError::Truncated)
        ));
    }

    #[test]
    fn truncated_body_is_rejected() {
        let mut bytes = Vec::from(5_u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2]);
        assert!(matches!(
            read_frame(&mut IoCursor::new(bytes)),
            Err(ProtocolError::Truncated)
        ));
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut body = encode_request(&Envelope::current(1, Request::Ping)).unwrap();
        body[..2].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        assert!(matches!(
            decode_request(&body),
            Err(ProtocolError::WrongVersion(version)) if version == PROTOCOL_VERSION + 1
        ));
    }

    #[test]
    fn unknown_message_is_rejected() {
        let body = encode_envelope(PROTOCOL_VERSION, 99, 1, &[]).unwrap();
        assert!(matches!(
            decode_request(&body),
            Err(ProtocolError::InvalidMessage)
        ));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut payload = Vec::from(secret());
        payload.extend_from_slice(&[1, 0xff]);
        let body = encode_envelope(PROTOCOL_VERSION, 1, 1, &payload).unwrap();
        assert!(matches!(
            decode_request(&body),
            Err(ProtocolError::InvalidUtf8)
        ));
    }

    #[test]
    fn mismatched_response_id_is_rejected() {
        let body = encode_response(&Envelope::current(8, Response::Pong)).unwrap();
        assert!(matches!(
            decode_response(&body, 7),
            Err(ProtocolError::MismatchedResponse {
                expected: 7,
                actual: 8
            })
        ));
    }
}
