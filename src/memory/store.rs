use rusqlite::{Connection, params};
use std::path::PathBuf;
use super::pattern::Pattern;
use chrono::Utc;

pub struct Store {
    conn: Connection,
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
        let conn = Connection::open(path)?;
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
                usage_count INTEGER DEFAULT 1
            )",
            [],
        )?;
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
        Ok(Self { conn })
    }

    pub fn next_output_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn find_pattern(
        &self,
        command_template: &str,
        stderr: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                    fix_command, fix_success_rate, last_used, usage_count
              FROM patterns
             WHERE command_template = ?1
             ORDER BY fix_success_rate DESC, usage_count DESC"
        )?;
        let mut rows = stmt.query(params![command_template])?;
        while let Some(row) = rows.next()? {
            let pattern = Pattern {
                id: row.get(0)?,
                command_hash: row.get(1)?,
                command_template: row.get(2)?,
                recovery_code: row.get(3)?,
                stderr_pattern: row.get(4)?,
                fix_command: row.get(5)?,
                fix_success_rate: row.get(6)?,
                last_used: row.get(7)?,
                usage_count: row.get(8)?,
            };

            if stderr.contains(&pattern.stderr_pattern) || pattern.stderr_pattern.contains(stderr) {
                return Ok(Some(pattern));
            }
        }

        Ok(None)
    }

    pub fn save_pattern(&self, pattern: &Pattern) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO patterns (command_hash, command_template, recovery_code, stderr_pattern,
                                   fix_command, fix_success_rate, last_used, usage_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
              ON CONFLICT(command_template, stderr_pattern) DO UPDATE SET
                 recovery_code = excluded.recovery_code,
                 fix_command = excluded.fix_command,
                 fix_success_rate = excluded.fix_success_rate,
                 last_used = excluded.last_used,
                 usage_count = usage_count + 1",
            params![
                &pattern.command_hash,
                &pattern.command_template,
                &pattern.recovery_code,
                &pattern.stderr_pattern,
                pattern.fix_command.as_ref(),
                pattern.fix_success_rate,
                Utc::now().to_rfc3339(),
                pattern.usage_count,
            ],
        )?;
        Ok(())
    }

    pub fn save_output(
        &self,
        output_id: &str,
        original_command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> anyhow::Result<()> {
        self.conn.execute(
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

    pub fn get_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let mut stmt = self.conn.prepare(
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

    pub fn previous_output(&self, output_id: &str) -> anyhow::Result<Option<StoredOutput>> {
        let mut stmt = self.conn.prepare(
            "SELECT prior.output_id, prior.original_command, prior.stdout, prior.stderr, prior.exit_code, prior.created_at
             FROM outputs current
             JOIN outputs prior
               ON prior.rowid < current.rowid
             WHERE current.output_id = ?1
             ORDER BY prior.rowid DESC
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

    pub fn latest_output(&self) -> anyhow::Result<Option<StoredOutput>> {
        let mut stmt = self.conn.prepare(
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

    pub fn pattern_count(&self) -> anyhow::Result<i64> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM patterns", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn find_pattern_exact(
        &self,
        command_template: &str,
        stderr_pattern: &str,
    ) -> anyhow::Result<Option<Pattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command_hash, command_template, recovery_code, stderr_pattern,
                    fix_command, fix_success_rate, last_used, usage_count
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
            }))
        } else {
            Ok(None)
        }
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
