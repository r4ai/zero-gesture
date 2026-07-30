use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_ENCODED_FRAME: usize = 1024 * 1024 - 1;
#[cfg(test)]
pub const MAX_BODY: usize = MAX_ENCODED_FRAME - size_of::<u32>();
pub const AUTH_SECRET_BYTES: usize = 32;
pub const MAX_VERSION_BYTES: usize = 64;
pub const MAX_REQUESTS_PER_CONNECTION: usize = 8;
pub const CAPABILITY_PING: u64 = 1 << 0;
pub const CAPABILITY_STATUS: u64 = 1 << 1;
pub const CAPABILITY_SHUTDOWN: u64 = 1 << 2;
pub const CAPABILITIES: u64 = CAPABILITY_PING | CAPABILITY_STATUS | CAPABILITY_SHUTDOWN;

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
            (131, payload)
        }
        Response::Shutdown { already_requested } => (132, vec![u8::from(*already_requested)]),
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
