use super::core::{ControlError, ACCEPT_POLL_INTERVAL, RETRY_INTERVAL};
use super::protocol::AUTH_SECRET_BYTES;
use log::warn;
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUNTIME_DIRECTORY: &str = "dev.r4ai.zero-gesture";
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const MAX_SUFFIX_BYTES: usize = 32;
const MAX_SOCKET_PATH_BYTES: usize = 103;

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
        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|error| endpoint_io("bind Unix control socket", error))?;
        let bound_metadata = fs::symlink_metadata(&self.socket_path)
            .map_err(|error| endpoint_io("inspect bound Unix control socket", error))?;
        let socket = OwnedPath::new(&self.socket_path, &bound_metadata, ObjectKind::Socket);
        fs::set_permissions(&self.socket_path, fs::Permissions::from_mode(FILE_MODE))
            .map_err(|error| endpoint_io("restrict Unix control socket", error))?;
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
                effective_uid: self.effective_uid,
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
        if Instant::now() >= deadline {
            return Err(ControlError::Timeout);
        }
        let stream =
            UnixStream::connect(&self.socket_path).map_err(|error| match error.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    ControlError::Unavailable
                }
                io::ErrorKind::PermissionDenied => ControlError::Security(
                    "connect to Unix control socket: permission denied".to_string(),
                ),
                _ => endpoint_io("connect to Unix control socket", error),
            })?;
        verify_peer(stream.as_raw_fd(), self.effective_uid)?;
        Ok(ClientConnection { stream })
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
    effective_uid: libc::uid_t,
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
                    stream
                        .set_nonblocking(false)
                        .map_err(|error| endpoint_io("configure Unix control connection", error))?;
                    if let Err(error) = verify_peer(stream.as_raw_fd(), self.effective_uid) {
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

pub(super) struct ClientConnection {
    stream: UnixStream,
}

impl ClientConnection {
    pub(super) fn set_deadline(&mut self, deadline: Instant) {
        set_stream_deadline(&self.stream, deadline);
    }
}

impl Read for ClientConnection {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.stream.read(output)
    }
}

impl Write for ClientConnection {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.stream.write(input)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub(super) struct AcceptedConnection<'a> {
    stream: UnixStream,
    marker: PhantomData<&'a mut ServerTransport>,
}

impl AcceptedConnection<'_> {
    pub(super) fn set_deadline(&mut self, deadline: Instant) {
        set_stream_deadline(&self.stream, deadline);
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

fn set_stream_deadline(stream: &UnixStream, deadline: Instant) {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1));
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
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

fn remove_stale_socket(path: &Path, effective_uid: libc::uid_t) -> Result<(), ControlError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_metadata(&metadata, effective_uid, ObjectKind::Socket, FILE_MODE)?;
            fs::remove_file(path)
                .map_err(|error| endpoint_io("remove stale Unix control socket", error))
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
    validate_metadata(&metadata, effective_uid, ObjectKind::Socket, FILE_MODE)?;
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

#[derive(Clone, Copy, Eq, PartialEq)]
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
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for OwnedPath {
    fn drop(&mut self) {
        self.remove_if_same();
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
    if task_bytes != size_of::<libc::proc_taskinfo>() as libc::c_int {
        return Err(endpoint_io(
            "read macOS process task information",
            io::Error::last_os_error(),
        ));
    }
    let task = unsafe { task.assume_init() };
    let descriptor_bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if descriptor_bytes < 0 {
        return Err(endpoint_io(
            "count macOS process descriptors",
            io::Error::last_os_error(),
        ));
    }
    Ok((
        u32::try_from(task.pti_threadnum).unwrap_or(0),
        u32::try_from(descriptor_bytes / libc::PROC_PIDLISTFD_SIZE).unwrap_or(u32::MAX),
        task.pti_resident_size,
    ))
}

fn endpoint_io(operation: &str, error: io::Error) -> ControlError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        ControlError::Security(format!("{operation}: permission denied"))
    } else {
        ControlError::Io(io::Error::new(error.kind(), operation.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::{
        EngineControl, EngineServer, ServerExit, CONNECT_TIMEOUT, IO_TIMEOUT,
    };
    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
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
            let handle =
                thread::spawn(move || server.run(server_stop, owner, |_, _| Ok(())).unwrap());
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
    fn stale_owned_socket_is_replaced_and_server_cleans_its_socket() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(directory.path());
        ensure_runtime_directory(&endpoint.runtime_dir, endpoint.effective_uid, true).unwrap();
        let stale = UnixListener::bind(&endpoint.socket_path).unwrap();
        fs::set_permissions(&endpoint.socket_path, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        drop(stale);

        let (_, server) = endpoint.prepare_server().unwrap().unwrap();
        assert!(endpoint.socket_path.exists());
        drop(server);
        assert!(!endpoint.socket_path.exists());
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
    fn peer_uid_mismatch_is_rejected_at_the_credential_boundary() {
        let effective_uid = unsafe { libc::geteuid() };
        assert!(matches!(
            require_peer_uid(effective_uid.wrapping_add(1), effective_uid),
            Err(ControlError::Security(_))
        ));
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
}
