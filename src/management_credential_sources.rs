use serde::Serialize;
use std::path::Path;

use crate::{credential_repository::KeyImportReport, state::PoolState};

#[derive(Debug, Serialize)]
pub struct KeyImportStatus {
    pub source_id: String,
    pub physical_line_count: usize,
    pub non_empty_count: usize,
    pub unique_count: usize,
    pub duplicate_occurrence_count: usize,
    pub ignored_empty_count: usize,
    pub invalid_line_count: usize,
    pub claimed_count: Option<usize>,
    pub claim_source: Option<String>,
    pub import_generation: u64,
    pub last_imported_at_unix_seconds: i64,
}

pub fn key_import_status(report: &KeyImportReport) -> KeyImportStatus {
    KeyImportStatus {
        source_id: source_id(&report.source_path),
        physical_line_count: report.physical_line_count,
        non_empty_count: report.non_empty_count,
        unique_count: report.unique_count,
        duplicate_occurrence_count: report.duplicate_occurrence_count,
        ignored_empty_count: report.ignored_empty_count,
        invalid_line_count: report.invalid_line_count,
        claimed_count: report.claimed_count,
        claim_source: report.claim_source.clone(),
        import_generation: report.import_generation,
        last_imported_at_unix_seconds: report.last_imported_at_unix_seconds,
    }
}

pub fn key_import_status_for_pool(pool_state: &PoolState) -> KeyImportStatus {
    let report = pool_state
        .key_import_report
        .lock()
        .expect("key import report mutex poisoned");
    key_import_status(&report)
}

pub fn key_import_source_id_for_pool(pool_state: &PoolState) -> String {
    let report = pool_state
        .key_import_report
        .lock()
        .expect("key import report mutex poisoned");
    key_import_status(&report).source_id
}

pub(crate) fn source_id(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown-source".to_string())
}
