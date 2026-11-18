//! SQLite run log at `<workspace>/.mwa/runs.db` (F13: kept outside git, pruned after N days).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

#[derive(Debug, Clone, Default)]
pub struct CallRecord {
    pub run_id: String,
    pub started_at_ms: i64,
    pub role: String,
    pub backend: String,
    pub kind: String,
    pub model: String,
    pub duration_ms: i64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    /// `ok` or `error`.
    pub status: String,
    pub error: Option<String>,
    pub raw: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("실행 기록 폴더 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("실행 기록 DB 오류: {0}")]
    Sql(#[from] rusqlite::Error),
}

pub struct RunLog {
    conn: Connection,
    raw_dir: PathBuf,
}

impl RunLog {
    /// Opens (creating if needed) the log under `state_dir` (usually `<workspace>/.mwa`).
    pub fn open(state_dir: &Path) -> Result<Self, LogError> {
        let raw_dir = state_dir.join("raw");
        std::fs::create_dir_all(&raw_dir)?;
        let conn = Connection::open(state_dir.join("runs.db"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS calls (
                id INTEGER PRIMARY KEY,
                run_id TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                role TEXT NOT NULL,
                backend TEXT NOT NULL,
                kind TEXT NOT NULL,
                model TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cost_usd REAL,
                status TEXT NOT NULL,
                error TEXT,
                raw_path TEXT
            );
            CREATE INDEX IF NOT EXISTS calls_run ON calls(run_id);
            CREATE INDEX IF NOT EXISTS calls_started ON calls(started_at_ms);",
        )?;
        Ok(Self { conn, raw_dir })
    }

    /// Stores the call; the raw backend output goes to `raw/<id>.txt`. Returns the row id.
    pub fn record(&self, r: &CallRecord) -> Result<i64, LogError> {
        self.conn.execute(
            "INSERT INTO calls (run_id, started_at_ms, role, backend, kind, model, duration_ms,
                input_tokens, output_tokens, cost_usd, status, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                r.run_id,
                r.started_at_ms,
                r.role,
                r.backend,
                r.kind,
                r.model,
                r.duration_ms,
                r.input_tokens.map(|v| v as i64),
                r.output_tokens.map(|v| v as i64),
                r.cost_usd,
                r.status,
                r.error,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        if !r.raw.is_empty() {
            let path = self.raw_dir.join(format!("{id}.txt"));
            if std::fs::write(&path, &r.raw).is_ok() {
                self.conn.execute(
                    "UPDATE calls SET raw_path = ?1 WHERE id = ?2",
                    params![path.to_string_lossy(), id],
                )?;
            }
        }
        Ok(id)
    }

    /// Deletes rows and raw files older than `days`. Returns the number of rows removed.
    pub fn prune(&self, days: u32, now_ms: i64) -> Result<usize, LogError> {
        let cutoff = now_ms - i64::from(days) * 86_400_000;
        let mut stmt = self.conn.prepare(
            "SELECT raw_path FROM calls WHERE started_at_ms < ?1 AND raw_path IS NOT NULL",
        )?;
        let paths: Vec<String> = stmt
            .query_map([cutoff], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
        Ok(self
            .conn
            .execute("DELETE FROM calls WHERE started_at_ms < ?1", [cutoff])?)
    }

    pub fn count_for_run(&self, run_id: &str) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM calls WHERE run_id = ?1",
            [run_id],
            |r| r.get(0),
        )
    }

    /// Sum of recorded cost for a run (calls without a cost count as zero).
    pub fn cost_for_run(&self, run_id: &str) -> rusqlite::Result<f64> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM calls WHERE run_id = ?1",
            [run_id],
            |r| r.get(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(run: &str, at: i64, raw: &str) -> CallRecord {
        CallRecord {
            run_id: run.into(),
            started_at_ms: at,
            role: "worker".into(),
            backend: "claude".into(),
            kind: "cli:claude-code".into(),
            model: "haiku".into(),
            status: "ok".into(),
            cost_usd: Some(0.5),
            raw: raw.into(),
            ..Default::default()
        }
    }

    #[test]
    fn records_and_sums_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::open(dir.path()).unwrap();
        let id = log.record(&rec("r1", 1, "raw-out")).unwrap();
        log.record(&rec("r1", 2, "")).unwrap();
        log.record(&rec("r2", 3, "")).unwrap();
        assert_eq!(log.count_for_run("r1").unwrap(), 2);
        assert_eq!(log.cost_for_run("r1").unwrap(), 1.0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(format!("raw/{id}.txt"))).unwrap(),
            "raw-out"
        );
    }

    #[test]
    fn prune_removes_old_rows_and_raw_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = RunLog::open(dir.path()).unwrap();
        let day = 86_400_000;
        let old = log.record(&rec("old", 0, "x")).unwrap();
        log.record(&rec("new", 20 * day, "y")).unwrap();
        assert_eq!(log.prune(14, 20 * day).unwrap(), 1);
        assert_eq!(log.count_for_run("old").unwrap(), 0);
        assert_eq!(log.count_for_run("new").unwrap(), 1);
        assert!(!dir.path().join(format!("raw/{old}.txt")).exists());
    }

    #[test]
    fn reopen_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        RunLog::open(dir.path())
            .unwrap()
            .record(&rec("r", 1, ""))
            .unwrap();
        assert_eq!(
            RunLog::open(dir.path())
                .unwrap()
                .count_for_run("r")
                .unwrap(),
            1
        );
    }
}
