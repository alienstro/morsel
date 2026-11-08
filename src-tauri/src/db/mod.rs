//! The SQLite database of Morsel: one file in the app data folder.

pub mod meetings;

#[cfg(test)]
mod tests;

use crate::types::Result;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("migrations.sql"))];

/// Opens the database, turns on WAL and foreign keys, and runs the migrations.
pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Migrations::from_slice(MIGRATIONS)
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
