use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::models::{RemoteObject, RunIdentity, RunRecord, RunState};
use crate::{Error, Result};

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS planner_runs (
 run_key TEXT PRIMARY KEY, profile_id TEXT NOT NULL, iso_year INTEGER NOT NULL, iso_week INTEGER NOT NULL,
 profile_version TEXT NOT NULL, planner_schema_version TEXT NOT NULL, state TEXT NOT NULL, input_hash TEXT NOT NULL,
 provider_model TEXT, failure_code TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS remote_objects (
 run_key TEXT NOT NULL REFERENCES planner_runs(run_key) ON DELETE RESTRICT, object_type TEXT NOT NULL,
 remote_id TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (run_key, object_type, remote_id));
CREATE TABLE IF NOT EXISTS run_artifacts (
 run_key TEXT NOT NULL REFERENCES planner_runs(run_key) ON DELETE RESTRICT, artifact_type TEXT NOT NULL,
 payload_json TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (run_key, artifact_type));
CREATE TABLE IF NOT EXISTS run_approvals (
 run_key TEXT PRIMARY KEY REFERENCES planner_runs(run_key) ON DELETE RESTRICT,
 artifact_hash TEXT NOT NULL, approved_at TEXT NOT NULL, revoked_at TEXT);
"#;

fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn build_run_key(identity: &RunIdentity) -> Result<String> {
    identity.validate()?;
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(identity)?));
    Ok(format!(
        "nutrition-run:v{}:{digest}",
        identity.planner_schema_version
    ))
}

pub struct RunLedger {
    connection: Connection,
}

