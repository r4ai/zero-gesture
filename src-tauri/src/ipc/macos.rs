use super::core::{ControlError, ACCEPT_POLL_INTERVAL, IO_TIMEOUT, RETRY_INTERVAL};
use super::protocol::AUTH_SECRET_BYTES;
use log::warn;
use std::ffi::CString;
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

const RUNTIME_DIRECTORY: &str = "dev.r4ai.zero-gesture";
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const MAX_SUFFIX_BYTES: usize = 32;
const MAX_SOCKET_PATH_BYTES: usize = 103;
const QUARANTINE_ATTEMPTS: usize = 4;

#[derive(Clone)]
pub(super) struct Endpoint {
    runtime_dir: PathBuf,
    socket_path: PathBuf,
    singleton_path: PathBuf,
    launch_path: PathBuf,
    secret_path: PathBuf,
    effective_uid: libc::uid_t,
}

impl Endpoint {
    pub(super) fn current_user(_config_dir: &Path, suffix: &str) -> Result<Self, ControlError> {
        if suffix.len() > MAX_SUFFIX_BYTES
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ControlError::Security(
                "invalid internal endpoint suffix".to_string(),
            ));
        }
        let runtime_dir = std::env::temp_dir().join(RUNTIME_DIRECTORY);
        let suffix = if suffix.is_empty() {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let endpoint = Self {
            socket_path: runtime_dir.join(format!("control{suffix}.sock")),
            singleton_path: runtime_dir.join(format!("engine{suffix}.lock")),
            launch_path: runtime_dir.join(format!("launch{suffix}.lock")),
            secret_path: runtime_dir.join(format!("control{suffix}.secret")),
            runtime_dir,
            effective_uid: unsafe { libc::geteuid() },
        };
        use std::os::unix::ffi::OsStrExt;
        if endpoint.socket_path.as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES {
            return Err(ControlError::Security(
                "internal socket path exceeds the macOS limit".to_string(),
            ));
        }
        Ok(endpoint)
    }

    pub(super) fn acquire_launch_lock(
        &self,
        deadline: Instant,
    ) -> Result<LaunchLock, ControlError> {
        ensure_runtime_directory(&self.runtime_dir, self.effective_uid, true)?;
        LaunchLock::acquire(&self.launch_path, self.effective_uid, deadline)
    }

    pub(super) fn prepare_server(
        &self,
    ) -> Result<Option<([u8; AUTH_SECRET_BYTES], ServerTransport)>, ControlError> {
        ensure_runtime_directory(&self.runtime_dir, self.effective_uid, true)?;
        let Some(singleton) = FileLock::try_singleton(&self.singleton_path, self.effective_uid)?
        else {
            return Ok(None);
        };
        remove_stale_socket(&self.socket_path, self.effective_uid)?;
        let secret = generate_secret();
        let secret_file = SecretFile::create(&self.secret_path, self.effective_uid, &secret)?;
        let listener = bind_private_socket(&self.socket_path)?;
        let bound_metadata = fs::symlink_metadata(&self.socket_path)
            .map_err(|error| endpoint_io("inspect bound Unix control socket", error))?;
        let socket = OwnedPath::new(&self.socket_path, &bound_metadata, ObjectKind::Socket);
        let socket_metadata = validate_socket(&self.socket_path, self.effective_uid)?;
        if identity(&socket_metadata) != socket.identity {
            return Err(ControlError::Security(
                "Unix control socket changed during bind".to_string(),
            ));
        }
        listener
            .set_nonblocking(true)
            .map_err(|error| endpoint_io("set Unix control socket nonblocking", error))?;
        Ok(Some((
            secret,
            ServerTransport {
                _singleton: singleton,
                _secret_file: secret_file,
                listener,
                _socket: socket,
                expected_peer_uid: self.effective_uid,
            },
        )))
    }

    pub(super) fn connect_before(
        &self,
        deadline: Instant,
    ) -> Result<ClientConnection, ControlError> {
        ensure_runtime_directory(&self.runtime_dir, self.effective_uid, false)?;
        match validate_socket(&self.socket_path, self.effective_uid) {
            Ok(_) => {}
            Err(ControlError::Unavailable) => return Err(ControlError::Unavailable),
            Err(error) => return Err(error),
        }
        let stream = connect_nonblocking(&self.socket_path, deadline)?;
        verify_peer(stream.as_raw_fd(), self.effective_uid)?;
        DeadlineStream::new(stream, deadline)
    }

    pub(super) fn read_secret(&self) -> Result<[u8; AUTH_SECRET_BYTES], ControlError> {
        ensure_runtime_directory(&self.runtime_dir, self.effective_uid, false)?;
        let mut file = open_existing_secure_file(&self.secret_path, self.effective_uid)?;
        let mut secret = [0_u8; AUTH_SECRET_BYTES];
        file.read_exact(&mut secret)
            .map_err(|error| endpoint_io("read Engine authentication secret", error))?;
        let mut trailing = [0_u8; 1];
        if file
            .read(&mut trailing)
            .map_err(|error| endpoint_io("validate Engine authentication secret", error))?
            != 0
        {
            return Err(ControlError::Security(
                "invalid Engine authentication secret".to_string(),
            ));
        }
        Ok(secret)
    }
}

pub(super) struct LaunchLock(FileLock);

