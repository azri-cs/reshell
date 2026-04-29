use super::pattern::{current_platform_tag, Pattern};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Cap stored command outputs to limit database growth (oldest rows pruned after insert).
const MAX_STORED_OUTPUT_ROWS: usize = 5000;

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn new() -> anyhow::Result<Self> {
        let path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".reshell")
            .join("patterns.db");
        Self::new_at_path(path)
    }

    pub fn new_at_path(path: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let conn = Connection::open(&path)?;
        // Performance: WAL mode for concurrent reads, synchronous=NORMAL for speed
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        Self::set_restrictive_permissions(&path);
        conn.execute(
            "CREATE TABLE IF NOT EXISTS patterns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command_hash TEXT NOT NULL,
                command_template TEXT NOT NULL,
                recovery_code TEXT NOT NULL,
                stderr_pattern TEXT NOT NULL,
                fix_command TEXT,
                fix_success_rate REAL DEFAULT 0.0,
                last_used TIMESTAMP,
                usage_count INTEGER DEFAULT 1,
                platform_tag TEXT DEFAULT 'unknown'
            )",
            [],
        )?;
        // Migration: add platform_tag column if missing from older DBs
        let _ = conn.execute(
            "ALTER TABLE patterns ADD COLUMN platform_tag TEXT DEFAULT 'unknown'",
            [],
        );
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_patterns_template_stderr
             ON patterns(command_template, stderr_pattern)",
            [],
        );
        conn.execute(
            "CREATE TABLE IF NOT EXISTS outputs (
                output_id TEXT PRIMARY KEY,
                execution_id TEXT,
                original_command TEXT NOT NULL,
                stdout TEXT,
                stderr TEXT,
                exit_code INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        // Migration: add execution_id column if missing from older DBs
        let _ = conn.execute("ALTER TABLE outputs ADD COLUMN execution_id TEXT", []);
        conn.execute(
            "CREATE TABLE IF NOT EXISTS recovery_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern_id INTEGER,
                recovery_code TEXT,
                original_command TEXT,
                suggested_action TEXT,
                attempted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (pattern_id) REFERENCES patterns(id)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command_hash TEXT NOT NULL,
                command_template TEXT NOT NULL,
                cwd TEXT,
                exit_code INTEGER,
                recovery_code TEXT,
                validation_passed BOOLEAN DEFAULT 1,
                executed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn run_db<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&Connection) -> anyhow::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            f(&guard)
        })
        .await
        .map_err(|e| anyhow::anyhow!("database task join: {}", e))?
    }

    pub fn next_output_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub async fn find_pattern(
        &self,
        command_template: &str,
        stderr: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let command_template = command_template.to_string();
        let stderr = stderr.to_string();
        self.run_db(move |conn| {
            let platform = current_platform_tag();
            let mut stmt = conn.prepare_cached(
                "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                        fix_command, fix_success_rate, last_used, usage_count, platform_tag
                   FROM patterns
                  WHERE command_template = ?1
                    AND length(?2) > 0
                    AND length(stderr_pattern) > 0
                    AND ?2 LIKE '%' || stderr_pattern || '%'
                  ORDER BY (platform_tag = ?3) DESC, fix_success_rate DESC, usage_count DESC
                  LIMIT 1",
            )?;
            let mut rows = stmt.query(params![command_template, stderr, platform])?;
            if let Some(row) = rows.next()? {
                Ok(Some(Pattern {
                    id: row.get(0)?,
                    command_hash: row.get(1)?,
                    command_template: row.get(2)?,
                    recovery_code: row.get(3)?,
                    stderr_pattern: row.get(4)?,
                    fix_command: row.get(5)?,
                    fix_success_rate: row.get(6)?,
                    last_used: row.get(7)?,
                    usage_count: row.get(8)?,
                    platform_tag: row.get(9)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    pub async fn save_pattern(&self, pattern: &Pattern) -> anyhow::Result<()> {
        let pattern = pattern.clone();
        self.run_db(move |conn| {
            conn.execute(
                "INSERT INTO patterns (command_hash, command_template, recovery_code, stderr_pattern,
                                       fix_command, fix_success_rate, last_used, usage_count, platform_tag)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                  ON CONFLICT(command_template, stderr_pattern) DO UPDATE SET
                     recovery_code = excluded.recovery_code,
                     fix_command = excluded.fix_command,
                     fix_success_rate = excluded.fix_success_rate,
                     last_used = excluded.last_used,
                     usage_count = usage_count + 1,
                     platform_tag = excluded.platform_tag",
                params![
                    &pattern.command_hash,
                    &pattern.command_template,
                    &pattern.recovery_code,
                    &pattern.stderr_pattern,
                    pattern.fix_command.as_ref(),
                    pattern.fix_success_rate,
                    Utc::now().to_rfc3339(),
                    pattern.usage_count,
                    pattern.platform_tag.as_deref().unwrap_or("unknown"),
                ],
            )?;
            Ok(())
        })
        .await
    }

    fn prune_old_outputs(conn: &Connection) -> anyhow::Result<()> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM outputs", [], |row| row.get(0))?;
        if count <= MAX_STORED_OUTPUT_ROWS as i64 {
            return Ok(());
        }
        let excess = count - MAX_STORED_OUTPUT_ROWS as i64;
        conn.execute(
            "DELETE FROM outputs WHERE rowid IN (
                SELECT rowid FROM outputs
                ORDER BY datetime(created_at) ASC, rowid ASC
                LIMIT ?1
            )",
            params![excess],
        )?;
        Ok(())
    }

    pub async fn save_output(
        &self,
        output_id: &str,
        execution_id: &str,
        original_command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> anyhow::Result<()> {
        let output_id = output_id.to_string();
        let execution_id = execution_id.to_string();
        let original_command = original_command.to_string();
        let stdout = stdout.to_string();
        let stderr = stderr.to_string();
        self.run_db(move |conn| {
            conn.execute(
                "INSERT INTO outputs (output_id, execution_id, original_command, stdout, stderr, exit_code)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(output_id) DO UPDATE SET
                    execution_id = excluded.execution_id,
                    stdout = excluded.stdout,
                    stderr = excluded.stderr,
                    exit_code = excluded.exit_code",
                params![output_id, execution_id, original_command, stdout, stderr, exit_code],
            )?;
            Self::prune_old_outputs(conn)?;
            Ok(())
        })
        .await
    }

    pub async fn get_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let output_id = output_id.to_string();
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT output_id, execution_id, original_command, stdout, stderr, exit_code, created_at
                 FROM outputs WHERE output_id = ?1",
            )?;
            let mut rows = stmt.query(params![output_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(StoredOutput {
                    output_id: row.get(0)?,
                    execution_id: row.get(1)?,
                    original_command: row.get(2)?,
                    stdout: row.get(3)?,
                    stderr: row.get(4)?,
                    exit_code: row.get(5)?,
                    created_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    /// Look up a stored output by its execution_id (from rsh_exec response).
    pub async fn get_output_by_execution_id(
        &self,
        execution_id: &str,
    ) -> anyhow::Result<Option<StoredOutput>> {
        let execution_id = execution_id.to_string();
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT output_id, execution_id, original_command, stdout, stderr, exit_code, created_at
                 FROM outputs WHERE execution_id = ?1
                 ORDER BY created_at DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![execution_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(StoredOutput {
                    output_id: row.get(0)?,
                    execution_id: row.get(1)?,
                    original_command: row.get(2)?,
                    stdout: row.get(3)?,
                    stderr: row.get(4)?,
                    exit_code: row.get(5)?,
                    created_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    pub async fn previous_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let output_id = output_id.to_string();
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT o.output_id, o.execution_id, o.original_command, o.stdout, o.stderr, o.exit_code, o.created_at
                 FROM outputs o
                 WHERE (
                   o.created_at < (SELECT created_at FROM outputs WHERE output_id = ?1)
                   OR (
                     o.created_at = (SELECT created_at FROM outputs WHERE output_id = ?1)
                     AND o.rowid < (SELECT rowid FROM outputs WHERE output_id = ?1)
                   )
                 )
                 ORDER BY o.created_at DESC, o.rowid DESC
                 LIMIT 1"
            )?;
            let mut rows = stmt.query(params![output_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(StoredOutput {
                    output_id: row.get(0)?,
                    execution_id: row.get(1)?,
                    original_command: row.get(2)?,
                    stdout: row.get(3)?,
                    stderr: row.get(4)?,
                    exit_code: row.get(5)?,
                    created_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    pub async fn latest_output(&self) -> anyhow::Result<Option<StoredOutput>> {
        self.run_db(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT output_id, execution_id, original_command, stdout, stderr, exit_code, created_at
                 FROM outputs ORDER BY created_at DESC, rowid DESC LIMIT 1",
            )?;
            let mut rows = stmt.query([])?;
            if let Some(row) = rows.next()? {
                Ok(Some(StoredOutput {
                    output_id: row.get(0)?,
                    execution_id: row.get(1)?,
                    original_command: row.get(2)?,
                    stdout: row.get(3)?,
                    stderr: row.get(4)?,
                    exit_code: row.get(5)?,
                    created_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    /// Get recent outputs for resource listing.
    pub async fn list_recent_outputs(&self, limit: i64) -> anyhow::Result<Vec<StoredOutput>> {
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT output_id, execution_id, original_command, stdout, stderr, exit_code, created_at
                 FROM outputs ORDER BY created_at DESC LIMIT ?1",
            )?;
            let mut results = Vec::new();
            let mut rows = stmt.query(params![limit])?;
            while let Some(row) = rows.next()? {
                results.push(StoredOutput {
                    output_id: row.get(0)?,
                    execution_id: row.get(1)?,
                    original_command: row.get(2)?,
                    stdout: row.get(3)?,
                    stderr: row.get(4)?,
                    exit_code: row.get(5)?,
                    created_at: row.get(6)?,
                });
            }
            Ok(results)
        })
        .await
    }

    pub async fn pattern_count(&self) -> anyhow::Result<i64> {
        self.run_db(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM patterns", [], |row| row.get(0))?;
            Ok(count)
        })
        .await
    }

    pub async fn find_pattern_exact(
        &self,
        command_template: &str,
        stderr_pattern: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let command_template = command_template.to_string();
        let stderr_pattern = stderr_pattern.to_string();
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                        fix_command, fix_success_rate, last_used, usage_count, platform_tag
                 FROM patterns
                 WHERE command_template = ?1 AND stderr_pattern = ?2
                 LIMIT 1",
            )?;
            let mut rows = stmt.query(params![command_template, stderr_pattern])?;
            if let Some(row) = rows.next()? {
                Ok(Some(Pattern {
                    id: row.get(0)?,
                    command_hash: row.get(1)?,
                    command_template: row.get(2)?,
                    recovery_code: row.get(3)?,
                    stderr_pattern: row.get(4)?,
                    fix_command: row.get(5)?,
                    fix_success_rate: row.get(6)?,
                    last_used: row.get(7)?,
                    usage_count: row.get(8)?,
                    platform_tag: row.get(9)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    /// Log a recovery suggestion being served to the agent.
    pub async fn log_recovery_attempt(
        &self,
        recovery_code: &str,
        original_command: &str,
        suggested_action: &str,
    ) -> anyhow::Result<()> {
        let recovery_code = recovery_code.to_string();
        let original_command = original_command.to_string();
        let suggested_action = suggested_action.to_string();
        self.run_db(move |conn| {
            conn.execute(
                "INSERT INTO recovery_attempts (recovery_code, original_command, suggested_action)
                 VALUES (?1, ?2, ?3)",
                params![recovery_code, original_command, suggested_action],
            )?;
            Ok(())
        })
        .await
    }

    /// Update a pattern's fix outcome after the agent reports whether a fix worked.
    /// Matches by `command_template` exact + `stderr_pattern` LIKE containment.
    /// Uses a rolling average to update `fix_success_rate`:
    /// new_rate = (old_rate * old_count + (1 if success else 0)) / (old_count + 1).
    pub async fn update_fix_outcome(
        &self,
        command_template: &str,
        stderr: &str,
        fix_command: Option<&str>,
        success: bool,
    ) -> anyhow::Result<()> {
        let command_template = command_template.to_string();
        let stderr = stderr.to_string();
        let fix_command = fix_command.map(|s| s.to_string());
        self.run_db(move |conn| {
            // Find the matching pattern using LIKE (same fuzzy logic as find_pattern)
            let mut stmt = conn.prepare_cached(
                "SELECT id, fix_success_rate, usage_count
                   FROM patterns
                  WHERE command_template = ?1
                    AND length(?2) > 0
                    AND length(stderr_pattern) > 0
                    AND ?2 LIKE '%' || stderr_pattern || '%'
                  ORDER BY fix_success_rate DESC, usage_count DESC
                  LIMIT 1",
            )?;
            let mut rows = stmt.query(params![command_template, stderr])?;
            let (pattern_id, old_rate, old_count): (i64, f64, i64) =
                if let Some(row) = rows.next()? {
                    (row.get(0)?, row.get(1)?, row.get(2)?)
                } else {
                    anyhow::bail!(
                        "No pattern found for command_template='{}' with matching stderr",
                        command_template
                    );
                };

            let score = if success { 1.0_f64 } else { 0.0_f64 };
            // Rolling average: clamp to [0.0, 1.0]
            let new_rate =
                ((old_rate * old_count as f64 + score) / (old_count as f64 + 1.0)).clamp(0.0, 1.0);

            conn.execute(
                "UPDATE patterns
                    SET fix_command = COALESCE(?1, fix_command),
                        fix_success_rate = ?2,
                        usage_count = usage_count + 1,
                        last_used = ?3
                  WHERE id = ?4",
                params![fix_command, new_rate, Utc::now().to_rfc3339(), pattern_id],
            )?;
            Ok(())
        })
        .await
    }

    /// Auto-detect when a successfully-executed command matches a known fix_command
    /// from a previously failed pattern. Increments the fix success rate.
    pub async fn auto_bump_fix_success(
        &self,
        command_template: &str,
        fix_success: bool,
    ) -> anyhow::Result<()> {
        let command_template = command_template.to_string();
        self.run_db(move |conn| {
            let score = if fix_success { 1.0_f64 } else { 0.0_f64 };
            conn.execute(
                "UPDATE patterns
                    SET fix_success_rate = CASE
                            WHEN usage_count > 0
                            THEN (fix_success_rate * usage_count + ?2) / (usage_count + 1)
                            ELSE ?2
                        END,
                        usage_count = usage_count + 1,
                        last_used = ?3
                  WHERE fix_command IS NOT NULL
                    AND fix_command = ?1",
                params![command_template, score, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
        .await
    }

    /// Count recovery attempts grouped by recovery code (for diagnostics).
    pub async fn recovery_attempt_counts(&self) -> anyhow::Result<Vec<(String, i64)>> {
        self.run_db(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT recovery_code, COUNT(*) as cnt
                 FROM recovery_attempts
                 GROUP BY recovery_code
                 ORDER BY cnt DESC",
            )?;
            let mut results = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                results.push((row.get(0)?, row.get(1)?));
            }
            Ok(results)
        })
        .await
    }

    /// Count patterns that have a known fix command.
    pub async fn patterns_with_fixes_count(&self) -> anyhow::Result<i64> {
        self.run_db(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM patterns WHERE fix_command IS NOT NULL",
                [],
                |row| row.get(0),
            )?;
            Ok(count)
        })
        .await
    }

    /// Average fix success rate across patterns that have a fix command.
    pub async fn average_fix_success_rate(&self) -> anyhow::Result<f64> {
        self.run_db(|conn| {
            let avg: f64 = conn.query_row(
                "SELECT COALESCE(AVG(fix_success_rate), 0.0)
                     FROM patterns
                     WHERE fix_command IS NOT NULL",
                [],
                |row| row.get(0),
            )?;
            Ok(avg)
        })
        .await
    }

    /// Count patterns grouped by recovery code (for diagnostics).
    pub async fn pattern_counts_by_code(&self) -> anyhow::Result<Vec<(String, i64)>> {
        self.run_db(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT recovery_code, COUNT(*) as cnt
                 FROM patterns
                 GROUP BY recovery_code
                 ORDER BY cnt DESC",
            )?;
            let mut results = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                results.push((row.get(0)?, row.get(1)?));
            }
            Ok(results)
        })
        .await
    }

    /// Log a command execution for audit purposes.
    pub async fn log_audit_entry(
        &self,
        command_hash: &str,
        command_template: &str,
        cwd: Option<&str>,
        exit_code: i32,
        recovery_code: &str,
        validation_passed: bool,
    ) -> anyhow::Result<()> {
        let command_hash = command_hash.to_string();
        let command_template = command_template.to_string();
        let cwd = cwd.unwrap_or("").to_string();
        let recovery_code = recovery_code.to_string();
        self.run_db(move |conn| {
            conn.execute(
                "INSERT INTO audit_log (command_hash, command_template, cwd, exit_code, recovery_code, validation_passed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    command_hash,
                    command_template,
                    cwd,
                    exit_code,
                    recovery_code,
                    validation_passed,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Get recent audit log entries.
    pub async fn recent_audit_entries(&self, limit: i64) -> anyhow::Result<Vec<AuditEntry>> {
        self.run_db(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, command_hash, command_template, cwd, exit_code, recovery_code,
                        validation_passed, executed_at
                 FROM audit_log
                 ORDER BY executed_at DESC
                 LIMIT ?1",
            )?;
            let mut entries = Vec::new();
            let mut rows = stmt.query(params![limit])?;
            while let Some(row) = rows.next()? {
                entries.push(AuditEntry {
                    id: row.get(0)?,
                    command_hash: row.get(1)?,
                    command_template: row.get(2)?,
                    cwd: row.get(3)?,
                    exit_code: row.get(4)?,
                    recovery_code: row.get(5)?,
                    validation_passed: row.get(6)?,
                    executed_at: row.get(7)?,
                });
            }
            Ok(entries)
        })
        .await
    }

    /// Set 0600 permissions on the database file (owner read/write only).
    #[cfg(unix)]
    fn set_restrictive_permissions(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "Warning: failed to set restrictive permissions on database: {}",
                e
            );
        }
    }

    #[cfg(not(unix))]
    fn set_restrictive_permissions(_path: &std::path::Path) {
        // Non-Unix: no-op
    }
}

#[derive(Debug, Clone)]
pub struct StoredOutput {
    pub output_id: String,
    pub execution_id: Option<String>,
    pub original_command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub command_hash: String,
    pub command_template: String,
    pub cwd: Option<String>,
    pub exit_code: i32,
    pub recovery_code: String,
    pub validation_passed: bool,
    pub executed_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_store() -> (Store, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = Store::new_at_path(db_path).unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn save_and_get_output() {
        let (store, _dir) = test_store();
        let id = store.next_output_id();
        store
            .save_output(&id, "test-id", "echo hello", "hello\n", "", 0)
            .await
            .unwrap();
        let output = store.get_output(&id).await.unwrap().unwrap();
        assert_eq!(output.stdout, "hello\n");
        assert_eq!(output.exit_code, 0);
    }

    #[tokio::test]
    async fn get_nonexistent_output() {
        let (store, _dir) = test_store();
        let result = store.get_output("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn save_and_find_pattern() {
        let (store, _dir) = test_store();
        let pattern = Pattern {
            id: None,
            command_hash: "abc123".to_string(),
            command_template: "cargo test".to_string(),
            recovery_code: "R24".to_string(),
            stderr_pattern: "FAILED".to_string(),
            fix_command: Some("cargo test -- --nocapture".to_string()),
            fix_success_rate: 0.8,
            last_used: Some(Utc::now()),
            usage_count: 1,
            platform_tag: Some("linux".to_string()),
        };
        store.save_pattern(&pattern).await.unwrap();
        let found = store
            .find_pattern("cargo test", "test FAILED")
            .await
            .unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.recovery_code, "R24");
        assert_eq!(
            found.fix_command,
            Some("cargo test -- --nocapture".to_string())
        );
    }

    #[tokio::test]
    async fn find_pattern_empty_stderr_matches_nothing() {
        let (store, _dir) = test_store();
        let pattern = Pattern {
            id: None,
            command_hash: "h".to_string(),
            command_template: "echo hi".to_string(),
            recovery_code: "R24".to_string(),
            stderr_pattern: "x".to_string(),
            fix_command: None,
            fix_success_rate: 0.0,
            last_used: Some(Utc::now()),
            usage_count: 1,
            platform_tag: Some("linux".to_string()),
        };
        store.save_pattern(&pattern).await.unwrap();
        let found = store.find_pattern("echo hi", "").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn pattern_count_starts_at_zero() {
        let (store, _dir) = test_store();
        assert_eq!(store.pattern_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn pattern_count_increments() {
        let (store, _dir) = test_store();
        let pattern = Pattern {
            id: None,
            command_hash: "abc".to_string(),
            command_template: "npm install".to_string(),
            recovery_code: "R24".to_string(),
            stderr_pattern: "npm ERR!".to_string(),
            fix_command: None,
            fix_success_rate: 0.0,
            last_used: Some(Utc::now()),
            usage_count: 1,
            platform_tag: Some("linux".to_string()),
        };
        store.save_pattern(&pattern).await.unwrap();
        assert_eq!(store.pattern_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn log_audit_entry_works() {
        let (store, _dir) = test_store();
        store
            .log_audit_entry("hash123", "echo hello", Some("/tmp"), 0, "R10", true)
            .await
            .unwrap();
        let entries = store.recent_audit_entries(10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].recovery_code, "R10");
    }

    #[tokio::test]
    async fn log_recovery_attempt_works() {
        let (store, _dir) = test_store();
        store
            .log_recovery_attempt("R22", "gh pr view", "install gh")
            .await
            .unwrap();
        let counts = store.recovery_attempt_counts().await.unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].0, "R22");
        assert_eq!(counts[0].1, 1);
    }

    #[tokio::test]
    async fn latest_output_returns_most_recent() {
        let (store, _dir) = test_store();
        let id1 = store.next_output_id();
        store
            .save_output(&id1, "test-1", "first", "out1", "", 0)
            .await
            .unwrap();
        let id2 = store.next_output_id();
        store
            .save_output(&id2, "test-2", "second", "out2", "", 0)
            .await
            .unwrap();
        let latest = store.latest_output().await.unwrap().unwrap();
        assert_eq!(latest.stdout, "out2");
    }

    #[tokio::test]
    async fn previous_output_returns_prior() {
        let (store, _dir) = test_store();
        let id1 = store.next_output_id();
        store
            .save_output(&id1, "test-1", "first", "out1", "", 0)
            .await
            .unwrap();
        let id2 = store.next_output_id();
        store
            .save_output(&id2, "test-2", "second", "out2", "", 0)
            .await
            .unwrap();
        let prev = store.previous_output(&id2).await.unwrap().unwrap();
        assert_eq!(prev.stdout, "out1");
    }

    #[tokio::test]
    async fn save_pattern_upserts() {
        let (store, _dir) = test_store();
        let pattern = Pattern {
            id: None,
            command_hash: "abc".to_string(),
            command_template: "cargo test".to_string(),
            recovery_code: "R24".to_string(),
            stderr_pattern: "FAILED".to_string(),
            fix_command: None,
            fix_success_rate: 0.0,
            last_used: Some(Utc::now()),
            usage_count: 1,
            platform_tag: Some("linux".to_string()),
        };
        store.save_pattern(&pattern).await.unwrap();
        store.save_pattern(&pattern).await.unwrap();
        // Should still be 1 pattern (upsert), but usage_count incremented
        assert_eq!(store.pattern_count().await.unwrap(), 1);
    }
}
