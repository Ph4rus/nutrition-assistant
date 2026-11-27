use sha2::{Digest, Sha256};

use crate::Result;
use crate::dry_run::DryRunPreview;
use crate::storage::RunLedger;

/// Hashes the typed artifact rather than its file formatting, so whitespace is irrelevant.
pub fn preview_hash(preview: &DryRunPreview) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(preview)?)
    ))
}

pub fn approve(preview: &DryRunPreview, ledger: &mut RunLedger) -> Result<bool> {
    ledger.approve_artifact(&preview.run_key, &preview_hash(preview)?)
}

pub fn revoke(preview: &DryRunPreview, ledger: &mut RunLedger) -> Result<bool> {
    ledger.revoke_approval(&preview.run_key)
}

pub fn is_approved(preview: &DryRunPreview, ledger: &RunLedger) -> Result<bool> {
    ledger.artifact_is_approved(&preview.run_key, &preview_hash(preview)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RunState;

    #[test]
    fn hash_changes_when_artifact_content_changes() {
        let mut preview = DryRunPreview {
            run_key: "run".into(),
            state: RunState::Validated,
            plan: None,
            issues: vec![],
            nutrition: vec![],
            shopping: vec![],
            destination_conflicts: vec![],
            writes_performed: 0,
        };
        let original = preview_hash(&preview).unwrap();
        preview.writes_performed = 1;
        assert_ne!(original, preview_hash(&preview).unwrap());
    }
}