impl LaunchLock {
    fn acquire(
        path: &Path,
        effective_uid: libc::uid_t,
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let lock = FileLock::open(path, effective_uid)?;
        loop {
            match lock.try_lock() {
                Ok(true) => return Ok(Self(lock)),
                Ok(false) if Instant::now() < deadline => {
                    thread::sleep(
                        RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Ok(false) => return Err(ControlError::Timeout),
                Err(error) => return Err(error),
            }
        }
    }
}

struct FileLock {
    file: File,
}

impl FileLock {
    fn open(path: &Path, effective_uid: libc::uid_t) -> Result<Self, ControlError> {
        Ok(Self {
            file: open_or_create_secure_file(path, effective_uid, false)?,
        })
    }

    fn try_singleton(
        path: &Path,
        effective_uid: libc::uid_t,
    ) -> Result<Option<Self>, ControlError> {
        let lock = Self::open(path, effective_uid)?;
        if lock.try_lock()? {
            Ok(Some(lock))
        } else {
            Ok(None)
        }
    }

    fn try_lock(&self) -> Result<bool, ControlError> {
        let result = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
        {
            Ok(false)
        } else {
            Err(endpoint_io("lock internal IPC file", error))
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

struct SecretFile {
    _file: File,
    _path: OwnedPath,
}

impl SecretFile {
    fn create(
        path: &Path,
        effective_uid: libc::uid_t,
        secret: &[u8; AUTH_SECRET_BYTES],
    ) -> Result<Self, ControlError> {
        let mut file = open_or_create_secure_file(path, effective_uid, true)?;
        let metadata = file
            .metadata()
            .map_err(|error| endpoint_io("inspect Engine authentication secret", error))?;
        let owned_path = OwnedPath::new(path, &metadata, ObjectKind::RegularFile);
        file.write_all(secret)
            .map_err(|error| endpoint_io("write Engine authentication secret", error))?;
        file.sync_all()
            .map_err(|error| endpoint_io("flush Engine authentication secret", error))?;
        Ok(Self {
            _file: file,
            _path: owned_path,
        })
    }
}

pub(super) struct ServerTransport {
    _secret_file: SecretFile,
    listener: UnixListener,
    _socket: OwnedPath,
    expected_peer_uid: libc::uid_t,
    // Rust drops fields in declaration order. Keep the singleton lock until
    // the secret and socket owned by this server have been removed.
    _singleton: FileLock,
}

impl ServerTransport {
    pub(super) fn accept(
        &mut self,
        stop: &AtomicBool,
    ) -> Result<Option<AcceptedConnection<'_>>, ControlError> {
        while !stop.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let stream = DeadlineStream::new(stream, Instant::now() + IO_TIMEOUT)?;
                    if let Err(error) = verify_peer(stream.as_raw_fd(), self.expected_peer_uid) {
                        warn!("rejected macOS IPC peer: {error}");
                        continue;
                    }
                    return Ok(Some(AcceptedConnection {
                        stream,
                        marker: PhantomData,
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(endpoint_io("accept Unix control connection", error));
                }
            }
        }
        Ok(None)
    }
}

pub(super) struct DeadlineStream {
    stream: UnixStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn new(stream: UnixStream, deadline: Instant) -> Result<Self, ControlError> {
        if Instant::now() >= deadline {
            return Err(ControlError::Timeout);
        }
        stream
            .set_nonblocking(true)
            .map_err(|error| endpoint_io("configure Unix control connection", error))?;
        Ok(Self { stream, deadline })
    }

    pub(super) fn set_deadline(&mut self, deadline: Instant) -> Result<(), ControlError> {
        if Instant::now() >= deadline {
            return Err(ControlError::Timeout);
        }
        self.deadline = deadline;
        Ok(())
    }

    fn wait(&self, events: libc::c_short) -> io::Result<()> {
        poll_until(self.stream.as_raw_fd(), events, self.deadline)
    }
}

impl AsRawFd for DeadlineStream {
    fn as_raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        loop {
            self.wait(libc::POLLIN)?;
            match self.stream.read(output) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        loop {
            self.wait(libc::POLLOUT)?;
            match self.stream.write(input) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) type ClientConnection = DeadlineStream;

pub(super) struct AcceptedConnection<'a> {
    stream: DeadlineStream,
    marker: PhantomData<&'a mut ServerTransport>,
}

impl AcceptedConnection<'_> {
    pub(super) fn set_deadline(&mut self, deadline: Instant) -> Result<(), ControlError> {
        self.stream.set_deadline(deadline)
    }
}

impl Read for AcceptedConnection<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.stream.read(output)
    }
}

impl Write for AcceptedConnection<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.stream.write(input)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

fn deadline_elapsed() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Unix control deadline elapsed")
}

fn poll_until(fd: RawFd, events: libc::c_short, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(deadline_elapsed)?;
        let millis = remaining
            .as_millis()
            .saturating_add((remaining.subsec_nanos() % 1_000_000 != 0) as u128)
            .clamp(1, i32::MAX as u128) as libc::c_int;
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        match unsafe { libc::poll(&mut pollfd, 1, millis) } {
            result if result > 0 => return Ok(()),
            0 => return Err(deadline_elapsed()),
            _ => {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

fn connect_nonblocking(path: &Path, deadline: Instant) -> Result<UnixStream, ControlError> {
    use std::os::unix::ffi::OsStrExt;

    if Instant::now() >= deadline {
        return Err(ControlError::Timeout);
    }
    let path_bytes = path.as_os_str().as_bytes();
    if path_bytes.len() > MAX_SOCKET_PATH_BYTES || path_bytes.contains(&0) {
        return Err(ControlError::Security(
            "invalid internal socket path".to_string(),
        ));
    }

    let raw_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw_fd < 0 {
        return Err(endpoint_io(
            "create Unix control connection",
            io::Error::last_os_error(),
        ));
    }
    let owned_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    configure_nonblocking(owned_fd.as_raw_fd())?;

    let mut address = unsafe { MaybeUninit::<libc::sockaddr_un>::zeroed().assume_init() };
    let address_length = std::mem::offset_of!(libc::sockaddr_un, sun_path)
        .checked_add(path_bytes.len() + 1)
        .ok_or_else(|| ControlError::Security("invalid internal socket path".to_string()))?;
    address.sun_len = u8::try_from(address_length)
        .map_err(|_| ControlError::Security("invalid internal socket path".to_string()))?;
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, source) in address.sun_path.iter_mut().zip(path_bytes) {
        *destination = *source as libc::c_char;
    }

    let connected = unsafe {
        libc::connect(
            owned_fd.as_raw_fd(),
            (&raw const address).cast(),
            address_length as libc::socklen_t,
        )
    };
    if connected != 0 {
        let error = io::Error::last_os_error();
        if !connect_is_pending(&error) {
            return Err(connect_error(error));
        }
        poll_until(owned_fd.as_raw_fd(), libc::POLLOUT, deadline).map_err(connect_wait_error)?;
        let mut socket_error = 0;
        let mut socket_error_bytes = size_of::<libc::c_int>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                owned_fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut socket_error).cast(),
                &mut socket_error_bytes,
            )
        } != 0
        {
            return Err(endpoint_io(
                "finish Unix control connection",
                io::Error::last_os_error(),
            ));
        }
        if socket_error != 0 {
            return Err(connect_error(io::Error::from_raw_os_error(socket_error)));
        }
    }
    Ok(UnixStream::from(owned_fd))
}

