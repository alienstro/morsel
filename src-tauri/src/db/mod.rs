//! The SQLite database of Morsel: one file in the app data folder.

pub mod library;
pub mod meetings;
pub mod parts;
pub mod templates;

#[cfg(test)]
mod tests;

use crate::types::Result;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// One connection for the whole app. Each user of it holds the lock for one short query.
pub type SharedDb = Arc<Mutex<Connection>>;
use std::time::{SystemTime, UNIX_EPOCH};

/// The schema steps, oldest first. Step 2 also writes the built-in template values.
pub(crate) fn migrations() -> Vec<M<'static>> {
    vec![
        M::up(include_str!("migrations.sql")),
        M::up_with_hook(include_str!("migrations_2.sql"), templates::write_built_ins),
        M::up(include_str!("migrations_3.sql")),
        M::up(include_str!("migrations_4.sql")),
    ]
}

/// Opens the database, turns on WAL and foreign keys, and runs the migrations.
pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Migrations::new(migrations())
        .to_latest(&mut conn)
        .map_err(|e| crate::types::MorselError::Db(e.to_string()))?;
    Ok(conn)
}

/// The current time in milliseconds since 1970.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
