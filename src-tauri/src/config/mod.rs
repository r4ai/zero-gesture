mod compile;
mod document;
mod owner;
mod publication;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use document::{
    AppMatcher, AppearanceSettings, Application, ApplicationRecord, BindingRecord, ConfigDocument,
    ConfigError, DocumentAction, GestureBinding, GestureMode, GesturePattern, GestureStep, Key,
    MatchMethod, MatchTarget, PlatformOverride, PlatformSettings, RecognitionSettings,
    SharedSettings, TriggerButton, DEFAULT_APP_ID, SCHEMA_VERSION,
};

pub(crate) use compile::RuntimeConfig;
#[cfg(any(windows, test))]
pub(crate) use owner::ConfigOwner;
pub(crate) use owner::MAX_CONFIG_BYTES;
#[cfg(windows)]
pub(crate) use owner::{ConfigOwnerError, ConfigOwnerStatus, PreparedToken};
pub(crate) use publication::ConfigSnapshotReader;

const CONFIG_FILE_NAME: &str = "zero-gesture.config.json";
const MIGRATION_BACKUP_STEM: &str = "zero-gesture.config.v1.backup";
const TEMPORARY_PREFIX: &str = ".zero-gesture.config.";
const TEMPORARY_SUFFIX: &str = ".tmp";

pub const MAX_GESTURE_STEPS: usize = 8;
pub const DEFAULT_SAFETY_TIMEOUT_MS: u32 = 2_000;
pub const DEFAULT_MIN_SEGMENT_PX: i32 = 12;
pub const DEFAULT_DIRECTION_SWITCH_CONFIRM_PX: i32 = 8;
pub const DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX: i32 = 2;
pub const DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX: i32 = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Action {
    Keyboard { keys: Vec<String> },
}

#[derive(Clone)]
pub struct ActiveConfig {
    document: ConfigDocument,
    runtime: Arc<RuntimeConfig>,
}

impl ActiveConfig {
    pub fn from_document(document: ConfigDocument) -> Result<Self, ConfigError> {
        let runtime = Arc::new(compile::compile(&document)?);
        Ok(Self { document, runtime })
    }

    pub fn document(&self) -> &ConfigDocument {
        &self.document
    }

    pub(crate) fn runtime(&self) -> Arc<RuntimeConfig> {
        self.runtime.clone()
    }

    pub fn enabled(&self) -> bool {
        self.runtime.enabled
    }
}

pub(crate) fn encode(active: &ActiveConfig) -> Result<Vec<u8>, ConfigError> {
    serde_json::to_vec_pretty(active.document())
        .map_err(|error| ConfigError::at("$", format!("failed to serialize config: {error}")))
}

pub enum LoadResult {
    Ready(ActiveConfig),
    Missing(ActiveConfig),
}

pub fn load(config_dir: &Path) -> Result<LoadResult, ConfigError> {
    let path = config_path(config_dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ActiveConfig::from_document(ConfigDocument::default()).map(LoadResult::Missing);
        }
        Err(error) => {
            return Err(ConfigError::at(
                path.display().to_string(),
                format!("failed to read config: {error}"),
            ));
        }
    };

    match document::decode(&bytes)? {
        document::DecodedDocument::Current(document) => {
            ActiveConfig::from_document(document).map(LoadResult::Ready)
        }
        document::DecodedDocument::Migrated(document) => {
            let active = ActiveConfig::from_document(document)?;
            persist_migration(&active.document, &bytes, config_dir)?;
            Ok(LoadResult::Ready(active))
        }
    }
}

pub fn decode_and_compile(bytes: &[u8]) -> Result<ActiveConfig, ConfigError> {
    let document = match document::decode(bytes)? {
        document::DecodedDocument::Current(document)
        | document::DecodedDocument::Migrated(document) => document,
    };
    ActiveConfig::from_document(document)
}

pub fn save_atomic(active: &ActiveConfig, config_dir: &Path) -> Result<(), ConfigError> {
    let bytes = encode(active)?;
    atomic_write(config_path(config_dir), &bytes)
        .map_err(|error| ConfigError::at(CONFIG_FILE_NAME, format!("failed to save: {error}")))
}

pub(crate) struct SaveOutcome {
    pub(crate) durability_warning: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PersistStage {
    TemporaryCreate,
    Write,
    Flush,
    Replace,
    DirectorySync,
}

pub(crate) fn save_atomic_durable(
    active: &ActiveConfig,
    config_dir: &Path,
) -> Result<SaveOutcome, ConfigError> {
    let bytes = encode(active)?;
    atomic_write_durable_with(config_path(config_dir), &bytes, |_| Ok(()))
        .map_err(|error| ConfigError::at(CONFIG_FILE_NAME, format!("failed to save: {error}")))
}

#[cfg(test)]
pub(crate) fn save_atomic_durable_with_fault(
    active: &ActiveConfig,
    config_dir: &Path,
    failure: PersistStage,
) -> Result<SaveOutcome, ConfigError> {
    let bytes = encode(active)?;
    atomic_write_durable_with(config_path(config_dir), &bytes, |stage| {
        if stage == failure {
            Err(io::Error::other(format!("injected {stage:?} failure")))
        } else {
            Ok(())
        }
    })
    .map_err(|error| ConfigError::at(CONFIG_FILE_NAME, format!("failed to save: {error}")))
}

pub fn export(document: &ConfigDocument, path: &Path) -> Result<(), ConfigError> {
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| ConfigError::at("$", format!("failed to serialize config: {error}")))?;
    fs::write(path, bytes).map_err(|error| {
        ConfigError::at(
            path.display().to_string(),
            format!("failed to export: {error}"),
        )
    })
}