fn connect_is_pending(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINPROGRESS | libc::EALREADY | libc::EAGAIN | libc::EINTR)
    )
}

fn configure_nonblocking(fd: RawFd) -> Result<(), ControlError> {
    let status_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if status_flags < 0
        || unsafe { libc::fcntl(fd, libc::F_SETFL, status_flags | libc::O_NONBLOCK) } < 0
    {
        return Err(endpoint_io(
            "configure Unix control connection status",
            io::Error::last_os_error(),
        ));
    }
    let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if descriptor_flags < 0
        || unsafe { libc::fcntl(fd, libc::F_SETFD, descriptor_flags | libc::FD_CLOEXEC) } < 0
    {
        return Err(endpoint_io(
            "configure Unix control connection descriptor",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn connect_wait_error(error: io::Error) -> ControlError {
    if error.kind() == io::ErrorKind::TimedOut {
        ControlError::Timeout
    } else {
        endpoint_io("wait for Unix control connection", error)
    }
}

fn connect_error(error: io::Error) -> ControlError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => ControlError::Unavailable,
        io::ErrorKind::TimedOut => ControlError::Timeout,
        io::ErrorKind::PermissionDenied => {
            ControlError::Security("connect to Unix control socket: permission denied".to_string())
        }
        _ => endpoint_io("connect to Unix control socket", error),
    }
}

fn ensure_runtime_directory(
    path: &Path,
    effective_uid: libc::uid_t,
    create: bool,
) -> Result<(), ControlError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_metadata(
                &metadata,
                effective_uid,
                ObjectKind::Directory,
                DIRECTORY_MODE,
            )?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && !create => {
            Err(ControlError::Unavailable)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.mode(DIRECTORY_MODE);
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(endpoint_io("create private IPC runtime directory", error));
                }
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| endpoint_io("inspect IPC runtime directory", error))?;
            validate_metadata(
                &metadata,
                effective_uid,
                ObjectKind::Directory,
                DIRECTORY_MODE,
            )?;
            Ok(())
        }
        Err(error) => Err(endpoint_io("inspect IPC runtime directory", error)),
    }
}

fn bind_private_socket(path: &Path) -> Result<UnixListener, ControlError> {
    UnixListener::bind(path).map_err(|error| endpoint_io("bind Unix control socket", error))
}

fn remove_stale_socket(path: &Path, effective_uid: libc::uid_t) -> Result<(), ControlError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() || metadata.uid() != effective_uid {
                return Err(ControlError::Security(
                    "unsafe existing internal IPC socket".to_string(),
                ));
            }
            quarantine_owned_path(path, identity(&metadata))?.remove()
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(endpoint_io("inspect Unix control socket", error)),
    }
}

fn validate_socket(path: &Path, effective_uid: libc::uid_t) -> Result<Metadata, ControlError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ControlError::Unavailable
        } else {
            endpoint_io("inspect Unix control socket", error)
        }
    })?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid {
        return Err(ControlError::Security(
            "unsafe existing internal IPC socket".to_string(),
        ));
    }
    Ok(metadata)
}

fn open_or_create_secure_file(
    path: &Path,
    effective_uid: libc::uid_t,
    truncate: bool,
) -> Result<File, ControlError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_metadata(&metadata, effective_uid, ObjectKind::RegularFile, FILE_MODE)?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(endpoint_io("inspect internal IPC file", error)),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| endpoint_io("open internal IPC file", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| endpoint_io("inspect opened IPC file", error))?;
    validate_metadata(&metadata, effective_uid, ObjectKind::RegularFile, FILE_MODE)?;
    validate_path_identity(path, &metadata)?;
    if truncate {
        file.set_len(0)
            .map_err(|error| endpoint_io("truncate internal IPC file", error))?;
    }
    Ok(file)
}

