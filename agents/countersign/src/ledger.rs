//! The receipt ledger: the queryable index behind the room's permanent log.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Receipt {
    pub id: String,
    pub room_id: String,
    pub branch: String,
    pub commit_sha: String,
    pub base_sha: String,
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub changed_files: Vec<String>,
    pub base_output: String,
    pub suite_output: String,
    pub diffstat: String,
    pub diff: String,
    pub duration_ms: i64,
    pub attempt: u8,
    pub supersedes: Option<String>,
    pub created_at: String,
    pub countersigned_by: Option<String>,
    pub countersigned_at: Option<String>,
    pub landed_url: Option<String>,
    pub pr_state: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "PASS" {
            Verdict::Pass
        } else {
            Verdict::Fail
        }
    }
}

#[derive(Clone)]
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create ledger directory {}", parent.display()))?;
        }
        let ledger = Self { path };
        ledger.with(|connection| {
            connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS receipts (
                    id TEXT PRIMARY KEY,
                    room_id TEXT NOT NULL,
                    branch TEXT NOT NULL,
                    commit_sha TEXT NOT NULL,
                    base_sha TEXT NOT NULL DEFAULT '',
                    verdict TEXT NOT NULL,
                    reason TEXT,
                    changed_files TEXT NOT NULL,
                    base_output TEXT NOT NULL,
                    suite_output TEXT NOT NULL,
                    diffstat TEXT NOT NULL DEFAULT '',
                    diff TEXT NOT NULL DEFAULT '',
                    duration_ms INTEGER NOT NULL,
                    attempt INTEGER NOT NULL DEFAULT 1,
                    supersedes TEXT,
                    created_at TEXT NOT NULL,
                    countersigned_by TEXT,
                    countersigned_at TEXT,
                    landed_url TEXT,
                    pr_state TEXT
                );
                CREATE INDEX IF NOT EXISTS receipts_created_at
                    ON receipts (created_at DESC);
                "#,
            )
        })?;
        // A ledger created by an earlier version of Countersign is upgraded in
        // place; the receipt history is not disposable.
        for (name, ddl) in [
            ("base_sha", "base_sha TEXT NOT NULL DEFAULT ''"),
            ("diffstat", "diffstat TEXT NOT NULL DEFAULT ''"),
            ("diff", "diff TEXT NOT NULL DEFAULT ''"),
            ("attempt", "attempt INTEGER NOT NULL DEFAULT 1"),
            ("supersedes", "supersedes TEXT"),
            ("pr_state", "pr_state TEXT"),
        ] {
            let exists: i64 = ledger.with(|connection| {
                connection.query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('receipts') WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
            })?;
            if exists == 0 {
                ledger.with(|connection| {
                    connection.execute_batch(&format!("ALTER TABLE receipts ADD COLUMN {ddl}"))
                })?;
            }
        }
        Ok(ledger)
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("open ledger {}", self.path.display()))?;
        Ok(f(&connection)?)
    }

    pub fn insert(&self, receipt: &Receipt) -> Result<()> {
        let changed = serde_json::to_string(&receipt.changed_files)?;
        self.with(|connection| {
            connection.execute(
                r#"INSERT INTO receipts
                   (id, room_id, branch, commit_sha, base_sha, verdict, reason,
                    changed_files, base_output, suite_output, diffstat, diff,
                    duration_ms, attempt, supersedes, created_at,
                    countersigned_by, countersigned_at, landed_url, pr_state)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                           ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)"#,
                params![
                    receipt.id,
                    receipt.room_id,
                    receipt.branch,
                    receipt.commit_sha,
                    receipt.base_sha,
                    receipt.verdict.as_str(),
                    receipt.reason,
                    changed,
                    receipt.base_output,
                    receipt.suite_output,
                    receipt.diffstat,
                    receipt.diff,
                    receipt.duration_ms,
                    receipt.attempt as i64,
                    receipt.supersedes,
                    receipt.created_at,
                    receipt.countersigned_by,
                    receipt.countersigned_at,
                    receipt.landed_url,
                    receipt.pr_state,
                ],
            )
        })?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Receipt>> {
        let row = self.with(|connection| {
            let mut statement = connection.prepare(
                r#"SELECT id, room_id, branch, commit_sha, base_sha, verdict, reason,
                          changed_files, base_output, suite_output, diffstat, diff,
                          duration_ms, attempt, supersedes, created_at,
                          countersigned_by, countersigned_at, landed_url, pr_state
                   FROM receipts WHERE id = ?1"#,
            )?;
            let mut rows = statement.query(params![id])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(receipt_from_row(row)?))
        })?;
        Ok(row)
    }

    pub fn countersign(&self, id: &str, by: &str, landed_url: Option<&str>) -> Result<()> {
        let at = Utc::now().to_rfc3339();
        self.with(|connection| {
            connection.execute(
                "UPDATE receipts SET countersigned_by = ?1, countersigned_at = ?2, landed_url = ?3 WHERE id = ?4",
                params![by, at, landed_url, id],
            )
        })?;
        Ok(())
    }

    pub fn set_pr_state(&self, id: &str, state: &str) -> Result<()> {
        self.with(|connection| {
            connection.execute(
                "UPDATE receipts SET pr_state = ?1 WHERE id = ?2",
                params![state, id],
            )
        })?;
        Ok(())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Receipt>> {
        let rows = self.with(|connection| {
            let mut statement = connection.prepare(
                r#"SELECT id, room_id, branch, commit_sha, base_sha, verdict, reason,
                          changed_files, base_output, suite_output, diffstat, diff,
                          duration_ms, attempt, supersedes, created_at,
                          countersigned_by, countersigned_at, landed_url, pr_state
                   FROM receipts ORDER BY created_at DESC LIMIT ?1"#,
            )?;
            let mut rows = statement.query(params![limit as i64])?;
            let mut receipts = Vec::new();
            while let Some(row) = rows.next()? {
                receipts.push(receipt_from_row(row)?);
            }
            Ok(receipts)
        })?;
        Ok(rows)
    }
}