fn persist_migration(
    document: &ConfigDocument,
    original: &[u8],
    config_dir: &Path,
) -> Result<(), ConfigError> {
    let migrated = serde_json::to_vec_pretty(document)
        .map_err(|error| ConfigError::at("$", format!("failed to serialize migration: {error}")))?;
    fs::create_dir_all(config_dir).map_err(|error| {
        ConfigError::at(
            config_dir.display().to_string(),
            format!("failed to create config directory: {error}"),
        )
    })?;
    write_migration_backup(config_dir, original).map_err(|error| {
        ConfigError::at(
            config_dir.display().to_string(),
            format!("failed to write migration backup: {error}"),
        )
    })?;
    atomic_write(config_path(config_dir), &migrated).map_err(|error| {
        ConfigError::at(
            CONFIG_FILE_NAME,
            format!("failed to replace migrated config: {error}"),
        )
    })
}

fn write_migration_backup(config_dir: &Path, original: &[u8]) -> io::Result<PathBuf> {
    for suffix in 0_u32.. {
        let file_name = if suffix == 0 {
            format!("{MIGRATION_BACKUP_STEM}.json")
        } else {
            format!("{MIGRATION_BACKUP_STEM}.{suffix}.json")
        };
        let path = config_dir.join(file_name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(original)?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("u32 backup suffix space exhausted")
}

fn atomic_write(path: PathBuf, bytes: &[u8]) -> io::Result<()> {
    atomic_write_durable_with(path, bytes, |_| Ok(())).map(|_| ())
}

fn atomic_write_durable_with(
    path: PathBuf,
    bytes: &[u8],
    mut stage: impl FnMut(PersistStage) -> io::Result<()>,
) -> io::Result<SaveOutcome> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;
    fs::create_dir_all(directory)?;
    stage(PersistStage::TemporaryCreate)?;
    let (temporary_path, mut temporary) = create_temporary_file(directory)?;
    let result = (|| {
        stage(PersistStage::Write)?;
        temporary.write_all(bytes)?;
        stage(PersistStage::Flush)?;
        temporary.sync_all()?;
        drop(temporary);
        stage(PersistStage::Replace)?;
        replace_file(&temporary_path, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result?;
    let durability_warning = stage(PersistStage::DirectorySync)
        .and_then(|()| sync_directory(directory))
        .is_err();
    Ok(SaveOutcome { durability_warning })
}

fn create_temporary_file(directory: &Path) -> io::Result<(PathBuf, File)> {
    for suffix in 0_u32.. {
        let path = directory.join(format!(
            "{TEMPORARY_PREFIX}{}.{}{TEMPORARY_SUFFIX}",
            std::process::id(),
            suffix
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("u32 temporary suffix space exhausted")
}

pub(crate) fn cleanup_owned_temporary_files(directory: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(middle) = name
            .strip_prefix(TEMPORARY_PREFIX)
            .and_then(|name| name.strip_suffix(TEMPORARY_SUFFIX))
        else {
            continue;
        };
        let mut parts = middle.split('.');
        if parts.next().is_some_and(|part| part.parse::<u32>().is_ok())
            && parts.next().is_some_and(|part| part.parse::<u32>().is_ok())
            && parts.next().is_none()
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn sync_directory(_directory: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows directory metadata flush is not guaranteed",
    ))
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(not(windows))]
fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_migration_keeps_original_file_bytes_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let original = br#"{"bindings":{"default":[{"id":"bad","gesture":{"trigger":"right_click","sequence":["up"]},"action":{"type":"keyboard","keys":["unsupported"]}}]}}"#;
        fs::write(config_path(directory.path()), original).unwrap();

        let error = load(directory.path()).err().expect("migration must fail");
        assert_eq!(error.path(), "bindings.default[0].action.keys[0]");
        assert_eq!(fs::read(config_path(directory.path())).unwrap(), original);
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("backup")));
    }

    #[test]
    fn successful_migration_backs_up_then_atomically_replaces_active_file() {
        let directory = tempfile::tempdir().unwrap();
        let original = br#"{"enabled":false}"#;
        fs::write(config_path(directory.path()), original).unwrap();

        let LoadResult::Ready(active) = load(directory.path()).unwrap() else {
            panic!("existing config must be ready");
        };
        assert!(!active.enabled());
        assert_eq!(
            fs::read(directory.path().join("zero-gesture.config.v1.backup.json")).unwrap(),
            original
        );
        let persisted: ConfigDocument =
            serde_json::from_slice(&fs::read(config_path(directory.path())).unwrap()).unwrap();
        assert_eq!(persisted.schema_version, SCHEMA_VERSION);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_replace_keeps_original_file_bytes_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let path = config_path(directory.path());
        let original = b"original";
        fs::write(&path, original).unwrap();
        let _locked = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();

        assert!(atomic_write(path.clone(), b"replacement").is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn startup_cleanup_removes_only_owned_orphan_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let owned = directory.path().join(".zero-gesture.config.12.3.tmp");
        let unrelated = directory.path().join(".zero-gesture.config.notes.tmp");
        fs::write(&owned, b"orphan").unwrap();
        fs::write(&unrelated, b"keep").unwrap();

        cleanup_owned_temporary_files(directory.path()).unwrap();

        assert!(!owned.exists());
        assert!(unrelated.exists());
    }
}