fn open_existing_secure_file(
    path: &Path,
    effective_uid: libc::uid_t,
) -> Result<File, ControlError> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| endpoint_io("inspect Engine authentication secret", error))?;
    validate_metadata(
        &path_metadata,
        effective_uid,
        ObjectKind::RegularFile,
        FILE_MODE,
    )?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| endpoint_io("open Engine authentication secret", error))?;
    let file_metadata = file
        .metadata()
        .map_err(|error| endpoint_io("inspect opened authentication secret", error))?;
    validate_metadata(
        &file_metadata,
        effective_uid,
        ObjectKind::RegularFile,
        FILE_MODE,
    )?;
    if identity(&path_metadata) != identity(&file_metadata) {
        return Err(ControlError::Security(
            "Engine authentication secret changed during access".to_string(),
        ));
    }
    Ok(file)
}

fn validate_path_identity(path: &Path, opened: &Metadata) -> Result<(), ControlError> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| endpoint_io("reinspect internal IPC file", error))?;
    if identity(&path_metadata) == identity(opened) {
        Ok(())
    } else {
        Err(ControlError::Security(
            "internal IPC file changed during access".to_string(),
        ))
    }
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Directory,
    RegularFile,
    Socket,
}

fn validate_metadata(
    metadata: &Metadata,
    effective_uid: libc::uid_t,
    kind: ObjectKind,
    exact_mode: u32,
) -> Result<(), ControlError> {
    let correct_kind = match kind {
        ObjectKind::Directory => metadata.file_type().is_dir(),
        ObjectKind::RegularFile => metadata.file_type().is_file(),
        ObjectKind::Socket => metadata.file_type().is_socket(),
    };
    if !correct_kind
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o7777 != exact_mode
        || matches!(kind, ObjectKind::RegularFile) && metadata.nlink() != 1
    {
        return Err(ControlError::Security(
            "unsafe existing internal IPC object".to_string(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PathIdentity {
    device: u64,
    inode: u64,
}

fn identity(metadata: &Metadata) -> PathIdentity {
    PathIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

struct OwnedPath {
    path: PathBuf,
    identity: PathIdentity,
    kind: ObjectKind,
}

impl OwnedPath {
    fn new(path: &Path, metadata: &Metadata, kind: ObjectKind) -> Self {
        Self {
            path: path.to_path_buf(),
            identity: identity(metadata),
            kind,
        }
    }

    fn remove_if_same(&self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        let correct_kind = match self.kind {
            ObjectKind::RegularFile => metadata.file_type().is_file(),
            ObjectKind::Socket => metadata.file_type().is_socket(),
            ObjectKind::Directory => false,
        };
        if correct_kind && identity(&metadata) == self.identity {
            match quarantine_owned_path(&self.path, self.identity) {
                Ok(quarantine) => {
                    if let Err(error) = quarantine.remove() {
                        warn!("preserved replaced internal IPC quarantine: {error}");
                    }
                }
                Err(error) => warn!("preserved replaced internal IPC object: {error}"),
            }
        }
    }
}

impl Drop for OwnedPath {
    fn drop(&mut self) {
        self.remove_if_same();
    }
}

struct QuarantinedPath {
    path: PathBuf,
    identity: PathIdentity,
}

impl QuarantinedPath {
    fn remove(self) -> Result<(), ControlError> {
        let metadata = fs::symlink_metadata(&self.path)
            .map_err(|error| endpoint_io("inspect quarantined IPC object", error))?;
        if identity(&metadata) != self.identity {
            return Err(ControlError::Security(
                "quarantined IPC object was replaced and was preserved".to_string(),
            ));
        }
        fs::remove_file(&self.path)
            .map_err(|error| endpoint_io("remove quarantined IPC object", error))
    }
}

fn quarantine_owned_path(
    path: &Path,
    expected: PathIdentity,
) -> Result<QuarantinedPath, ControlError> {
    for _ in 0..QUARANTINE_ATTEMPTS {
        let quarantine = quarantine_path(path)?;
        match rename_exclusive(path, &quarantine) {
            Ok(()) => {
                let metadata = fs::symlink_metadata(&quarantine)
                    .map_err(|error| endpoint_io("inspect quarantined IPC object", error))?;
                if identity(&metadata) != expected {
                    return Err(ControlError::Security(
                        "internal IPC object changed during quarantine and was preserved"
                            .to_string(),
                    ));
                }
                return Ok(QuarantinedPath {
                    path: quarantine,
                    identity: expected,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(endpoint_io("quarantine internal IPC object", error)),
        }
    }
    Err(ControlError::Security(
        "cannot allocate bounded IPC quarantine".to_string(),
    ))
}

fn quarantine_path(path: &Path) -> Result<PathBuf, ControlError> {
    let parent = path.parent().ok_or_else(|| {
        ControlError::Security("internal IPC object has no parent directory".to_string())
    })?;
    let mut random = [0_u8; 8];
    unsafe {
        libc::arc4random_buf(random.as_mut_ptr().cast(), random.len());
    }
    Ok(parent.join(format!(
        ".ipc-quarantine-{:016x}",
        u64::from_ne_bytes(random)
    )))
}

fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid IPC source path"))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid IPC quarantine path"))?;
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn verify_peer(fd: RawFd, effective_uid: libc::uid_t) -> Result<(), ControlError> {
    let mut peer_uid = 0;
    let mut peer_gid = 0;
    if unsafe { libc::getpeereid(fd, &mut peer_uid, &mut peer_gid) } != 0 {
        return Err(endpoint_io(
            "read Unix peer credentials",
            io::Error::last_os_error(),
        ));
    }
    require_peer_uid(peer_uid, effective_uid)
}

fn require_peer_uid(peer_uid: libc::uid_t, effective_uid: libc::uid_t) -> Result<(), ControlError> {
    if peer_uid == effective_uid {
        Ok(())
    } else {
        Err(ControlError::Security(
            "Unix control peer UID does not match the Engine user".to_string(),
        ))
    }
}

fn generate_secret() -> [u8; AUTH_SECRET_BYTES] {
    let mut secret = [0_u8; AUTH_SECRET_BYTES];
    unsafe {
        libc::arc4random_buf(secret.as_mut_ptr().cast(), secret.len());
    }
    secret
}

pub(super) fn process_resources() -> Result<(u32, u32, u64), ControlError> {
    let pid = i32::try_from(std::process::id())
        .map_err(|_| ControlError::Io(io::Error::other("process id exceeds macOS range")))?;
    process_resources_for_pid(pid)
}

fn process_resources_for_pid(pid: libc::c_int) -> Result<(u32, u32, u64), ControlError> {
    let mut task = MaybeUninit::<libc::proc_taskinfo>::uninit();
    let task_bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            task.as_mut_ptr().cast(),
            size_of::<libc::proc_taskinfo>() as libc::c_int,
        )
    };
    let task_error = io::Error::last_os_error();
    require_positive_proc_result(
        task_bytes,
        "read macOS process task information",
        task_error,
    )?;
    if task_bytes != size_of::<libc::proc_taskinfo>() as libc::c_int {
        return Err(ControlError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "macOS process task information was incomplete",
        )));
    }
    let task = unsafe { task.assume_init() };
    let descriptor_bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let descriptor_error = io::Error::last_os_error();
    require_positive_proc_result(
        descriptor_bytes,
        "count macOS process descriptors",
        descriptor_error,
    )?;
    Ok((
        u32::try_from(task.pti_threadnum).unwrap_or(0),
        u32::try_from(descriptor_bytes / libc::PROC_PIDLISTFD_SIZE).unwrap_or(u32::MAX),
        task.pti_resident_size,
    ))
}

fn require_positive_proc_result(
    result: libc::c_int,
    operation: &'static str,
    error: io::Error,
) -> Result<libc::c_int, ControlError> {
    if result <= 0 {
        Err(context_io(operation, error))
    } else {
        Ok(result)
    }
}

#[derive(Debug)]
struct OperationIoError {
    operation: &'static str,
    source: io::Error,
}

impl std::fmt::Display for OperationIoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.source)
    }
}

