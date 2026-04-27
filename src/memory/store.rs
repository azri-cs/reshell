use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use super::pattern::{Pattern, current_platform_tag};
use chrono::Utc;

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
             PRAGMA busy_timeout = 5000;"
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
        let _ = conn.execute("ALTER TABLE patterns ADD COLUMN platform_tag TEXT DEFAULT 'unknown'", []);
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_patterns_template_stderr
             ON patterns(command_template, stderr_pattern)",
            [],
        );
        conn.execute(
            "CREATE TABLE IF NOT EXISTS outputs (
                output_id TEXT PRIMARY KEY,
                original_command TEXT NOT NULL,
                stdout TEXT,
                stderr TEXT,
                exit_code INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
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
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn next_output_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub async fn find_pattern(
        &self,
        command_template: &str,
        stderr: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let conn = self.conn.lock().await;
        let platform = current_platform_tag();
        let mut stmt = conn.prepare_cached(
            "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                    fix_command, fix_success_rate, last_used, usage_count, platform_tag
               FROM patterns
              WHERE command_template = ?1
                AND (?2 LIKE '%' || stderr_pattern || '%' OR stderr_pattern LIKE '%' || ?2 || '%')
              ORDER BY (platform_tag = ?3) DESC, fix_success_rate DESC, usage_count DESC
              LIMIT 1"
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
    }

    pub async fn save_pattern(&self, pattern: &Pattern) -> anyhow::Result<()> {
        let conn = self.conn.lock().await;
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
    }

    pub async fn save_output(
        &self,
        output_id: &str,
        original_command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO outputs (output_id, original_command, stdout, stderr, exit_code)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(output_id) DO UPDATE SET
                stdout = excluded.stdout,
                stderr = excluded.stderr,
                exit_code = excluded.exit_code",
            params![output_id, original_command, stdout, stderr, exit_code],
        )?;
        Ok(())
    }

    pub async fn get_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT output_id, original_command, stdout, stderr, exit_code, created_at
             FROM outputs WHERE output_id = ?1"
        )?;
        let mut rows = stmt.query(params![output_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(StoredOutput {
                output_id: row.get(0)?,
                original_command: row.get(1)?,
                stdout: row.get(2)?,
                stderr: row.get(3)?,
                exit_code: row.get(4)?,
                created_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn previous_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT output_id, original_command, stdout, stderr, exit_code, created_at
             FROM outputs
             WHERE rowid < (SELECT rowid FROM outputs WHERE output_id = ?1)
             ORDER BY rowid DESC
             LIMIT 1"
        )?;
        let mut rows = stmt.query(params![output_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(StoredOutput {
                output_id: row.get(0)?,
                original_command: row.get(1)?,
                stdout: row.get(2)?,
                stderr: row.get(3)?,
                exit_code: row.get(4)?,
                created_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn latest_output(&self) -> anyhow::Result<Option<StoredOutput>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT output_id, original_command, stdout, stderr, exit_code, created_at
             FROM outputs ORDER BY created_at DESC, rowid DESC LIMIT 1"
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(StoredOutput {
                output_id: row.get(0)?,
                original_command: row.get(1)?,
                stdout: row.get(2)?,
                stderr: row.get(3)?,
                exit_code: row.get(4)?,
                created_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn pattern_count(&self) -> anyhow::Result<i64> {
        let conn = self.conn.lock().await;
        let count = conn
            .query_row("SELECT COUNT(*) FROM patterns", [], |row| row.get(0))?;
        Ok(count)
    }

    pub async fn find_pattern_exact(
        &self,
        command_template: &str,
        stderr_pattern: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                    fix_command, fix_success_rate, last_used, usage_count, platform_tag
             FROM patterns
             WHERE command_template = ?1 AND stderr_pattern = ?2
             LIMIT 1"
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
    }

    /// Log a recovery suggestion being served to the agent.
    pub async fn log_recovery_attempt(
        &self,
        recovery_code: &str,
        original_command: &str,
        suggested_action: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO recovery_attempts (recovery_code, original_command, suggested_action)
             VALUES (?1, ?2, ?3)",
            params![recovery_code, original_command, suggested_action],
        )?;
        Ok(())
    }

    /// Count recovery attempts grouped by recovery code (for diagnostics).
    pub async fn recovery_attempt_counts(&self) -> anyhow::Result<Vec<(String, i64)>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT recovery_code, COUNT(*) as cnt
             FROM recovery_attempts
             GROUP BY recovery_code
             ORDER BY cnt DESC"
        )?;
        let mut results = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            results.push((row.get(0)?, row.get(1)?));
        }
        Ok(results)
    }

    /// Set 0600 permissions on the database file (owner read/write only).
    #[cfg(unix)]
    fn set_restrictive_permissions(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!("Warning: failed to set restrictive permissions on database: {}", e);
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
    pub original_command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub created_at: String,
}
