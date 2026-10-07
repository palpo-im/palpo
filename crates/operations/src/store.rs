use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::{Result, fail};

/// Uses the Node service's exact `.lock` convention. Never steal a stale lock:
/// the operator must establish that the old process has stopped before recovery.
struct OwnerLock {
    path: PathBuf,
    _file: File,
}
impl Drop for OwnerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct Store {
    db: Connection,
    _owner: Option<OwnerLock>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = std::fs::DirBuilder::new();
                builder
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .map_err(|_| fail(503, "workflow_directory_unavailable"))?;
            }
            #[cfg(not(unix))]
            std::fs::create_dir_all(parent)
                .map_err(|_| fail(503, "workflow_directory_unavailable"))?;
        }
        let path = parent
            .canonicalize()
            .map_err(|_| fail(503, "workflow_directory_unavailable"))?
            .join(
                path.file_name()
                    .ok_or_else(|| fail(400, "invalid_database_path"))?,
            );
        // Do not let a symlink create a second lock name for the same database.
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail(409, "database_symlink_forbidden"));
        }
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&lock_path)
            .map_err(|_| fail(409, "workflow_database_in_use"))?;
        let mut owner = OwnerLock {
            path: lock_path,
            _file: file,
        };
        writeln!(owner._file, "{}", std::process::id())
            .map_err(|_| fail(503, "workflow_lock_unavailable"))?;
        // Precreate a new database privately before SQLite can write secrets.
        if !path.exists() {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(&path)
                .map_err(|_| fail(503, "workflow_database_unavailable"))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| fail(503, "workflow_database_unavailable"))?;
        }
        let db = Connection::open(&path)?;
        Self::initialize(db, Some(owner))
    }

    pub fn memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?, None)
    }

    fn initialize(db: Connection, owner: Option<OwnerLock>) -> Result<Self> {
        db.busy_timeout(Duration::from_secs(5))?;
        // A persistent cutover trigger calls this connection-local function.
        // Old Node/Rust binaries do not register it and cannot write through
        // the migrated database even after its process lock has been released.
        db.create_scalar_function(
            "palpo_rust_workflow_writer_v1",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            |_| Ok(1i64),
        )?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS state(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL);")?;
        let store = Self { db, _owner: owner };
        store.read()?;
        Ok(store)
    }

    pub fn read(&self) -> Result<Value> {
        let body: Option<String> = self
            .db
            .query_row("SELECT body FROM state WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let state: Value = match body {
            Some(body) => {
                serde_json::from_str(&body).map_err(|_| fail(503, "workflow_state_invalid"))?
            }
            None => json!({"version": 1, "fleets": {}, "audit": []}),
        };
        if state["version"] != 1 || !state.is_object() {
            return Err(fail(503, "unsupported_workflow_database_version"));
        }
        Ok(state)
    }

    /// No external I/O is allowed in this synchronous closure. Read current state
    /// under BEGIN IMMEDIATE and persist the entire decision/outbox atomically.
    pub fn transaction<T>(&mut self, operation: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
        self.transaction_sql(|state, _| operation(state))
    }

    pub(crate) fn inspect_sql<T>(
        &mut self,
        inspect: impl FnOnce(&Value, &rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let body: Option<String> = tx
            .query_row("SELECT body FROM state WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let state = body
            .map(|body| serde_json::from_str::<Value>(&body))
            .transpose()?
            .unwrap_or_else(|| json!({"version":1,"fleets":{},"audit":[]}));
        let result = inspect(&state, &tx)?;
        tx.rollback()?;
        Ok(result)
    }

    /// Commit workflow state and delivery rows under the same SQLite writer.
    pub(crate) fn transaction_sql<T>(
        &mut self,
        operation: impl FnOnce(&mut Value, &rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let body: Option<String> = tx
            .query_row("SELECT body FROM state WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let mut state = match body {
            Some(body) => serde_json::from_str::<Value>(&body)
                .map_err(|_| fail(503, "workflow_state_invalid"))?,
            None => json!({"version":1,"fleets":{},"audit":[]}),
        };
        if state["version"] != 1 {
            return Err(fail(503, "unsupported_workflow_database_version"));
        }
        let result = operation(&mut state, &tx)?;
        tx.execute("INSERT INTO state(id,body) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            params![serde_json::to_string(&state)?])?;
        tx.commit()?;
        Ok(result)
    }

    pub fn bind(&mut self, server: &str, origin: &str) -> Result<()> {
        self.transaction(|state| {
            let binding = json!({"serverName":server,"palpoOrigin":origin});
            if !state["serverBinding"].is_null() && state["serverBinding"] != binding {
                return Err(fail(409, "workflow_server_binding_mismatch"));
            }
            state["serverBinding"] = binding;
            Ok(())
        })
    }
}

/// Install only inside the semantic migration transaction. The marker and
/// triggers roll back with the import; failed validation never strands Node.
pub(crate) fn fence_legacy_writers(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch(crate::outbound::SCHEMA)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_writer (
        id INTEGER PRIMARY KEY CHECK(id=1),
        engine TEXT NOT NULL CHECK(engine='rust'),
        version INTEGER NOT NULL CHECK(version=1));
        INSERT OR IGNORE INTO workflow_writer VALUES(1,'rust',1);",
    )?;
    for table in [
        "state",
        "fleet_delivery",
        "workflow_writer",
        "workflow_migrations",
    ] {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            continue;
        }
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            tx.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS rust_writer_{table}_{operation}
                BEFORE {operation} ON {table}
                WHEN palpo_rust_workflow_writer_v1() != 1
                BEGIN SELECT RAISE(ABORT,'Rust owns workflow writes'); END;"
            ))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutover_fences_old_connections_and_retains_current_decisions_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admin.sqlite");
        let mut store = Store::open(&path).unwrap();
        store
            .transaction(|state| {
                state["history"] = json!(["legacy"]);
                Ok(())
            })
            .unwrap();
        let old = Connection::open(&path).unwrap();
        // A rejected migration does not claim the database.
        let refused: Result<()> = store.transaction_sql(|_, tx| {
            fence_legacy_writers(tx)?;
            Err(fail(409, "fixture_validation_failed"))
        });
        assert!(refused.is_err());
        old.execute("UPDATE state SET body=body", []).unwrap();
        store
            .transaction_sql(|state, tx| {
                fence_legacy_writers(tx)?;
                state["history"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("rust decision"));
                Ok(())
            })
            .unwrap();
        assert!(old.execute("UPDATE state SET body=body", []).is_err());
        assert!(old.execute("DELETE FROM fleet_delivery", []).is_err());
        assert!(old.execute("DELETE FROM workflow_writer", []).is_err());
        let checkpoint = store.read().unwrap();
        drop(store);
        // Inspection is possible. Releasing the service lock never transfers
        // decision authority back to a stale writer or snapshot.
        assert_eq!(
            serde_json::from_str::<Value>(
                &old.query_row("SELECT body FROM state", [], |row| row.get::<_, String>(0))
                    .unwrap()
            )
            .unwrap(),
            checkpoint
        );
        assert!(old.execute("DELETE FROM state", []).is_err());
        let mut reopened = Store::open(&path).unwrap();
        reopened
            .transaction(|state| {
                assert_eq!(*state, checkpoint);
                Ok(())
            })
            .unwrap();
    }
}
