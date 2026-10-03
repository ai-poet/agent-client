//! Reads the welcome screen's activity overview out of `app.db`.
//!
//! Fork addition; the record shape and every fold over it live in
//! `waku_protocol::activity_overview`, shared with the desktop. This is only
//! the read: every finished user and assistant message, bucketed by local
//! day, hour and its session's model, and every session that holds one.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use chrono::{Local, TimeZone, Timelike};
use rusqlite::{Connection, OpenFlags};
pub use waku_protocol::activity_overview::ActivityRecords;
use waku_protocol::activity_overview::{MessageBucket, SessionRecord};

/// Read the whole history's activity. Opens its own read-only connection,
/// so it neither waits on nor delays the writer (the database is in WAL
/// mode).
pub fn load(path: &Path) -> io::Result<ActivityRecords> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(io::Error::other)?;
    load_from(&connection, &Local)
}

/// The part of a stored model id a person would recognise: the built-in
/// agent stores `platform::model` and `custom:<endpoint>::model`.
fn display_model(stored: &str) -> String {
    stored.rsplit("::").next().unwrap_or(stored).trim().to_owned()
}

#[derive(Default)]
struct Models {
    names: Vec<String>,
    index: HashMap<String, u32>,
}

impl Models {
    fn intern(&mut self, model: Option<String>) -> u32 {
        let model = model.map(|model| display_model(&model)).unwrap_or_default();
        if let Some(&index) = self.index.get(&model) {
            return index;
        }
        let index = self.names.len() as u32;
        self.names.push(model.clone());
        self.index.insert(model, index);
        index
    }
}

fn load_from<Tz: TimeZone>(connection: &Connection, zone: &Tz) -> io::Result<ActivityRecords> {
    let local = |seconds: i64| {
        zone.timestamp_opt(seconds, 0)
            .single()
            .map(|time| time.naive_local())
    };
    let mut models = Models::default();

    let mut statement = connection
        .prepare(
            "SELECT messages.created_at, sessions.model
               FROM messages
               INNER JOIN sessions ON sessions.id = messages.session_id
              WHERE messages.streaming = 0
                AND messages.role IN ('user', 'assistant')",
        )
        .map_err(io::Error::other)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(io::Error::other)?;
    let mut counts: HashMap<(chrono::NaiveDate, u8, u32), u32> = HashMap::new();
    for row in rows {
        let (created_at, model) = row.map_err(io::Error::other)?;
        let Some(time) = local(created_at) else {
            continue;
        };
        let model = models.intern(model);
        *counts
            .entry((time.date(), time.hour() as u8, model))
            .or_default() += 1;
    }
    let mut buckets = counts
        .into_iter()
        .map(|((day, hour, model), messages)| MessageBucket {
            day,
            hour,
            model,
            messages,
        })
        .collect::<Vec<_>>();
    buckets.sort_by_key(|bucket| (bucket.day, bucket.hour, bucket.model));

    let mut statement = connection
        .prepare(
            "SELECT sessions.created_at, sessions.model
               FROM sessions
              WHERE EXISTS (
                    SELECT 1 FROM messages
                     WHERE messages.session_id = sessions.id
                       AND messages.streaming = 0
                       AND messages.role IN ('user', 'assistant'))",
        )
        .map_err(io::Error::other)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(io::Error::other)?;
    let mut sessions = Vec::new();
    for row in rows {
        let (created_at, model) = row.map_err(io::Error::other)?;
        let Some(time) = local(created_at) else {
            continue;
        };
        sessions.push(SessionRecord {
            day: time.date(),
            model: models.intern(model),
        });
    }
    Ok(ActivityRecords {
        buckets,
        sessions,
        models: models.names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate};

    #[test]
    fn load_buckets_messages_by_local_day_hour_and_model() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, model TEXT, created_at INTEGER NOT NULL);
                 CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL,
                                        created_at INTEGER NOT NULL, streaming INTEGER NOT NULL);",
            )
            .unwrap();
        // 2026-10-03 23:30 UTC is 2026-10-04 07:30 in UTC+8.
        let late = 1_791_070_200;
        connection
            .execute_batch(&format!(
                "INSERT INTO sessions VALUES ('a', 'deepseek::deepseek-v4-flash', {late});
                 INSERT INTO sessions VALUES ('b', NULL, {late});
                 INSERT INTO sessions VALUES ('empty', 'gpt-6-sol', {late});
                 INSERT INTO messages VALUES ('1', 'a', 'user', {late}, 0);
                 INSERT INTO messages VALUES ('2', 'a', 'assistant', {late}, 0);
                 INSERT INTO messages VALUES ('3', 'a', 'system', {late}, 0);
                 INSERT INTO messages VALUES ('4', 'a', 'assistant', {late}, 1);
                 INSERT INTO messages VALUES ('5', 'b', 'user', {late}, 0);
                 INSERT INTO messages VALUES ('6', 'empty', 'system', {late}, 0);"
            ))
            .unwrap();
        let zone = FixedOffset::east_opt(8 * 3600).unwrap();
        let records = load_from(&connection, &zone).unwrap();

        let flash = records
            .models
            .iter()
            .position(|model| model == "deepseek-v4-flash")
            .unwrap() as u32;
        assert!(records.buckets.contains(&MessageBucket {
            day: NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
            hour: 7,
            model: flash,
            messages: 2,
        }));
        // System, streaming and message-less sessions do not count.
        assert_eq!(
            records.buckets.iter().map(|b| b.messages).sum::<u32>(),
            3
        );
        assert_eq!(records.sessions.len(), 2);
    }
}
