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
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub changed_files: Vec<String>,
    pub base_output: String,
    pub suite_output: String,
    pub duration_ms: i64,
    pub created_at: String,
    pub countersigned_by: Option<String>,
    pub countersigned_at: Option<String>,
    pub landed_url: Option<String>,
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
                    verdict TEXT NOT NULL,
                    reason TEXT,
                    changed_files TEXT NOT NULL,
                    base_output TEXT NOT NULL,
                    suite_output TEXT NOT NULL,
                    duration_ms INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    countersigned_by TEXT,
                    countersigned_at TEXT,
                    landed_url TEXT
                );
                CREATE INDEX IF NOT EXISTS receipts_created_at
                    ON receipts (created_at DESC);
                "#,
            )
        })?;
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
                   (id, room_id, branch, commit_sha, verdict, reason, changed_files,
                    base_output, suite_output, duration_ms, created_at,
                    countersigned_by, countersigned_at, landed_url)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
                params![
                    receipt.id,
                    receipt.room_id,
                    receipt.branch,
                    receipt.commit_sha,
                    receipt.verdict.as_str(),
                    receipt.reason,
                    changed,
                    receipt.base_output,
                    receipt.suite_output,
                    receipt.duration_ms,
                    receipt.created_at,
                    receipt.countersigned_by,
                    receipt.countersigned_at,
                    receipt.landed_url,
                ],
            )
        })?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Receipt>> {
        let row = self.with(|connection| {
            let mut statement = connection.prepare(
                r#"SELECT id, room_id, branch, commit_sha, verdict, reason, changed_files,
                          base_output, suite_output, duration_ms, created_at,
                          countersigned_by, countersigned_at, landed_url
                   FROM receipts WHERE id = ?1"#,
            )?;
            let mut rows = statement.query(params![id])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(Receipt {
                id: row.get(0)?,
                room_id: row.get(1)?,
                branch: row.get(2)?,
                commit_sha: row.get(3)?,
                verdict: Verdict::parse(&row.get::<_, String>(4)?),
                reason: row.get(5)?,
                changed_files: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                base_output: row.get(7)?,
                suite_output: row.get(8)?,
                duration_ms: row.get(9)?,
                created_at: row.get(10)?,
                countersigned_by: row.get(11)?,
                countersigned_at: row.get(12)?,
                landed_url: row.get(13)?,
            }))
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

    pub fn recent(&self, limit: usize) -> Result<Vec<Receipt>> {
        let rows = self.with(|connection| {
            let mut statement = connection.prepare(
                r#"SELECT id, room_id, branch, commit_sha, verdict, reason, changed_files,
                          base_output, suite_output, duration_ms, created_at,
                          countersigned_by, countersigned_at, landed_url
                   FROM receipts ORDER BY created_at DESC LIMIT ?1"#,
            )?;
            let mut rows = statement.query(params![limit as i64])?;
            let mut receipts = Vec::new();
            while let Some(row) = rows.next()? {
                receipts.push(Receipt {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    branch: row.get(2)?,
                    commit_sha: row.get(3)?,
                    verdict: Verdict::parse(&row.get::<_, String>(4)?),
                    reason: row.get(5)?,
                    changed_files: serde_json::from_str(&row.get::<_, String>(6)?)
                        .unwrap_or_default(),
                    base_output: row.get(7)?,
                    suite_output: row.get(8)?,
                    duration_ms: row.get(9)?,
                    created_at: row.get(10)?,
                    countersigned_by: row.get(11)?,
                    countersigned_at: row.get(12)?,
                    landed_url: row.get(13)?,
                });
            }
            Ok(receipts)
        })?;
        Ok(rows)
    }
}
