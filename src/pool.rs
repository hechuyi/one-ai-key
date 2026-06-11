use crate::credentials::{
    Credential, CredentialFingerprint, CredentialId, CredentialSnapshot, CredentialSource,
};
#[cfg(test)]
use std::time::Duration;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    time::Instant,
};

#[derive(Debug, Clone)]
pub struct KeyPoolConfig {
    pub name: String,
    pub credential_namespace: String,
    pub api_base: String,
    pub credentials: Vec<PoolCredentialInput>,
}

#[derive(Debug, Clone)]
pub struct PoolCredentialInput {
    pub secret: String,
    pub source: CredentialSource,
}

#[derive(Debug, Clone)]
pub struct SelectedKey {
    pub api_base: String,
    pub key: String,
    pub credential_id: CredentialId,
    pub credential_fingerprint: CredentialFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialCandidate {
    pub credential_id: CredentialId,
    pub fingerprint: CredentialFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPoolSnapshot {
    pub current_index: usize,
    pub total_credentials: usize,
    pub available_credentials: usize,
    pub cooling_down_credentials: usize,
    pub expired_credentials: usize,
    pub quota_exhausted_credentials: usize,
    pub disabled_credentials: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSnapshotFilter {
    All,
    Available,
    CoolingDown,
    Expired,
    QuotaExhausted,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialSnapshotPage {
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub credentials: Vec<CredentialSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchOutcome {
    Switched,
    NoAlternative,
    StaleFailure,
}

#[derive(Debug)]
pub struct KeyPool {
    config: KeyPoolConfig,
    credentials: Vec<Credential>,
    credential_index: HashMap<CredentialId, usize>,
    available_indices: BTreeSet<usize>,
    cooldown_deadlines: BTreeMap<Instant, BTreeSet<usize>>,
    cooldown_deadline_by_index: HashMap<usize, Instant>,
    current: usize,
}

impl KeyPool {
    pub fn new(config: KeyPoolConfig) -> anyhow::Result<Self> {
        if config.credentials.is_empty() {
            anyhow::bail!("pool {} has no keys", config.name);
        }

        let credentials: Vec<Credential> = config
            .credentials
            .iter()
            .cloned()
            .map(|input| {
                Credential::with_source(&config.credential_namespace, input.secret, input.source)
            })
            .collect();
        let credential_index = credentials
            .iter()
            .enumerate()
            .map(|(index, credential)| (credential.id().clone(), index))
            .collect();
        let available_indices = (0..credentials.len()).collect();

        Ok(Self {
            config,
            credentials,
            credential_index,
            available_indices,
            cooldown_deadlines: BTreeMap::new(),
            cooldown_deadline_by_index: HashMap::new(),
            current: 0,
        })
    }

    pub fn select(&mut self) -> anyhow::Result<SelectedKey> {
        let index = self.next_available_from(self.current).ok_or_else(|| {
            anyhow::anyhow!("pool {} has no available credentials", self.config.name)
        })?;
        self.current = index;
        Ok(self.selected_key_at(index))
    }

    pub fn select_credential_by_id(
        &mut self,
        credential_id: &CredentialId,
    ) -> anyhow::Result<SelectedKey> {
        let index = self.index_for_id(credential_id).ok_or_else(|| {
            anyhow::anyhow!(
                "pool {} has no credential {}",
                self.config.name,
                credential_id.0
            )
        })?;
        self.refresh_available_indices(Instant::now());
        if !self.available_indices.contains(&index) {
            anyhow::bail!(
                "credential {} in pool {} is not available",
                credential_id.0,
                self.config.name
            );
        }
        self.current = index;
        Ok(self.selected_key_at(index))
    }

    pub fn credential_key_by_id(&self, credential_id: &CredentialId) -> Option<SelectedKey> {
        let index = self.index_for_id(credential_id)?;
        Some(self.selected_key_at(index))
    }

    pub fn has_available_credentials_read_only(&self) -> bool {
        if !self.available_indices.is_empty() {
            return true;
        }

        self.cooldown_deadlines
            .first_key_value()
            .is_some_and(|(deadline, _)| *deadline <= Instant::now())
    }

    pub fn has_cooling_down_credentials_read_only(&self) -> bool {
        let now = Instant::now();
        self.cooldown_deadlines
            .keys()
            .next_back()
            .is_some_and(|deadline| *deadline > now)
    }

    pub fn add_credentials(
        &mut self,
        credential_inputs: Vec<PoolCredentialInput>,
    ) -> anyhow::Result<Vec<CredentialSnapshot>> {
        let credentials: Vec<Credential> = credential_inputs
            .into_iter()
            .map(|input| {
                Credential::with_source(
                    &self.config.credential_namespace,
                    input.secret,
                    input.source,
                )
            })
            .collect();
        let mut new_ids = HashSet::new();
        for credential in &credentials {
            if self.credential_index.contains_key(credential.id())
                || !new_ids.insert(credential.id().clone())
            {
                anyhow::bail!(
                    "credential {} already exists in pool {}",
                    credential.id().0,
                    self.config.name
                );
            }
        }

        let mut snapshots = Vec::with_capacity(credentials.len());
        for credential in credentials {
            let index = self.credentials.len();
            snapshots.push(credential.snapshot());
            self.credential_index.insert(credential.id().clone(), index);
            self.credentials.push(credential);
            self.available_indices.insert(index);
        }
        Ok(snapshots)
    }

    pub fn select_cooling_down_last_resort(&mut self) -> anyhow::Result<SelectedKey> {
        self.refresh_available_indices(Instant::now());
        if let Some(index) = self.next_available_from(self.current) {
            self.current = index;
            return Ok(self.selected_key_at(index));
        }

        let now = Instant::now();
        let start = self.current;
        let indices = (start..self.credentials.len()).chain(0..start);
        for index in indices {
            if self
                .cooldown_deadline_by_index
                .get(&index)
                .is_some_and(|deadline| *deadline > now)
            {
                self.current = index;
                return Ok(self.selected_key_at(index));
            }
        }

        anyhow::bail!(
            "pool {} has no cooling-down credentials for last-resort selection",
            self.config.name
        )
    }

    pub fn contains_secret(&self, secret: &str) -> bool {
        let credential = Credential::with_source(
            &self.config.credential_namespace,
            secret.to_string(),
            CredentialSource {
                source_path: None,
                source_line: None,
                batch_id: None,
            },
        );
        self.credential_index.contains_key(credential.id())
    }

    fn selected_key_at(&self, index: usize) -> SelectedKey {
        let _source_context = &self.credentials[index].source;
        SelectedKey {
            api_base: self.config.api_base.clone(),
            key: self.credentials[index].secret().to_string(),
            credential_id: self.credentials[index].id().clone(),
            credential_fingerprint: self.credentials[index].fingerprint().clone(),
        }
    }

    pub fn retry_candidates_from_current(&mut self, limit: usize) -> Vec<CredentialCandidate> {
        if limit == 0 || self.credentials.is_empty() {
            return Vec::new();
        }
        self.refresh_available_indices(Instant::now());

        let mut candidates = Vec::new();
        let start = (self.current + 1) % self.credentials.len();
        for index in self.available_indices_from(start) {
            if candidates.len() >= limit {
                break;
            }
            if index == self.current {
                continue;
            }
            let credential = &self.credentials[index];
            candidates.push(CredentialCandidate {
                credential_id: credential.id().clone(),
                fingerprint: credential.fingerprint().clone(),
            });
        }
        candidates
    }

    #[cfg(test)]
    fn report_switchable_failure_with_cooldown(
        &mut self,
        failed_index: usize,
        cooldown: Duration,
    ) -> SwitchOutcome {
        self.report_switchable_failure_until(
            failed_index,
            Instant::now() + cooldown,
            "switchable upstream failure",
        )
    }

    fn report_switchable_failure_until(
        &mut self,
        failed_index: usize,
        until: Instant,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        if failed_index >= self.credentials.len() {
            return SwitchOutcome::StaleFailure;
        }
        if self.credentials[failed_index].is_disabled() {
            return SwitchOutcome::StaleFailure;
        }
        if failed_index != self.current {
            return SwitchOutcome::StaleFailure;
        }

        self.credentials[failed_index].mark_cooling_down(until, reason);
        self.mark_index_cooling_down(failed_index, until);

        if let Some(next) = self.next_available_from((self.current + 1) % self.credentials.len()) {
            if next != failed_index {
                self.current = next;
                return SwitchOutcome::Switched;
            }
        }

        SwitchOutcome::NoAlternative
    }

    #[cfg(test)]
    pub fn report_switchable_failure_by_id_with_cooldown(
        &mut self,
        credential_id: &CredentialId,
        cooldown: Duration,
    ) -> SwitchOutcome {
        match self.index_for_id(credential_id) {
            Some(index) => self.report_switchable_failure_with_cooldown(index, cooldown),
            None => SwitchOutcome::StaleFailure,
        }
    }

    pub fn apply_credential_cooldown_until(
        &mut self,
        credential_id: &CredentialId,
        until: Instant,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        match self.index_for_id(credential_id) {
            Some(index) => self.report_switchable_failure_until(index, until, reason),
            None => SwitchOutcome::StaleFailure,
        }
    }

    fn report_expired_failure(
        &mut self,
        failed_index: usize,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        if failed_index >= self.credentials.len() {
            return SwitchOutcome::StaleFailure;
        }
        if self.credentials[failed_index].is_disabled() {
            return SwitchOutcome::StaleFailure;
        }
        self.credentials[failed_index].mark_expired(reason);
        self.mark_index_unavailable(failed_index);

        if failed_index != self.current {
            return SwitchOutcome::StaleFailure;
        }

        if let Some(next) = self.next_available_from((self.current + 1) % self.credentials.len()) {
            if next != failed_index {
                self.current = next;
                return SwitchOutcome::Switched;
            }
        }

        SwitchOutcome::NoAlternative
    }

    pub fn report_expired_failure_by_id(
        &mut self,
        credential_id: &CredentialId,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        match self.index_for_id(credential_id) {
            Some(index) => self.report_expired_failure(index, reason),
            None => SwitchOutcome::StaleFailure,
        }
    }

    pub fn apply_credential_expired(
        &mut self,
        credential_id: &CredentialId,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        self.report_expired_failure_by_id(credential_id, reason)
    }

    pub fn apply_credential_quota_exhausted(
        &mut self,
        credential_id: &CredentialId,
        reason: impl Into<String>,
    ) -> SwitchOutcome {
        match self.index_for_id(credential_id) {
            Some(index) => {
                if self.credentials[index].is_disabled() {
                    return SwitchOutcome::StaleFailure;
                }
                self.credentials[index].mark_quota_exhausted(reason);
                self.mark_index_unavailable(index);
                if index == self.current {
                    if let Some(next) =
                        self.next_available_from((self.current + 1) % self.credentials.len())
                    {
                        if next != index {
                            self.current = next;
                            return SwitchOutcome::Switched;
                        }
                    }
                    SwitchOutcome::NoAlternative
                } else {
                    SwitchOutcome::StaleFailure
                }
            }
            None => SwitchOutcome::StaleFailure,
        }
    }

    #[cfg(test)]
    fn report_keep_key_failure(&mut self, failed_index: usize) -> bool {
        failed_index == self.current
    }

    #[cfg(test)]
    pub fn report_keep_key_failure_by_id(&mut self, credential_id: &CredentialId) -> bool {
        self.index_for_id(credential_id)
            .is_some_and(|index| self.report_keep_key_failure(index))
    }

    pub fn expire_credential_by_id(
        &mut self,
        credential_id: &CredentialId,
        reason: impl Into<String>,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        self.report_expired_failure(index, reason);
        Some(self.credentials[index].snapshot())
    }

    pub fn credential_snapshot_by_id(
        &self,
        credential_id: &CredentialId,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        Some(self.credentials[index].snapshot())
    }

    pub fn restore_credential_by_id(
        &mut self,
        credential_id: &CredentialId,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        if self.credentials[index].is_expired() || self.credentials[index].is_quota_exhausted() {
            self.credentials[index].mark_available();
            self.mark_index_available(index);
        }
        Some(self.credentials[index].snapshot())
    }

    pub fn disable_credential_by_id(
        &mut self,
        credential_id: &CredentialId,
        reason: impl Into<String>,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        self.credentials[index].mark_disabled(reason);
        self.mark_index_unavailable(index);
        if index == self.current {
            if let Some(next) =
                self.next_available_from((self.current + 1) % self.credentials.len())
            {
                self.current = next;
            }
        }
        Some(self.credentials[index].snapshot())
    }

    pub fn enable_credential_by_id(
        &mut self,
        credential_id: &CredentialId,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        self.credentials[index].mark_available();
        self.mark_index_available(index);
        Some(self.credentials[index].snapshot())
    }

    pub fn clear_credential_cooldown_by_id(
        &mut self,
        credential_id: &CredentialId,
    ) -> Option<CredentialSnapshot> {
        let index = self.index_for_id(credential_id)?;
        if self.credentials[index].is_cooling_down() {
            self.credentials[index].mark_available();
            self.mark_index_available(index);
        }
        Some(self.credentials[index].snapshot())
    }

    fn index_for_id(&self, credential_id: &CredentialId) -> Option<usize> {
        self.credential_index.get(credential_id).copied()
    }

    #[cfg(test)]
    pub fn credential_snapshots(&self) -> Vec<CredentialSnapshot> {
        self.credentials.iter().map(Credential::snapshot).collect()
    }

    pub fn credential_snapshot_page(
        &self,
        filter: CredentialSnapshotFilter,
        offset: usize,
        limit: usize,
    ) -> CredentialSnapshotPage {
        let mut filtered_credentials = 0;
        let mut credentials = Vec::new();
        for credential in &self.credentials {
            let snapshot = credential.snapshot();
            if !credential_snapshot_matches_filter(&snapshot, filter) {
                continue;
            }
            if filtered_credentials >= offset && credentials.len() < limit {
                credentials.push(snapshot);
            }
            filtered_credentials += 1;
        }
        CredentialSnapshotPage {
            total_credentials: self.credentials.len(),
            filtered_credentials,
            credentials,
        }
    }

    pub fn snapshot(&self) -> KeyPoolSnapshot {
        let now = Instant::now();
        let mut available_credentials = 0;
        let mut cooling_down_credentials = 0;
        let mut expired_credentials = 0;
        let mut quota_exhausted_credentials = 0;
        let mut disabled_credentials = 0;
        for credential in &self.credentials {
            if credential.is_available_at(now) {
                available_credentials += 1;
            } else if credential.is_cooling_down_at(now) {
                cooling_down_credentials += 1;
            } else if credential.is_expired() {
                expired_credentials += 1;
            } else if credential.is_quota_exhausted() {
                quota_exhausted_credentials += 1;
            } else if credential.is_disabled() {
                disabled_credentials += 1;
            }
        }
        KeyPoolSnapshot {
            current_index: self.current,
            total_credentials: self.credentials.len(),
            available_credentials,
            cooling_down_credentials,
            expired_credentials,
            quota_exhausted_credentials,
            disabled_credentials,
        }
    }

    fn next_available_from(&mut self, start: usize) -> Option<usize> {
        self.refresh_available_indices(Instant::now());
        self.available_indices_from(start).next()
    }

    fn available_indices_from(&self, start: usize) -> impl Iterator<Item = usize> + '_ {
        self.available_indices
            .range(start..)
            .chain(self.available_indices.range(..start))
            .copied()
    }

    fn refresh_available_indices(&mut self, now: Instant) {
        let expired_deadlines: Vec<Instant> = self
            .cooldown_deadlines
            .range(..=now)
            .map(|(deadline, _)| *deadline)
            .collect();

        for deadline in expired_deadlines {
            let Some(indices) = self.cooldown_deadlines.remove(&deadline) else {
                continue;
            };
            for index in indices {
                if self.cooldown_deadline_by_index.get(&index) != Some(&deadline) {
                    continue;
                }
                self.cooldown_deadline_by_index.remove(&index);
                if self.credentials[index].is_available_at(now) {
                    self.available_indices.insert(index);
                }
            }
        }
    }

    fn mark_index_available(&mut self, index: usize) {
        self.remove_cooldown_deadline(index);
        self.available_indices.insert(index);
    }

    fn mark_index_unavailable(&mut self, index: usize) {
        self.remove_cooldown_deadline(index);
        self.available_indices.remove(&index);
    }

    fn mark_index_cooling_down(&mut self, index: usize, until: Instant) {
        self.remove_cooldown_deadline(index);
        self.available_indices.remove(&index);
        self.cooldown_deadlines
            .entry(until)
            .or_default()
            .insert(index);
        self.cooldown_deadline_by_index.insert(index, until);
    }

    fn remove_cooldown_deadline(&mut self, index: usize) {
        let Some(deadline) = self.cooldown_deadline_by_index.remove(&index) else {
            return;
        };
        let Some(indices) = self.cooldown_deadlines.get_mut(&deadline) else {
            return;
        };
        indices.remove(&index);
        if indices.is_empty() {
            self.cooldown_deadlines.remove(&deadline);
        }
    }
}

fn credential_snapshot_matches_filter(
    credential: &CredentialSnapshot,
    filter: CredentialSnapshotFilter,
) -> bool {
    matches!(filter, CredentialSnapshotFilter::All)
        || matches!(
            (&credential.state, filter),
            (
                crate::credentials::CredentialStateSnapshot::Available,
                CredentialSnapshotFilter::Available
            ) | (
                crate::credentials::CredentialStateSnapshot::CoolingDown { .. },
                CredentialSnapshotFilter::CoolingDown
            ) | (
                crate::credentials::CredentialStateSnapshot::Expired { .. },
                CredentialSnapshotFilter::Expired
            ) | (
                crate::credentials::CredentialStateSnapshot::QuotaExhausted { .. },
                CredentialSnapshotFilter::QuotaExhausted
            ) | (
                crate::credentials::CredentialStateSnapshot::Disabled { .. },
                CredentialSnapshotFilter::Disabled
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn imported(secret: &str) -> PoolCredentialInput {
        PoolCredentialInput {
            secret: secret.to_string(),
            source: crate::credentials::CredentialSource::unknown(),
        }
    }

    fn pool() -> KeyPool {
        KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1"), imported("k2"), imported("k3")],
        })
        .unwrap()
    }

    #[test]
    fn select_keeps_same_key_until_failure() {
        let p = pool();
        let mut p = p;
        assert_eq!(p.select().unwrap().key, "k1");
        assert_eq!(p.select().unwrap().key, "k1");
    }

    #[test]
    fn selected_key_exposes_stable_credential_identity() {
        let mut p = pool();
        let first = p.select().unwrap();
        let second = p.select().unwrap();

        assert_eq!(first.credential_id, second.credential_id);
        assert_eq!(first.credential_fingerprint, second.credential_fingerprint);
        assert_ne!(first.credential_id.0.as_str(), first.key);
        assert_ne!(first.credential_fingerprint.0.as_str(), first.key);
    }

    #[test]
    fn add_credentials_inserts_available_credentials_without_moving_current() {
        let mut p = pool();
        let selected = p.select().unwrap();
        let added = p
            .add_credentials(vec![imported("k4"), imported("k5")])
            .expect("new credentials should be inserted");

        assert_eq!(p.snapshot().total_credentials, 5);
        assert_eq!(p.snapshot().available_credentials, 5);
        assert_eq!(p.snapshot().current_index, 0);
        assert_eq!(p.select().unwrap().credential_id, selected.credential_id);
        assert_eq!(
            added[0].state,
            crate::credentials::CredentialStateSnapshot::Available
        );
    }

    #[test]
    fn add_credentials_rejects_existing_credential_id_without_partial_apply() {
        let mut p = pool();
        let result = p.add_credentials(vec![imported("k4"), imported("k1")]);
        assert!(result.is_err());
        assert_eq!(p.snapshot().total_credentials, 3);
    }

    #[test]
    fn retry_candidates_are_bounded_and_do_not_expose_raw_secret() {
        let mut p = pool();
        let first = p.select().unwrap();
        let candidates = p.retry_candidates_from_current(1);

        assert_eq!(candidates.len(), 1);
        assert_ne!(candidates[0].credential_id, first.credential_id);
        assert_ne!(candidates[0].fingerprint.0.as_str(), "k2");

        let all_candidates = p.retry_candidates_from_current(10);
        assert_eq!(all_candidates.len(), 2);
    }

    #[test]
    fn credential_snapshot_page_filters_before_collecting_page() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "test expired"),
            SwitchOutcome::Switched
        );

        let page = p.credential_snapshot_page(CredentialSnapshotFilter::Available, 1, 1);

        assert_eq!(page.total_credentials, 3);
        assert_eq!(page.filtered_credentials, 2);
        assert_eq!(page.credentials.len(), 1);
        assert!(matches!(
            page.credentials[0].state,
            crate::credentials::CredentialStateSnapshot::Available
        ));
    }

    #[test]
    fn switchable_failure_advances_to_next_key() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::Switched
        );
        assert_eq!(p.select().unwrap().key, "k2");
    }

    #[test]
    fn switchable_failure_can_be_reported_by_stable_credential_id() {
        let mut p = pool();
        let first = p.select().unwrap();

        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::Switched
        );
        assert_eq!(p.select().unwrap().key, "k2");
    }

    #[test]
    fn expired_failure_can_be_reported_by_stable_credential_id() {
        let mut p = pool();
        let first = p.select().unwrap();

        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "invalid_api_key"),
            SwitchOutcome::Switched
        );
        assert_eq!(p.select().unwrap().key, "k2");
        assert_eq!(p.snapshot().expired_credentials, 1);
    }

    #[test]
    fn keep_failure_can_be_reported_by_stable_credential_id() {
        let mut p = pool();
        let first = p.select().unwrap();

        assert!(p.report_keep_key_failure_by_id(&first.credential_id));
        assert_eq!(p.select().unwrap().key, "k1");
    }

    #[test]
    fn expired_credential_can_be_restored_by_stable_credential_id() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "manual"),
            SwitchOutcome::Switched
        );

        let restored = p.restore_credential_by_id(&first.credential_id).unwrap();

        assert!(matches!(
            restored.state,
            crate::credentials::CredentialStateSnapshot::Available
        ));
        assert_eq!(p.snapshot().expired_credentials, 0);
        assert_eq!(p.snapshot().available_credentials, 3);
    }

    #[test]
    fn cooling_down_credential_can_be_cleared_by_stable_credential_id() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(60)
            ),
            SwitchOutcome::Switched
        );
        assert_eq!(p.snapshot().cooling_down_credentials, 1);

        let cleared = p
            .clear_credential_cooldown_by_id(&first.credential_id)
            .unwrap();

        assert!(matches!(
            cleared.state,
            crate::credentials::CredentialStateSnapshot::Available
        ));
        assert_eq!(p.snapshot().cooling_down_credentials, 0);
        assert_eq!(p.snapshot().available_credentials, 3);
    }

    #[test]
    fn clear_cooldown_does_not_restore_expired_credential() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "invalid"),
            SwitchOutcome::Switched
        );

        let unchanged = p
            .clear_credential_cooldown_by_id(&first.credential_id)
            .unwrap();

        assert_eq!(
            unchanged.state,
            crate::credentials::CredentialStateSnapshot::Expired {
                reason: "invalid".to_string()
            }
        );
        assert_eq!(p.snapshot().expired_credentials, 1);
    }

    #[test]
    fn disabled_current_credential_is_skipped_until_enabled() {
        let mut p = pool();
        let first = p.select().unwrap();

        let disabled = p
            .disable_credential_by_id(&first.credential_id, "manual disable")
            .unwrap();

        assert_eq!(
            disabled.state,
            crate::credentials::CredentialStateSnapshot::Disabled {
                reason: "manual disable".to_string()
            }
        );
        assert_eq!(p.select().unwrap().key, "k2");
        assert_eq!(p.snapshot().disabled_credentials, 1);
        assert_eq!(p.snapshot().available_credentials, 2);

        let enabled = p.enable_credential_by_id(&first.credential_id).unwrap();

        assert!(matches!(
            enabled.state,
            crate::credentials::CredentialStateSnapshot::Available
        ));
        assert_eq!(p.snapshot().disabled_credentials, 0);
        assert_eq!(p.snapshot().available_credentials, 3);
    }

    #[test]
    fn credential_key_lookup_does_not_mutate_sticky_selection_or_require_availability() {
        let mut p = pool();
        let first = p.select().unwrap();
        let second = p
            .retry_candidates_from_current(1)
            .into_iter()
            .next()
            .unwrap();
        p.disable_credential_by_id(&second.credential_id, "manual disable")
            .unwrap();

        let selected = p.credential_key_by_id(&second.credential_id).unwrap();

        assert_eq!(selected.key, "k2");
        assert_eq!(selected.credential_id, second.credential_id);
        assert_eq!(p.select().unwrap().credential_id, first.credential_id);
    }

    #[test]
    fn disabled_cooling_down_credential_is_not_reenabled_by_stale_deadline() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_millis(1)
            ),
            SwitchOutcome::Switched
        );

        p.disable_credential_by_id(&first.credential_id, "manual disable")
            .unwrap();
        std::thread::sleep(Duration::from_millis(5));

        assert!(p.select_credential_by_id(&first.credential_id).is_err());
        assert_eq!(p.snapshot().disabled_credentials, 1);
        assert_eq!(p.snapshot().available_credentials, 2);
    }

    #[test]
    fn expired_failure_does_not_override_disabled_credential() {
        let mut p = pool();
        let first = p.select().unwrap();
        p.disable_credential_by_id(&first.credential_id, "manual disable")
            .unwrap();

        assert_eq!(
            p.apply_credential_expired(&first.credential_id, "invalid_api_key"),
            SwitchOutcome::StaleFailure
        );
        let snapshot = p.credential_snapshot_by_id(&first.credential_id).unwrap();
        assert_eq!(
            snapshot.state,
            crate::credentials::CredentialStateSnapshot::Disabled {
                reason: "manual disable".to_string()
            }
        );
    }

    #[test]
    fn switchable_failure_does_not_override_disabled_credential() {
        let mut p = pool();
        let first = p.select().unwrap();
        p.disable_credential_by_id(&first.credential_id, "manual disable")
            .unwrap();

        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::StaleFailure
        );
        let snapshot = p.credential_snapshot_by_id(&first.credential_id).unwrap();
        assert_eq!(
            snapshot.state,
            crate::credentials::CredentialStateSnapshot::Disabled {
                reason: "manual disable".to_string()
            }
        );
    }

    #[test]
    fn switchable_failure_does_not_claim_switch_when_no_alternative_exists() {
        let mut p = KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1")],
        })
        .unwrap();
        let first = p.select().unwrap();

        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::NoAlternative
        );
        assert!(p.select().is_err());
    }

    #[test]
    fn expired_cooldown_is_promoted_without_manual_clear() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_millis(1)
            ),
            SwitchOutcome::Switched
        );

        std::thread::sleep(Duration::from_millis(5));

        let all_candidates = p.retry_candidates_from_current(10);
        assert!(all_candidates
            .iter()
            .any(|candidate| candidate.credential_id == first.credential_id));
        let restored = p.select_credential_by_id(&first.credential_id).unwrap();
        assert_eq!(restored.credential_id, first.credential_id);
    }

    #[test]
    fn selector_index_tracks_cooldown_transitions() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_millis(1)
            ),
            SwitchOutcome::Switched
        );
        let first_index = p.index_for_id(&first.credential_id).unwrap();

        assert!(!p.available_indices.contains(&first_index));
        assert_eq!(p.cooldown_deadline_by_index.len(), 1);

        std::thread::sleep(Duration::from_millis(5));
        let restored = p.select_credential_by_id(&first.credential_id).unwrap();

        assert_eq!(restored.credential_id, first.credential_id);
        assert!(p.available_indices.contains(&first_index));
        assert!(p.cooldown_deadline_by_index.is_empty());
        assert!(p.cooldown_deadlines.is_empty());
    }

    #[test]
    fn repeated_cooldown_ignores_stale_deadline() {
        let mut p = KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1")],
        })
        .unwrap();
        let first = p.select().unwrap();
        let index = p.index_for_id(&first.credential_id).unwrap();

        assert_eq!(
            p.report_switchable_failure_until(
                index,
                Instant::now() + Duration::from_millis(5),
                "first cooldown"
            ),
            SwitchOutcome::NoAlternative
        );
        assert_eq!(
            p.report_switchable_failure_until(
                index,
                Instant::now() + Duration::from_millis(100),
                "second cooldown"
            ),
            SwitchOutcome::NoAlternative
        );

        std::thread::sleep(Duration::from_millis(20));

        assert!(p.select().is_err());
        assert!(!p.available_indices.contains(&index));
        assert_eq!(p.cooldown_deadline_by_index.len(), 1);

        std::thread::sleep(Duration::from_millis(120));

        assert_eq!(p.select().unwrap().credential_id, first.credential_id);
        assert!(p.cooldown_deadline_by_index.is_empty());
        assert!(p.cooldown_deadlines.is_empty());
    }

    #[test]
    fn selector_index_ignores_stale_cooldown_deadlines() {
        let mut p = pool();
        let first = p.select().unwrap();
        let first_index = p.index_for_id(&first.credential_id).unwrap();
        let first_deadline = Instant::now() + Duration::from_millis(1);
        let second_deadline = Instant::now() + Duration::from_secs(60);

        p.credentials[first_index].mark_cooling_down(first_deadline, "short");
        p.mark_index_cooling_down(first_index, first_deadline);
        p.credentials[first_index].mark_cooling_down(second_deadline, "long");
        p.mark_index_cooling_down(first_index, second_deadline);

        std::thread::sleep(Duration::from_millis(5));
        let candidates = p.retry_candidates_from_current(10);

        assert!(!candidates
            .iter()
            .any(|candidate| candidate.credential_id == first.credential_id));
        assert!(!p.available_indices.contains(&first_index));
        assert_eq!(
            p.cooldown_deadline_by_index.get(&first_index),
            Some(&second_deadline)
        );
    }

    #[test]
    fn keep_key_failure_does_not_advance() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert!(p.report_keep_key_failure_by_id(&first.credential_id));
        assert_eq!(p.select().unwrap().key, "k1");
    }

    #[test]
    fn expired_key_is_skipped() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "invalid_api_key"),
            SwitchOutcome::Switched
        );
        assert_eq!(p.select().unwrap().key, "k2");
        assert_eq!(
            p.snapshot(),
            KeyPoolSnapshot {
                current_index: 1,
                total_credentials: 3,
                available_credentials: 2,
                cooling_down_credentials: 0,
                expired_credentials: 1,
                quota_exhausted_credentials: 0,
                disabled_credentials: 0
            }
        );
    }

    #[test]
    fn stale_expired_failure_still_expires_failed_credential() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::Switched
        );
        assert_eq!(
            p.report_expired_failure_by_id(&first.credential_id, "invalid_api_key"),
            SwitchOutcome::StaleFailure
        );

        let snapshot = p.snapshot();
        assert_eq!(snapshot.current_index, 1);
        assert_eq!(snapshot.available_credentials, 2);
        assert_eq!(snapshot.cooling_down_credentials, 0);
        assert_eq!(snapshot.expired_credentials, 1);
    }

    #[test]
    fn stale_switchable_failure_does_not_mutate_current_pointer() {
        let mut p = pool();
        let first = p.select().unwrap();
        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::Switched
        );
        assert_eq!(p.select().unwrap().key, "k2");

        assert_eq!(
            p.report_switchable_failure_by_id_with_cooldown(
                &first.credential_id,
                Duration::from_secs(20)
            ),
            SwitchOutcome::StaleFailure
        );
        assert_eq!(p.select().unwrap().key, "k2");
    }
}