impl std::error::Error for OperationIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn context_io(operation: &'static str, error: io::Error) -> ControlError {
    ControlError::Io(io::Error::new(
        error.kind(),
        OperationIoError {
            operation,
            source: error,
        },
    ))
}

fn endpoint_io(operation: &'static str, error: io::Error) -> ControlError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        ControlError::Security(format!("{operation}: permission denied"))
    } else {
        context_io(operation, error)
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::{
        EngineControl, EngineServer, ServerExit, CONNECT_TIMEOUT, IO_TIMEOUT,
    };
    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    static NEXT_SUFFIX: AtomicU64 = AtomicU64::new(1);
    const HELPER_ENV: &str = "ZG_P04B1_PROCESS_HELPER";
    const HELPER_DIR_ENV: &str = "ZG_P04B1_PROCESS_HELPER_DIR";
    const HELPER_SUFFIX_ENV: &str = "ZG_P04B1_PROCESS_HELPER_SUFFIX";

    fn suffix() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            NEXT_SUFFIX.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn endpoint_in(root: &Path) -> Endpoint {
        let runtime_dir = root.join("runtime");
        Endpoint {
            socket_path: runtime_dir.join("control.sock"),
            singleton_path: runtime_dir.join("engine.lock"),
            launch_path: runtime_dir.join("launch.lock"),
            secret_path: runtime_dir.join("control.secret"),
            runtime_dir,
            effective_uid: unsafe { libc::geteuid() },
        }
    }

    fn load_active(path: &Path) -> config::ActiveConfig {
        match config::load(path).unwrap() {
            config::LoadResult::Ready(active) | config::LoadResult::Missing(active) => active,
        }
    }

    struct RunningServer {
        control: EngineControl,
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<ServerExit>>,
        _directory: TempDir,
    }

    impl RunningServer {
        fn start() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let suffix = suffix();
            let server = EngineServer::for_test(directory.path(), &suffix)
                .unwrap()
                .unwrap();
            let control = EngineControl::for_prepared_server(&server);
            let (owner, _) = ConfigOwner::startup(directory.path());
            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                server
                    .run(
                        server_stop,
                        owner,
                        Arc::new(crate::capture::WindowCapture::new()),
                        crate::window_info::get_window_info_at_point,
                        |_, _| Ok(()),
                    )
                    .unwrap()
            });
            Self {
                control,
                stop,
                handle: Some(handle),
                _directory: directory,
            }
        }
    }

    impl Drop for RunningServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
        }
    }

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    fn spawn_process_helper(directory: &Path, suffix: &str) -> ChildGuard {
        ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ipc::macos::tests::process_helper",
                    "--nocapture",
                ])
                .env(HELPER_ENV, "1")
                .env(HELPER_DIR_ENV, directory)
                .env(HELPER_SUFFIX_ENV, suffix)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn connect_process_helper(control: &EngineControl) {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match control.ping() {
                Ok(()) => return,
                Err(ControlError::Unavailable) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                result => panic!("process helper did not become ready: {result:?}"),
            }
        }
    }

    #[test]
    fn process_helper() {
        if std::env::var_os(HELPER_ENV).is_none() {
            return;
        }
        let directory = PathBuf::from(std::env::var_os(HELPER_DIR_ENV).unwrap());
        let suffix = std::env::var(HELPER_SUFFIX_ENV).unwrap();
        let server = EngineServer::for_test(&directory, &suffix)
            .unwrap()
            .unwrap();
        let (owner, _) = ConfigOwner::startup(&directory);
        assert_eq!(
            server
                .run(Arc::new(AtomicBool::new(false)), owner, |_, _| Ok(()))
                .unwrap(),
            ServerExit::Shutdown
        );
    }

    #[test]
    fn runtime_directory_has_exact_mode_and_effective_uid_owner() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let _launch = endpoint
            .acquire_launch_lock(Instant::now() + IO_TIMEOUT)
            .unwrap();
        let metadata = fs::symlink_metadata(&endpoint.runtime_dir).unwrap();
        assert_eq!(metadata.mode() & 0o7777, DIRECTORY_MODE);
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    }

    #[test]
    fn authentication_secret_has_literal_exact_mode_0600() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let (_, _server) = endpoint.prepare_server().unwrap().unwrap();
        assert_eq!(
            fs::symlink_metadata(&endpoint.secret_path).unwrap().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn singleton_lock_has_literal_exact_mode_0600() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let (_, _server) = endpoint.prepare_server().unwrap().unwrap();
        assert_eq!(
            fs::symlink_metadata(&endpoint.singleton_path)
                .unwrap()
                .mode()
                & 0o7777,
            0o600
        );
    }

    #[test]
    fn launch_lock_has_literal_exact_mode_0600() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let _launch = endpoint
            .acquire_launch_lock(Instant::now() + IO_TIMEOUT)
            .unwrap();
        assert_eq!(
            fs::symlink_metadata(&endpoint.launch_path).unwrap().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn unsafe_runtime_directory_mode_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        fs::create_dir(&endpoint.runtime_dir).unwrap();
        fs::set_permissions(&endpoint.runtime_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
    }

    #[test]
    fn symlink_runtime_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let target = directory.path().join("target-runtime");
        fs::create_dir(&target).unwrap();
        symlink(&target, &endpoint.runtime_dir).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
    }

    #[test]
    fn symlink_socket_object_is_rejected_without_removal() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"owned").unwrap();
        symlink(&target, &endpoint.socket_path).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
        assert!(endpoint
            .socket_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(target).unwrap(), b"owned");
    }

    #[test]
    fn non_socket_endpoint_object_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        fs::write(&endpoint.socket_path, b"not-a-socket").unwrap();
        fs::set_permissions(&endpoint.socket_path, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
    }

    #[test]
    fn symlink_secret_object_is_rejected_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let target = directory.path().join("target-secret");
        fs::write(&target, b"do-not-change").unwrap();
        symlink(&target, &endpoint.secret_path).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
        assert_eq!(fs::read(target).unwrap(), b"do-not-change");
    }

    #[test]
    fn hard_linked_secret_object_is_rejected_without_truncation() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let target = directory.path().join("target-secret");
        fs::write(&target, b"do-not-change").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        fs::hard_link(&target, &endpoint.secret_path).unwrap();
        assert!(matches!(
            endpoint.prepare_server(),
            Err(ControlError::Security(_))
        ));
        assert_eq!(fs::read(target).unwrap(), b"do-not-change");
    }

    #[test]
    fn wrong_owner_is_rejected_at_the_metadata_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("owned");
        fs::write(&file, b"owned").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        let metadata = fs::symlink_metadata(file).unwrap();
        assert!(matches!(
            validate_metadata(
                &metadata,
                metadata.uid().wrapping_add(1),
                ObjectKind::RegularFile,
                FILE_MODE
            ),
            Err(ControlError::Security(_))
        ));
    }

    #[test]
    fn safe_stale_socket_is_replaced_before_bind() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let stale = UnixListener::bind(&endpoint.socket_path).unwrap();
        fs::set_permissions(&endpoint.socket_path, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        let stale_identity = identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap());
        drop(stale);

        let (_, server) = endpoint.prepare_server().unwrap().unwrap();
        assert_ne!(
            identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap()),
            stale_identity
        );
        drop(server);
    }

    #[test]
    fn server_drop_removes_its_owned_socket() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let (_, server) = endpoint.prepare_server().unwrap().unwrap();
        assert!(endpoint.socket_path.exists());
        drop(server);
        assert!(!endpoint.socket_path.exists());
    }

    #[test]
    fn socket_creation_does_not_chmod_a_replacement_path() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let _listener = bind_private_socket(&endpoint.socket_path).unwrap();
        let displaced = endpoint.runtime_dir.join("displaced.sock");
        fs::rename(&endpoint.socket_path, &displaced).unwrap();
        fs::write(&endpoint.socket_path, b"replacement").unwrap();
        fs::set_permissions(&endpoint.socket_path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            validate_socket(&endpoint.socket_path, endpoint.effective_uid),
            Err(ControlError::Security(_))
        ));
        assert_eq!(
            fs::symlink_metadata(&endpoint.socket_path).unwrap().mode() & 0o7777,
            0o644
        );
    }

    #[test]
    fn stale_cleanup_preserves_a_racing_replacement_in_quarantine() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let _stale = bind_private_socket(&endpoint.socket_path).unwrap();
        let stale_identity = identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap());
        fs::rename(
            &endpoint.socket_path,
            endpoint.runtime_dir.join("displaced.sock"),
        )
        .unwrap();
        let _replacement = bind_private_socket(&endpoint.socket_path).unwrap();
        let replacement_identity = identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap());

        assert!(matches!(
            quarantine_owned_path(&endpoint.socket_path, stale_identity),
            Err(ControlError::Security(_))
        ));
        assert!(fs::read_dir(&endpoint.runtime_dir).unwrap().any(|entry| {
            entry
                .ok()
                .and_then(|entry| entry.metadata().ok())
                .is_some_and(|metadata| identity(&metadata) == replacement_identity)
        }));
    }

    #[test]
    fn owned_drop_preserves_a_replacement_at_the_socket_path() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let _owned_listener = bind_private_socket(&endpoint.socket_path).unwrap();
        let owned_metadata = fs::symlink_metadata(&endpoint.socket_path).unwrap();
        let owned = OwnedPath::new(&endpoint.socket_path, &owned_metadata, ObjectKind::Socket);
        fs::rename(
            &endpoint.socket_path,
            endpoint.runtime_dir.join("displaced.sock"),
        )
        .unwrap();
        let _replacement = bind_private_socket(&endpoint.socket_path).unwrap();
        let replacement_identity = identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap());

        drop(owned);
        assert_eq!(
            identity(&fs::symlink_metadata(&endpoint.socket_path).unwrap()),
            replacement_identity
        );
    }

    #[test]
    fn quarantined_cleanup_preserves_a_replacement_object() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quarantine");
        fs::write(&path, b"owned").unwrap();
        let owned_identity = identity(&fs::symlink_metadata(&path).unwrap());
        let quarantined = QuarantinedPath {
            path: path.clone(),
            identity: owned_identity,
        };
        fs::rename(&path, directory.path().join("displaced")).unwrap();
        fs::write(&path, b"replacement").unwrap();

        assert!(matches!(
            quarantined.remove(),
            Err(ControlError::Security(_))
        ));
        assert_eq!(fs::read(path).unwrap(), b"replacement");
    }

    #[test]
    fn singleton_lock_allows_only_one_engine_endpoint_owner() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let (_, first) = endpoint.prepare_server().unwrap().unwrap();
        assert!(endpoint.prepare_server().unwrap().is_none());
        drop(first);
    }

    #[test]
    fn actual_bsd_peer_credentials_match_the_effective_uid() {
        let (left, right) = UnixStream::pair().unwrap();
        verify_peer(left.as_raw_fd(), unsafe { libc::geteuid() }).unwrap();
        verify_peer(right.as_raw_fd(), unsafe { libc::geteuid() }).unwrap();
    }

    #[test]
    fn accepted_connection_rejects_actual_peer_uid_before_protocol() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        let (_, mut server) = endpoint.prepare_server().unwrap().unwrap();
        server.expected_peer_uid = endpoint.effective_uid.wrapping_add(1);
        let path = endpoint.socket_path.clone();
        let client = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            let _ = stream.write_all(&[0xff]);
        });
        let stop = Arc::new(AtomicBool::new(false));
        let stop_later = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            stop_later.store(true, Ordering::Release);
        });

        assert!(server.accept(&stop).unwrap().is_none());
        client.join().unwrap();
        stopper.join().unwrap();
    }

    #[test]
    fn uds_status_reports_engine_and_config_owner_state() {
        let server = RunningServer::start();
        let status = server.control.status().unwrap();
        assert_eq!(status.process_id, std::process::id());
        assert!(status.config_available);
        assert_eq!((status.config_revision, status.config_generation), (1, 1));
    }

    #[test]
    fn uds_apply_uses_the_engine_config_owner_and_persists() {
        let server = RunningServer::start();
        let mut document = ConfigDocument::default();
        document.shared.appearance.trail_thickness = 7.0;
        let applied = server.control.apply_config(document, 1).unwrap();
        assert_eq!(
            (applied.current.revision, applied.current.generation),
            (2, 2)
        );
        assert_eq!(
            load_active(server._directory.path())
                .document()
                .shared
                .appearance
                .trail_thickness,
            7.0
        );
    }

    #[test]
    fn uds_import_bytes_uses_the_same_validation_and_owner_transaction() {
        let server = RunningServer::start();
        let mut document = ConfigDocument::default();
        document.shared.appearance.trail_thickness = 8.0;
        let applied = server
            .control
            .apply_config_bytes(serde_json::to_vec(&document).unwrap(), 1)
            .unwrap();
        assert_eq!(
            applied
                .current
                .config
                .unwrap()
                .shared
                .appearance
                .trail_thickness,
            8.0
        );
    }

    #[test]
    fn uds_export_reads_the_exact_engine_snapshot() {
        let server = RunningServer::start();
        let current = server.control.current_config().unwrap();
        let export = server._directory.path().join("export.json");
        config::export(&current.config.unwrap(), &export).unwrap();
        let exported = config::decode_and_compile(&fs::read(export).unwrap()).unwrap();
        assert_eq!(exported.document(), &ConfigDocument::default());
    }

    #[test]
    fn uds_set_enabled_commits_through_the_config_owner() {
        let server = RunningServer::start();
        let applied = server.control.set_enabled(false, 1).unwrap();
        assert!(!applied.current.config.unwrap().shared.enabled);
        assert_eq!(
            (applied.current.revision, applied.current.generation),
            (2, 2)
        );
    }

    #[test]
    fn malformed_uds_client_does_not_stop_the_engine() {
        let server = RunningServer::start();
        let endpoint = server.control.endpoint.clone();
        let mut connection = endpoint
            .connect_before(Instant::now() + IO_TIMEOUT)
            .unwrap();
        connection.write_all(&1_u32.to_le_bytes()).unwrap();
        connection.write_all(&[0xff]).unwrap();
        drop(connection);
        server.control.ping().unwrap();
    }

    #[test]
    fn slowloris_frame_cannot_extend_the_absolute_connection_deadline() {
        let server = RunningServer::start();
        let path = server.control.endpoint.socket_path.clone();
        let slow_client = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            for byte in [4_u8, 0] {
                let _ = stream.write_all(&[byte]);
                thread::sleep(Duration::from_millis(600));
            }
        });
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();

        server.control.ping().unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(1_100),
            "slow client extended the absolute frame deadline"
        );
        slow_client.join().unwrap();
    }

    #[test]
    fn saturated_unix_backlog_respects_the_bounded_connect_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let _listener = bind_private_socket(&endpoint.socket_path).unwrap();
        let mut clients = Vec::new();
        let mut saturated = false;
        for _ in 0..1_024 {
            match connect_nonblocking(
                &endpoint.socket_path,
                Instant::now() + Duration::from_millis(25),
            ) {
                Ok(stream) => clients.push(stream),
                Err(ControlError::Timeout | ControlError::Unavailable) => {
                    saturated = true;
                    break;
                }
                Err(error) => panic!("unexpected backlog fill error: {error}"),
            }
        }
        assert!(saturated, "Unix socket backlog did not saturate");
        let started = Instant::now();
        assert!(matches!(
            endpoint.connect_before(Instant::now() + Duration::from_millis(100)),
            Err(ControlError::Timeout | ControlError::Unavailable)
        ));
        assert!(started.elapsed() < Duration::from_millis(300));
    }

    #[test]
    fn expired_deadline_setup_is_rejected() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let mut connection = DeadlineStream::new(stream, Instant::now() + IO_TIMEOUT).unwrap();
        assert!(matches!(
            connection.set_deadline(Instant::now()),
            Err(ControlError::Timeout)
        ));
    }

    #[test]
    fn deadline_stream_constructor_rejects_an_expired_deadline() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert!(matches!(
            DeadlineStream::new(stream, Instant::now()),
            Err(ControlError::Timeout)
        ));
    }

    #[test]
    fn interrupted_nonblocking_connect_is_classified_as_pending() {
        assert!(connect_is_pending(&io::Error::from_raw_os_error(
            libc::EINTR
        )));
    }

    #[test]
    fn disconnected_uds_client_releases_only_its_candidate() {
        let server = RunningServer::start();
        let mut session = super::super::core::Session::connect(&server.control.endpoint).unwrap();
        assert!(matches!(
            session
                .exchange(super::super::protocol::Request::PrepareConfig {
                    expected_revision: 1,
                    config_bytes: serde_json::to_vec(&ConfigDocument::default()).unwrap(),
                })
                .unwrap(),
            super::super::protocol::Response::Prepared { .. }
        ));
        drop(session);
        server.control.set_enabled(false, 1).unwrap();
    }

    #[test]
    fn macos_process_uds_persists_engine_owned_config() {
        let directory = tempfile::tempdir().unwrap();
        let suffix = suffix();
        let control = EngineControl::for_test(directory.path(), &suffix).unwrap();
        let _child = spawn_process_helper(directory.path(), &suffix);
        connect_process_helper(&control);
        let mut document = ConfigDocument::default();
        document.shared.appearance.trail_thickness = 9.0;
        control.apply_config(document, 1).unwrap();
        assert_eq!(
            load_active(directory.path())
                .document()
                .shared
                .appearance
                .trail_thickness,
            9.0
        );
        assert!(!control.shutdown().unwrap());
    }

    #[test]
    fn uds_shutdown_is_idempotent_for_the_accepted_session() {
        let server = RunningServer::start();
        let mut session = super::super::core::Session::connect(&server.control.endpoint).unwrap();
        assert_eq!(
            session
                .exchange(super::super::protocol::Request::Shutdown)
                .unwrap(),
            super::super::protocol::Response::Shutdown {
                already_requested: false
            }
        );
        assert_eq!(
            session
                .exchange(super::super::protocol::Request::Shutdown)
                .unwrap(),
            super::super::protocol::Response::Shutdown {
                already_requested: true
            }
        );
    }

    #[test]
    fn invalid_proc_pidinfo_pid_preserves_the_actual_errno_source() {
        use std::error::Error;

        let error = process_resources_for_pid(libc::c_int::MAX).unwrap_err();
        let ControlError::Io(error) = error else {
            panic!("expected contextual I/O failure");
        };
        let context = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<OperationIoError>())
            .unwrap();
        assert_eq!(context.operation, "read macOS process task information");
        let raw_errno = Error::source(context)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .and_then(io::Error::raw_os_error)
            .unwrap();
        assert_ne!(raw_errno, 0);
    }
}
