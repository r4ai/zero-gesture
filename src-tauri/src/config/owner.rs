use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::publication::{ConfigPublication, MAX_GENERATION};
use super::{ActiveConfig, ConfigSnapshotReader};

pub(crate) const MAX_CONFIG_BYTES: usize = 512 * 1024;
const CANDIDATE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedToken(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedConfig {
    pub(crate) token: PreparedToken,
    pub(crate) base_revision: u64,
    pub(crate) base_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppliedConfig {
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) durability_warning: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConfigOwnerStatus {
    pub(crate) available: bool,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) candidate_prepared: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigOwnerError {
    PayloadTooLarge,
    Busy,
    RevisionConflict,
    ValidationFailed,
    TokenMismatch,
    NoPreparedConfig,
    GenerationExhausted,
    PersistenceFailed,
}

struct Candidate {
    session: u64,
    token: PreparedToken,
    base_revision: u64,
    base_generation: u64,
    next_revision: u64,
    next_generation: u64,
    active: ActiveConfig,
    slot: usize,
    expires_at: Instant,
}

pub(crate) struct ConfigOwner {
    config_dir: PathBuf,
    active: Option<ActiveConfig>,
    revision: u64,
    generation: u64,
    publication: ConfigPublication,
    candidate: Option<Candidate>,
    next_token: u64,
    last_applied: Option<PreparedToken>,
}

impl ConfigOwner {
    pub(crate) fn startup(config_dir: &Path) -> (Self, Option<ActiveConfig>) {
        let active = match super::cleanup_owned_temporary_files(config_dir)
            .map_err(|error| {
                super::ConfigError::at(config_dir.display().to_string(), error.to_string())
            })
            .and_then(|()| super::load(config_dir))
        {
            Ok(super::LoadResult::Ready(active) | super::LoadResult::Missing(active)) => {
                Some(active)
            }
            Err(_) => None,
        };
        let initial = active.clone();
        let generation = u64::from(active.is_some());
        let publication =
            ConfigPublication::new(active.as_ref().map(ActiveConfig::runtime), generation);
        (
            Self {
                config_dir: config_dir.to_path_buf(),
                active,
                revision: generation,
                generation,
                publication,
                candidate: None,
                next_token: 1,
                last_applied: None,
            },
            initial,
        )
    }

    #[cfg(test)]
    fn from_active(config_dir: &Path, active: ActiveConfig) -> Self {
        let publication = ConfigPublication::new(Some(active.runtime()), 1);
        Self {
            config_dir: config_dir.to_path_buf(),
            active: Some(active),
            revision: 1,
            generation: 1,
            publication,
            candidate: None,
            next_token: 1,
            last_applied: None,
        }
    }

    pub(crate) fn reader(&self) -> ConfigSnapshotReader {
        self.publication.reader()
    }

    pub(crate) fn status(&mut self, now: Instant) -> ConfigOwnerStatus {
        self.expire(now);
        let published_generation = self
            .reader()
            .read()
            .map_or(0, |snapshot| snapshot.generation());
        debug_assert_eq!(published_generation, self.generation);
        ConfigOwnerStatus {
            available: self.active.is_some(),
            revision: self.revision,
            generation: self.generation,
            candidate_prepared: self.candidate.is_some(),
        }
    }

    pub(crate) fn current_bytes(
        &mut self,
        now: Instant,
    ) -> Result<(u64, u64, Option<Vec<u8>>), ConfigOwnerError> {
        self.expire(now);
        let bytes = self
            .active
            .as_ref()
            .map(super::encode)
            .transpose()
            .map_err(|_| ConfigOwnerError::ValidationFailed)?;
        if bytes
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAX_CONFIG_BYTES)
        {
            return Err(ConfigOwnerError::PayloadTooLarge);
        }
        Ok((self.revision, self.generation, bytes))
    }

    pub(crate) fn active(&self) -> Option<&ActiveConfig> {
        self.active.as_ref()
    }

    pub(crate) fn prepare(
        &mut self,
        session: u64,
        expected_revision: u64,
        bytes: &[u8],
        now: Instant,
    ) -> Result<PreparedConfig, ConfigOwnerError> {
        self.expire(now);
        if self.candidate.is_some() {
            return Err(ConfigOwnerError::Busy);
        }
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigOwnerError::PayloadTooLarge);
        }
        if expected_revision != self.revision {
            return Err(ConfigOwnerError::RevisionConflict);
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(ConfigOwnerError::GenerationExhausted)?;
        let next_generation = self
            .generation
            .checked_add(1)
            .filter(|generation| *generation <= MAX_GENERATION)
            .ok_or(ConfigOwnerError::GenerationExhausted)?;
        let next_token = self
            .next_token
            .checked_add(1)
            .ok_or(ConfigOwnerError::GenerationExhausted)?;
        let active =
            super::decode_and_compile(bytes).map_err(|_| ConfigOwnerError::ValidationFailed)?;
        if super::encode(&active)
            .map_err(|_| ConfigOwnerError::ValidationFailed)?
            .len()
            > MAX_CONFIG_BYTES
        {
            return Err(ConfigOwnerError::PayloadTooLarge);
        }
        let slot = self
            .publication
            .reserve(active.runtime())
            .map_err(|_| ConfigOwnerError::Busy)?;
        let token = PreparedToken(self.next_token);
        self.next_token = next_token;
        let prepared = PreparedConfig {
            token,
            base_revision: self.revision,
            base_generation: self.generation,
        };
        self.candidate = Some(Candidate {
            session,
            token,
            base_revision: self.revision,
            base_generation: self.generation,
            next_revision,
            next_generation,
            active,
            slot,
            expires_at: now + CANDIDATE_TTL,
        });
        Ok(prepared)
    }

    pub(crate) fn commit(
        &mut self,
        session: u64,
        token: PreparedToken,
        base_revision: u64,
        base_generation: u64,
        now: Instant,
    ) -> Result<AppliedConfig, ConfigOwnerError> {
        self.commit_with(
            session,
            token,
            base_revision,
            base_generation,
            now,
            super::save_atomic_durable,
        )
    }

    pub(crate) fn set_enabled(
        &mut self,
        session: u64,
        expected_revision: u64,
        enabled: bool,
        now: Instant,
    ) -> Result<AppliedConfig, ConfigOwnerError> {
        let mut document = self
            .active
            .as_ref()
            .ok_or(ConfigOwnerError::ValidationFailed)?
            .document()
            .clone();
        document.shared.enabled = enabled;
        let bytes =
            serde_json::to_vec(&document).map_err(|_| ConfigOwnerError::ValidationFailed)?;
        let prepared = self.prepare(session, expected_revision, &bytes, now)?;
        self.commit(
            session,
            prepared.token,
            prepared.base_revision,
            prepared.base_generation,
            now,
        )
    }

    fn commit_with(
        &mut self,
        session: u64,
        token: PreparedToken,
        base_revision: u64,
        base_generation: u64,
        now: Instant,
        persist: impl FnOnce(&ActiveConfig, &Path) -> Result<super::SaveOutcome, super::ConfigError>,
    ) -> Result<AppliedConfig, ConfigOwnerError> {
        self.expire(now);
        let Some(candidate) = self.candidate.as_ref() else {
            return Err(if self.last_applied == Some(token) {
                ConfigOwnerError::TokenMismatch
            } else {
                ConfigOwnerError::NoPreparedConfig
            });
        };
        if candidate.session != session
            || candidate.token != token
            || candidate.base_revision != base_revision
            || candidate.base_generation != base_generation
        {
            return Err(ConfigOwnerError::TokenMismatch);
        }
        let outcome = match persist(&candidate.active, &self.config_dir) {
            Ok(outcome) => outcome,
            Err(_) => {
                let candidate = self.candidate.take().expect("candidate exists");
                self.publication.abort(candidate.slot);
                return Err(ConfigOwnerError::PersistenceFailed);
            }
        };

        let candidate = self.candidate.take().expect("candidate exists");
        self.publication
            .publish(candidate.slot, candidate.next_generation);
        self.revision = candidate.next_revision;
        self.generation = candidate.next_generation;
        self.active = Some(candidate.active);
        self.last_applied = Some(candidate.token);
        Ok(AppliedConfig {
            revision: self.revision,
            generation: self.generation,
            durability_warning: outcome.durability_warning,
        })
    }

    pub(crate) fn disconnect(&mut self, session: u64) {
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| candidate.session == session)
        {
            let candidate = self.candidate.take().expect("candidate exists");
            self.publication.abort(candidate.slot);
        }
    }

    fn expire(&mut self, now: Instant) {
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| now >= candidate.expires_at)
        {
            let candidate = self.candidate.take().expect("candidate exists");
            self.publication.abort(candidate.slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigDocument;
    use std::fs;

    fn owner(directory: &Path) -> ConfigOwner {
        ConfigOwner::from_active(
            directory,
            ActiveConfig::from_document(ConfigDocument::default()).unwrap(),
        )
    }

    fn changed_bytes() -> Vec<u8> {
        let mut document = ConfigDocument::default();
        document.shared.appearance.trail_thickness = 7.0;
        serde_json::to_vec(&document).unwrap()
    }

    #[test]
    fn prepare_does_not_change_active_config_or_disk() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let before = owner.current_bytes(now).unwrap();
        let reader = owner.reader();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        assert_eq!(owner.current_bytes(now).unwrap(), before);
        assert_eq!(reader.read().unwrap().generation(), 1);
        assert!(!directory
            .path()
            .join(super::super::CONFIG_FILE_NAME)
            .exists());
        assert_eq!(prepared.base_generation, 1);
    }

    #[test]
    fn prepare_rejects_a_second_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        assert_eq!(
            owner.prepare(1, 1, &changed_bytes(), now),
            Err(ConfigOwnerError::Busy)
        );
    }

    #[test]
    fn stale_revision_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        assert_eq!(
            owner.prepare(1, 0, &changed_bytes(), now),
            Err(ConfigOwnerError::RevisionConflict)
        );
    }

    #[test]
    fn mismatched_connection_is_rejected_without_consuming_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        assert_eq!(
            owner.commit(2, prepared.token, 1, 1, now),
            Err(ConfigOwnerError::TokenMismatch)
        );
        assert!(owner.status(now).candidate_prepared);
    }

    #[test]
    fn mismatched_token_is_rejected_without_consuming_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        assert_eq!(
            owner.commit(1, PreparedToken(prepared.token.0 + 1), 1, 1, now),
            Err(ConfigOwnerError::TokenMismatch)
        );
        assert!(owner.status(now).candidate_prepared);
    }

    #[test]
    fn mismatched_base_revision_is_rejected_without_consuming_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        assert_eq!(
            owner.commit(1, prepared.token, 0, 1, now),
            Err(ConfigOwnerError::TokenMismatch)
        );
        assert!(owner.status(now).candidate_prepared);
    }

    #[test]
    fn mismatched_base_generation_is_rejected_without_consuming_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        owner
            .commit(1, prepared.token, prepared.base_revision, 0, now)
            .expect_err("mismatched generation must fail");
        assert!(owner.status(now).candidate_prepared);
    }

    #[test]
    fn disconnect_releases_the_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        owner.prepare(7, 1, &changed_bytes(), now).unwrap();
        owner.disconnect(7);
        assert!(!owner.status(now).candidate_prepared);
    }

    #[test]
    fn timeout_releases_the_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        owner.prepare(8, 1, &changed_bytes(), now).unwrap();
        assert!(!owner.status(now + CANDIDATE_TTL).candidate_prepared);
    }

    #[test]
    fn validation_and_compile_failure_leave_active_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let before = owner.current_bytes(Instant::now()).unwrap();
        assert_eq!(
            owner.prepare(1, 1, br#"{"schema_version":2}"#, Instant::now()),
            Err(ConfigOwnerError::ValidationFailed)
        );
        assert_eq!(owner.current_bytes(Instant::now()).unwrap(), before);
    }

    #[test]
    fn prepare_migrates_legacy_without_persisting() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let legacy = br#"{"enabled":false,"bindings":{"default":[]}}"#;
        owner.prepare(1, 1, legacy, Instant::now()).unwrap();
        assert!(!directory
            .path()
            .join(super::super::CONFIG_FILE_NAME)
            .exists());
        assert_eq!(owner.status(Instant::now()).revision, 1);
    }

    #[test]
    fn persistence_failure_aborts_without_publication() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let reader = owner.reader();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        let error = owner.commit_with(
            1,
            prepared.token,
            prepared.base_revision,
            prepared.base_generation,
            now,
            |_, _| Err(super::super::ConfigError::at("$", "injected")),
        );
        assert_eq!(error, Err(ConfigOwnerError::PersistenceFailed));
        assert_eq!(reader.read().unwrap().generation(), 1);
        assert_eq!(owner.status(now).revision, 1);
    }

    fn assert_pre_replace_failure(failure: super::super::PersistStage) {
        let directory = tempfile::tempdir().unwrap();
        let original =
            super::super::encode(&ActiveConfig::from_document(ConfigDocument::default()).unwrap())
                .unwrap();
        fs::write(
            directory.path().join(super::super::CONFIG_FILE_NAME),
            &original,
        )
        .unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let reader = owner.reader();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        let result = owner.commit_with(
            1,
            prepared.token,
            prepared.base_revision,
            prepared.base_generation,
            now,
            |active, config_dir| {
                super::super::save_atomic_durable_with_fault(active, config_dir, failure)
            },
        );
        assert_eq!(result, Err(ConfigOwnerError::PersistenceFailed));
        assert_eq!(
            fs::read(directory.path().join(super::super::CONFIG_FILE_NAME)).unwrap(),
            original
        );
        assert_eq!(reader.read().unwrap().generation(), 1);
    }

    #[test]
    fn temporary_create_failure_keeps_disk_and_snapshot_unchanged() {
        assert_pre_replace_failure(super::super::PersistStage::TemporaryCreate);
    }

    #[test]
    fn temporary_write_failure_keeps_disk_and_snapshot_unchanged() {
        assert_pre_replace_failure(super::super::PersistStage::Write);
    }

    #[test]
    fn temporary_flush_failure_keeps_disk_and_snapshot_unchanged() {
        assert_pre_replace_failure(super::super::PersistStage::Flush);
    }

    #[test]
    fn atomic_replace_failure_keeps_disk_and_snapshot_unchanged() {
        assert_pre_replace_failure(super::super::PersistStage::Replace);
    }

    #[test]
    fn post_replace_directory_sync_failure_still_publishes() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let reader = owner.reader();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        let applied = owner
            .commit_with(
                1,
                prepared.token,
                prepared.base_revision,
                prepared.base_generation,
                now,
                |active, config_dir| {
                    super::super::save_atomic_durable_with_fault(
                        active,
                        config_dir,
                        super::super::PersistStage::DirectorySync,
                    )
                },
            )
            .unwrap();
        assert!(applied.durability_warning);
        assert_eq!(reader.read().unwrap().generation(), 2);
    }

    #[test]
    fn atomic_replacement_commit_publishes_exact_applied_revision_and_generation() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let reader = owner.reader();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        let applied = owner
            .commit(
                1,
                prepared.token,
                prepared.base_revision,
                prepared.base_generation,
                now,
            )
            .unwrap();
        assert_eq!((applied.revision, applied.generation), (2, 2));
        assert_eq!(reader.read().unwrap().generation(), 2);
        assert!(
            fs::read(directory.path().join(super::super::CONFIG_FILE_NAME))
                .unwrap()
                .windows(b"7.0".len())
                .any(|window| window == b"7.0")
        );
    }

    #[test]
    fn duplicate_applied_token_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        let now = Instant::now();
        let prepared = owner.prepare(1, 1, &changed_bytes(), now).unwrap();
        owner
            .commit(
                1,
                prepared.token,
                prepared.base_revision,
                prepared.base_generation,
                now,
            )
            .unwrap();
        assert_eq!(
            owner.commit(
                1,
                prepared.token,
                prepared.base_revision,
                prepared.base_generation,
                now
            ),
            Err(ConfigOwnerError::TokenMismatch)
        );
    }

    #[test]
    fn startup_loads_persisted_config_as_generation_one() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(super::super::CONFIG_FILE_NAME),
            changed_bytes(),
        )
        .unwrap();
        let (mut owner, initial) = ConfigOwner::startup(directory.path());
        assert_eq!(owner.status(Instant::now()).generation, 1);
        assert_eq!(
            initial
                .unwrap()
                .document()
                .shared
                .appearance
                .trail_thickness,
            7.0
        );
    }

    #[test]
    fn generation_exhaustion_rejects_prepare_without_wrap() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner = owner(directory.path());
        owner.generation = MAX_GENERATION;
        assert_eq!(
            owner.prepare(1, 1, &changed_bytes(), Instant::now()),
            Err(ConfigOwnerError::GenerationExhausted)
        );
        assert!(owner.candidate.is_none());
    }
}