fn receipt_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    Ok(Receipt {
        id: row.get(0)?,
        room_id: row.get(1)?,
        branch: row.get(2)?,
        commit_sha: row.get(3)?,
        base_sha: row.get(4)?,
        verdict: Verdict::parse(&row.get::<_, String>(5)?),
        reason: row.get(6)?,
        changed_files: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
        base_output: row.get(8)?,
        suite_output: row.get(9)?,
        diffstat: row.get(10)?,
        diff: row.get(11)?,
        duration_ms: row.get(12)?,
        attempt: row.get::<_, i64>(13)?.clamp(1, 9) as u8,
        supersedes: row.get(14)?,
        created_at: row.get(15)?,
        countersigned_by: row.get(16)?,
        countersigned_at: row.get(17)?,
        landed_url: row.get(18)?,
        pr_state: row.get(19)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{Ledger, Receipt, Verdict};

    fn sample() -> Receipt {
        Receipt {
            id: "R-TEST01".into(),
            room_id: "room".into(),
            branch: "fix/x".into(),
            commit_sha: "abc123".into(),
            base_sha: "base123".into(),
            verdict: Verdict::Pass,
            reason: None,
            changed_files: vec!["demo/orders/src/lib.rs".into()],
            base_output: "failed".into(),
            suite_output: "passed".into(),
            diffstat: "1 file changed".into(),
            diff: "+fixed".into(),
            duration_ms: 1234,
            attempt: 2,
            supersedes: Some("R-PARENT".into()),
            created_at: "2026-11-04T10:00:00Z".into(),
            countersigned_by: None,
            countersigned_at: None,
            landed_url: None,
            pr_state: None,
        }
    }

    #[test]
    fn a_receipt_round_trips_and_is_signed_once() {
        let path = std::env::temp_dir().join(format!(
            "countersign-test-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let ledger = Ledger::open(&path).expect("open");
        ledger.insert(&sample()).expect("insert");
        let stored = ledger.get("R-TEST01").expect("get").expect("present");
        assert_eq!(stored.verdict, Verdict::Pass);
        assert_eq!(stored.attempt, 2);
        assert_eq!(stored.supersedes.as_deref(), Some("R-PARENT"));
        assert!(stored.countersigned_by.is_none());

        ledger
            .countersign("R-TEST01", "Ada", Some("https://example.test/pr/1"))
            .expect("sign");
        let signed = ledger.get("R-TEST01").expect("get").expect("present");
        assert_eq!(signed.countersigned_by.as_deref(), Some("Ada"));
        assert_eq!(signed.landed_url.as_deref(), Some("https://example.test/pr/1"));
        assert_eq!(ledger.recent(10).expect("recent").len(), 1);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_old_ledger_is_upgraded_in_place() {
        let path = std::env::temp_dir().join(format!(
            "countersign-old-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE receipts (
                        id TEXT PRIMARY KEY, room_id TEXT NOT NULL, branch TEXT NOT NULL,
                        commit_sha TEXT NOT NULL, verdict TEXT NOT NULL, reason TEXT,
                        changed_files TEXT NOT NULL, base_output TEXT NOT NULL,
                        suite_output TEXT NOT NULL, duration_ms INTEGER NOT NULL,
                        created_at TEXT NOT NULL, countersigned_by TEXT,
                        countersigned_at TEXT, landed_url TEXT
                    );",
                )
                .unwrap();
        }
        let ledger = Ledger::open(&path).expect("upgrade");
        ledger.insert(&sample()).expect("insert after upgrade");
        assert!(ledger.get("R-TEST01").unwrap().is_some());
        let _ = std::fs::remove_file(path);
    }
}
