use serde::Serialize;
use std::{path::PathBuf, time::Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialFingerprint(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialSource {
    pub source_path: Option<PathBuf>,
    pub source_line: Option<usize>,
    pub batch_id: Option<String>,
}

impl CredentialSource {
    #[cfg(test)]
    pub fn unknown() -> Self {
        Self {
            source_path: None,
            source_line: None,
            batch_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialState {
    Available,
    CoolingDown { until: Instant, reason: String },
    Expired { reason: String },
    QuotaExhausted { reason: String },
    Disabled { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialSnapshot {
    pub id: String,
    pub fingerprint: String,
    pub state: CredentialStateSnapshot,
    pub source: CredentialSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialStateSnapshot {
    Available,
    CoolingDown {
        reason: String,
        remaining_seconds: u64,
    },
    Expired {
        reason: String,
    },
    QuotaExhausted {
        reason: String,
    },
    Disabled {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Credential {
    id: CredentialId,
    fingerprint: CredentialFingerprint,
    secret: String,
    pub source: CredentialSource,
    state: CredentialState,
}

impl Credential {
    pub fn with_source(namespace: &str, secret: String, source: CredentialSource) -> Self {
        let fingerprint = CredentialFingerprint(short_hash(&secret));
        let id = CredentialId(format!(
            "cred_{}",
            short_hash(&format!("{namespace}:{secret}"))
        ));
        Self {
            id,
            fingerprint,
            secret,
            source,
            state: CredentialState::Available,
        }
    }

    pub fn id(&self) -> &CredentialId {
        &self.id
    }

    pub fn fingerprint(&self) -> &CredentialFingerprint {
        &self.fingerprint
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn snapshot(&self) -> CredentialSnapshot {
        let state = match &self.state {
            CredentialState::Available => CredentialStateSnapshot::Available,
            CredentialState::CoolingDown { until, reason } if Instant::now() < *until => {
                let remaining_seconds = until
                    .saturating_duration_since(Instant::now())
                    .as_secs()
                    .max(1);
                CredentialStateSnapshot::CoolingDown {
                    reason: reason.clone(),
                    remaining_seconds,
                }
            }
            CredentialState::CoolingDown { .. } => CredentialStateSnapshot::Available,
            CredentialState::Expired { reason } => CredentialStateSnapshot::Expired {
                reason: reason.clone(),
            },
            CredentialState::QuotaExhausted { reason } => CredentialStateSnapshot::QuotaExhausted {
                reason: reason.clone(),
            },
            CredentialState::Disabled { reason } => CredentialStateSnapshot::Disabled {
                reason: reason.clone(),
            },
        };
        CredentialSnapshot {
            id: self.id.0.clone(),
            fingerprint: self.fingerprint.0.clone(),
            state,
            source: self.source.clone(),
        }
    }

    pub fn is_available_at(&self, now: Instant) -> bool {
        match &self.state {
            CredentialState::Available => true,
            CredentialState::CoolingDown { until, .. } => now >= *until,
            CredentialState::Expired { .. }
            | CredentialState::QuotaExhausted { .. }
            | CredentialState::Disabled { .. } => false,
        }
    }

    pub fn is_cooling_down(&self) -> bool {
        self.is_cooling_down_at(Instant::now())
    }

    pub fn is_cooling_down_at(&self, now: Instant) -> bool {
        matches!(
            &self.state,
            CredentialState::CoolingDown { until, .. } if now < *until
        )
    }

    pub fn is_expired(&self) -> bool {
        matches!(self.state, CredentialState::Expired { .. })
    }

    pub fn is_quota_exhausted(&self) -> bool {
        matches!(self.state, CredentialState::QuotaExhausted { .. })
    }

    pub fn is_disabled(&self) -> bool {
        matches!(self.state, CredentialState::Disabled { .. })
    }

    pub fn mark_cooling_down(&mut self, until: Instant, reason: impl Into<String>) {
        self.state = CredentialState::CoolingDown {
            until,
            reason: reason.into(),
        };
    }

    pub fn mark_expired(&mut self, reason: impl Into<String>) {
        self.state = CredentialState::Expired {
            reason: reason.into(),
        };
    }

    pub fn mark_quota_exhausted(&mut self, reason: impl Into<String>) {
        self.state = CredentialState::QuotaExhausted {
            reason: reason.into(),
        };
    }

    pub fn mark_disabled(&mut self, reason: impl Into<String>) {
        self.state = CredentialState::Disabled {
            reason: reason.into(),
        };
    }

    pub fn mark_available(&mut self) {
        self.state = CredentialState::Available;
    }
}

pub fn short_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hash_uses_stable_known_vectors() {
        assert_eq!(short_hash(""), "cbf29ce484222325");
        assert_eq!(short_hash("hello"), "a430d84680aabd0b");
    }
}