impl RunLedger {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.execute_batch(MIGRATION_1)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(1, ?1)",
            [now()],
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(2, ?1)",
            [now()],
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(4, ?1)",
            [now()],
        )?;
        Ok(Self { connection })
    }
    pub fn begin_run(
        &mut self,
        identity: &RunIdentity,
        input_hash: &str,
        provider_model: Option<&str>,
    ) -> Result<RunRecord> {
        let run_key = build_run_key(identity)?;
        let timestamp = now();
        self.connection.execute(r#"INSERT OR IGNORE INTO planner_runs(run_key,profile_id,iso_year,iso_week,profile_version,
            planner_schema_version,state,input_hash,provider_model,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)"#,
            params![run_key, identity.profile_id, identity.iso_year, identity.iso_week, identity.profile_version,
                identity.planner_schema_version, RunState::Drafting.as_str(), input_hash, provider_model, timestamp])?;
        let mut record = self
            .get_run(&run_key)?
            .ok_or_else(|| Error::RunIdentityConflict("run insert produced no record".into()))?;
        if record.input_hash != input_hash || record.provider_model.as_deref() != provider_model {
            return Err(Error::RunIdentityConflict(
                "existing run identity has different immutable input or provider model".into(),
            ));
        }
        if record.state == RunState::Failed {
            let owned_remote_objects: u32 = self.connection.query_row(
                "SELECT COUNT(*) FROM remote_objects WHERE run_key=?1",
                [&run_key],
                |row| row.get(0),
            )?;
            let artifacts: u32 = self.connection.query_row(
                "SELECT COUNT(*) FROM run_artifacts WHERE run_key=?1",
                [&run_key],
                |row| row.get(0),
            )?;
            if owned_remote_objects != 0 || artifacts != 0 {
                return Err(Error::RunIdentityConflict(
                    "failed run with artifacts or remote objects requires explicit reconciliation"
                        .into(),
                ));
            }
            let changed = self.connection.execute(
                "UPDATE planner_runs SET state=?1,failure_code=NULL,updated_at=?2 WHERE run_key=?3 AND state=?4",
                params![
                    RunState::Drafting.as_str(),
                    now(),
                    run_key,
                    RunState::Failed.as_str()
                ],
            )?;
            if changed != 1 {
                return Err(Error::RunIdentityConflict(
                    "failed run changed concurrently during retry".into(),
                ));
            }
            record = self
                .get_run(&run_key)?
                .ok_or_else(|| Error::RunIdentityConflict("retried run disappeared".into()))?;
        }
        Ok(record)
    }
    pub fn get_run(&self, run_key: &str) -> Result<Option<RunRecord>> {
        self.connection.query_row("SELECT profile_id,iso_year,iso_week,profile_version,planner_schema_version,state,input_hash,provider_model,failure_code,created_at,updated_at FROM planner_runs WHERE run_key=?1",
            [run_key], |row| Ok((row.get::<_,String>(0)?, row.get::<_,i32>(1)?, row.get::<_,u32>(2)?, row.get::<_,String>(3)?,
                row.get::<_,String>(4)?, row.get::<_,String>(5)?, row.get::<_,String>(6)?, row.get::<_,Option<String>>(7)?,
                row.get::<_,Option<String>>(8)?, row.get::<_,String>(9)?, row.get::<_,String>(10)?)))
            .optional()?.map(|v| Ok(RunRecord { run_key: run_key.into(), identity: RunIdentity { profile_id:v.0, iso_year:v.1,
                iso_week:v.2, profile_version:v.3, planner_schema_version:v.4 }, state:RunState::parse(&v.5)?, input_hash:v.6,
                provider_model:v.7, failure_code:v.8, created_at:v.9, updated_at:v.10 })).transpose()
    }
    pub fn transition(
        &mut self,
        run_key: &str,
        new_state: RunState,
        failure_code: Option<&str>,
    ) -> Result<RunRecord> {
        let current = self
            .get_run(run_key)?
            .ok_or_else(|| Error::InvalidRunTransition("unknown run".into()))?;
        let allowed = matches!(
            (current.state, new_state),
            (
                RunState::Drafting,
                RunState::NeedsReview | RunState::Validated | RunState::Failed
            ) | (
                RunState::NeedsReview,
                RunState::Validated | RunState::Failed
            ) | (RunState::Validated, RunState::Publishing | RunState::Failed)
                | (
                    RunState::Publishing,
                    RunState::Published | RunState::Partial | RunState::Failed
                )
                | (RunState::Partial, RunState::Publishing | RunState::Failed)
        );
        if !allowed {
            return Err(Error::InvalidRunTransition(format!(
                "{} -> {}",
                current.state.as_str(),
                new_state.as_str()
            )));
        }
        if matches!(new_state, RunState::Failed | RunState::Partial) != failure_code.is_some() {
            return Err(Error::InvalidRunTransition(
                "failure_code is required only for failed/partial runs".into(),
            ));
        }
        let changed = self.connection.execute("UPDATE planner_runs SET state=?1,failure_code=?2,updated_at=?3 WHERE run_key=?4 AND state=?5",
            params![new_state.as_str(), failure_code, now(), run_key, current.state.as_str()])?;
        if changed != 1 {
            return Err(Error::InvalidRunTransition(
                "run state changed concurrently".into(),
            ));
        }
        self.get_run(run_key)?
            .ok_or_else(|| Error::InvalidRunTransition("transitioned run disappeared".into()))
    }
    pub fn record_remote_object(
        &mut self,
        run_key: &str,
        object_type: &str,
        remote_id: &str,
    ) -> Result<()> {
        self.connection.execute("INSERT OR IGNORE INTO remote_objects(run_key,object_type,remote_id,created_at) VALUES(?1,?2,?3,?4)",
            params![run_key,object_type,remote_id,now()])?;
        Ok(())
    }
    pub fn remote_objects(&self, run_key: &str) -> Result<Vec<RemoteObject>> {
        let mut statement = self.connection.prepare("SELECT object_type,remote_id,created_at FROM remote_objects WHERE run_key=?1 ORDER BY object_type,remote_id")?;
        Ok(statement
            .query_map([run_key], |row| {
                Ok(RemoteObject {
                    object_type: row.get(0)?,
                    remote_id: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn put_artifact(
        &mut self,
        run_key: &str,
        artifact_type: &str,
        payload: &str,
    ) -> Result<()> {
        self.connection.execute(r#"INSERT INTO run_artifacts(run_key,artifact_type,payload_json,created_at) VALUES(?1,?2,?3,?4)
            ON CONFLICT(run_key,artifact_type) DO UPDATE SET payload_json=excluded.payload_json
            WHERE run_artifacts.payload_json=excluded.payload_json"#, params![run_key,artifact_type,payload,now()])?;
        if self.get_artifact(run_key, artifact_type)?.as_deref() != Some(payload) {
            return Err(Error::RunIdentityConflict(
                "run artifact cannot be overwritten with new content".into(),
            ));
        }
        Ok(())
    }
    pub fn get_artifact(&self, run_key: &str, artifact_type: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT payload_json FROM run_artifacts WHERE run_key=?1 AND artifact_type=?2",
                params![run_key, artifact_type],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Approves exactly one immutable artifact hash for a validated run.
    /// Returns `false` when the same active approval already exists.
    pub fn approve_artifact(&mut self, run_key: &str, artifact_hash: &str) -> Result<bool> {
        if artifact_hash.len() != 64
            || !artifact_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Validation(
                "artifact approval requires a lowercase SHA-256 hash".into(),
            ));
        }
        let run = self
            .get_run(run_key)?
            .ok_or_else(|| Error::RunIdentityConflict("approval run does not exist".into()))?;
        if run.state != RunState::Validated {
            return Err(Error::InvalidRunTransition(format!(
                "cannot approve run in state {}",
                run.state.as_str()
            )));
        }
        if !self.remote_objects(run_key)?.is_empty() {
            return Err(Error::RunIdentityConflict(
                "run with remote objects cannot be approved".into(),
            ));
        }
        let existing: Option<(String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT artifact_hash,revoked_at FROM run_approvals WHERE run_key=?1",
                [run_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((existing_hash, None)) if existing_hash == artifact_hash => Ok(false),
            Some((existing_hash, Some(_))) if existing_hash == artifact_hash => {
                self.connection.execute(
                    "UPDATE run_approvals SET approved_at=?1,revoked_at=NULL WHERE run_key=?2",
                    params![now(), run_key],
                )?;
                Ok(true)
            }
            Some(_) => Err(Error::RunIdentityConflict(
                "run approval is already bound to a different immutable artifact".into(),
            )),
            None => {
                self.connection.execute(
                    "INSERT INTO run_approvals(run_key,artifact_hash,approved_at) VALUES(?1,?2,?3)",
                    params![run_key, artifact_hash, now()],
                )?;
                Ok(true)
            }
        }
    }

    /// Revokes an approval while the validated run is still side-effect free.
    pub fn revoke_approval(&mut self, run_key: &str) -> Result<bool> {
        let run = self
            .get_run(run_key)?
            .ok_or_else(|| Error::RunIdentityConflict("approval run does not exist".into()))?;
        if run.state != RunState::Validated || !self.remote_objects(run_key)?.is_empty() {
            return Err(Error::InvalidRunTransition(
                "approval can only be revoked before publishing starts".into(),
            ));
        }
        Ok(self.connection.execute(
            "UPDATE run_approvals SET revoked_at=?1 WHERE run_key=?2 AND revoked_at IS NULL",
            params![now(), run_key],
        )? == 1)
    }

    pub fn artifact_is_approved(&self, run_key: &str, artifact_hash: &str) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM run_approvals WHERE run_key=?1 AND artifact_hash=?2 AND revoked_at IS NULL",
                params![run_key, artifact_hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    fn identity() -> RunIdentity {
        RunIdentity {
            profile_id: "p".into(),
            iso_year: 2026,
            iso_week: 48,
            profile_version: "v".into(),
            planner_schema_version: "2".into(),
        }
    }
    #[test]
    fn repeated_run_is_idempotent_and_conflicts_fail() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let first = ledger
            .begin_run(&identity(), "hash", Some("model"))
            .unwrap();
        assert_eq!(
            first,
            ledger
                .begin_run(&identity(), "hash", Some("model"))
                .unwrap()
        );
        assert!(matches!(
            ledger.begin_run(&identity(), "other", Some("model")),
            Err(Error::RunIdentityConflict(_))
        ));
    }
    #[test]
    fn failed_draft_without_side_effects_can_retry() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let run = ledger
            .begin_run(&identity(), "hash", Some("model"))
            .unwrap();
        ledger
            .transition(&run.run_key, RunState::Failed, Some("ProviderError"))
            .unwrap();
        let retried = ledger
            .begin_run(&identity(), "hash", Some("model"))
            .unwrap();
        assert_eq!(retried.state, RunState::Drafting);
        assert_eq!(retried.failure_code, None);
    }
    #[test]
    fn failed_run_with_remote_objects_cannot_retry() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let run = ledger
            .begin_run(&identity(), "hash", Some("model"))
            .unwrap();
        ledger
            .record_remote_object(&run.run_key, "recipe", "remote-1")
            .unwrap();
        ledger
            .transition(&run.run_key, RunState::Failed, Some("PublishError"))
            .unwrap();
        assert!(matches!(
            ledger.begin_run(&identity(), "hash", Some("model")),
            Err(Error::RunIdentityConflict(_))
        ));
    }
    #[test]
    fn artifacts_are_immutable() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let run = ledger.begin_run(&identity(), "hash", None).unwrap();
        ledger.put_artifact(&run.run_key, "preview", "one").unwrap();
        assert!(ledger.put_artifact(&run.run_key, "preview", "two").is_err());
    }
    #[test]
    fn invalid_transition_fails() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let run = ledger.begin_run(&identity(), "hash", None).unwrap();
        assert!(
            ledger
                .transition(&run.run_key, RunState::Published, None)
                .is_err()
        );
    }

    #[test]
    fn approvals_are_idempotent_hash_bound_and_revocable() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("db")).unwrap();
        let run = ledger.begin_run(&identity(), "hash", None).unwrap();
        ledger
            .transition(&run.run_key, RunState::Validated, None)
            .unwrap();
        let first_hash = "a".repeat(64);
        let other_hash = "b".repeat(64);

        assert!(ledger.approve_artifact(&run.run_key, &first_hash).unwrap());
        assert!(!ledger.approve_artifact(&run.run_key, &first_hash).unwrap());
        assert!(
            ledger
                .artifact_is_approved(&run.run_key, &first_hash)
                .unwrap()
        );
        assert!(matches!(
            ledger.approve_artifact(&run.run_key, &other_hash),
            Err(Error::RunIdentityConflict(_))
        ));
        assert!(ledger.revoke_approval(&run.run_key).unwrap());
        assert!(!ledger.revoke_approval(&run.run_key).unwrap());
        assert!(
            !ledger
                .artifact_is_approved(&run.run_key, &first_hash)
                .unwrap()
        );
        assert!(ledger.approve_artifact(&run.run_key, &first_hash).unwrap());
    }
}
