use anyhow::Result;
use rusqlite::{params, Connection};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                output TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS agent_log (
                id INTEGER PRIMARY KEY,
                role TEXT NOT NULL,
                content TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        conn.execute("PRAGMA journal_mode=WAL", [])?;
        conn.execute("PRAGMA busy_timeout=5000", [])?;
        Ok(Self { conn })
    }

    pub fn insert_task(&self, kind: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO tasks (kind, status) VALUES (?1, 'running')",
            params![kind],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_task_status(&self, id: i64, status: &str, output: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = ?1, output = ?2 WHERE id = ?3",
            params![status, output, id],
        )?;
        Ok(())
    }

    pub fn cancel_task(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled' WHERE id = ?1 AND status IN ('pending', 'running')",
            params![id],
        )?;
        Ok(affected > 0)
    }

    pub fn delete_task(&self, id: i64) -> Result<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    pub fn retry_task(&self, id: i64) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind FROM tasks WHERE id = ?1 AND status IN ('failed', 'error', 'cancelled')",
        )?;
        let kind: Option<String> = stmt
            .query_map(params![id], |row| row.get(0))?
            .filter_map(Result::ok)
            .next();
        drop(stmt);

        match kind {
            Some(k) => {
                self.conn.execute(
                    "INSERT INTO tasks (kind, status) VALUES (?1, 'pending')",
                    params![k],
                )?;
                Ok(Some(self.conn.last_insert_rowid()))
            }
            None => Ok(None),
        }
    }

    pub fn get_task(&self, id: i64) -> Result<Option<(i64, String, String, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, status, output FROM tasks WHERE id = ?1")?;
        let row = stmt
            .query_map(params![id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(Result::ok)
            .next();
        Ok(row)
    }

    pub fn get_history(&self, limit: i64) -> Result<Vec<(i64, String, String, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, status, output FROM tasks ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn get_running_tasks(&self) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE status IN ('pending', 'running')")?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn log_agent_message(&self, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent_log (role, content) VALUES (?1, ?2)",
            params![role, content],
        )?;
        Ok(())
    }

    pub fn get_agent_history(&self, limit: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT role, content FROM agent_log ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt
            .query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }
}
