//! Reads the welcome screen's activity overview out of `app.db`.
//!
//! Fork addition; the record shape and every fold over it live in
//! `waku_protocol::activity_overview`, shared with the desktop.
//!
//! The overview is history, so it must not shrink when a task is deleted or a
//! conversation is rewound — and both delete `messages` rows. Its source is
//! therefore a ledger of its own, two fork tables that only ever grow:
//! `fork_activity_messages` (one row per finished user or assistant message)
//! and `fork_activity_sessions` (one row per session that holds one, with its
//! last known model). Triggers on `messages` and `sessions` fill them inside
//! the transaction that writes the message, so every write path is covered and
//! the store's save is untouched. [`ensure_ledger`] creates them each time the
//! store opens the database, backfilling from the history still on disk the
//! first time; they are not in `db/schema.ts`, whose drizzle migrations belong
//! to upstream.
//!
//! A message counts once, by id, however often its row is rewritten. A message
//! older than its session is a copy a fork carried over, not new activity, and
//! is never recorded.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use chrono::{Local, TimeZone, Timelike};
use rusqlite::{Connection, OpenFlags};
pub use waku_protocol::activity_overview::ActivityRecords;
use waku_protocol::activity_overview::{MessageBucket, SessionRecord};

/// The ledger's tables, and the backfill from the history on disk. Run only
/// when the tables are missing; harmless if it runs again.
const LEDGER_TABLES: &str = "
    CREATE TABLE IF NOT EXISTS fork_activity_messages (
        message_id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        created_at INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS fork_activity_sessions (
        session_id TEXT PRIMARY KEY,
        created_at INTEGER NOT NULL,
        model      TEXT
    ) WITHOUT ROWID;
    INSERT OR IGNORE INTO fork_activity_messages(message_id, session_id, created_at)
        SELECT messages.id, messages.session_id, messages.created_at
          FROM messages
          INNER JOIN sessions ON sessions.id = messages.session_id
         WHERE messages.streaming = 0
           AND messages.role IN ('user', 'assistant')
           AND messages.created_at >= sessions.created_at;
    INSERT OR IGNORE INTO fork_activity_sessions(session_id, created_at, model)
        SELECT id, created_at, model
          FROM sessions
         WHERE id IN (SELECT session_id FROM fork_activity_messages);";

/// Records `NEW` — a finished user or assistant message — unless it is older
/// than its session. The session's row is written once; its model is kept
/// current by `fork_activity_session_model`, so a later message writes only
/// its own row.
///
/// Guarded by `NOT EXISTS` rather than `INSERT OR IGNORE`: inside a trigger,
/// the conflict policy of the statement that fired it wins, and the store's
/// message upsert would turn an ignored duplicate into an error that fails
/// the whole save.
const RECORD_MESSAGE: &str = "
    INSERT INTO fork_activity_messages(message_id, session_id, created_at)
        SELECT NEW.id, NEW.session_id, NEW.created_at
          FROM sessions
         WHERE sessions.id = NEW.session_id
           AND NEW.created_at >= sessions.created_at
           AND NOT EXISTS (
               SELECT 1 FROM fork_activity_messages WHERE message_id = NEW.id);
    INSERT INTO fork_activity_sessions(session_id, created_at, model)
        SELECT sessions.id, sessions.created_at, sessions.model
          FROM sessions
         WHERE sessions.id = NEW.session_id
           AND NEW.created_at >= sessions.created_at
           AND NOT EXISTS (
               SELECT 1 FROM fork_activity_sessions WHERE session_id = NEW.session_id);";

/// The triggers, created on every open: a migration that rebuilds `messages`
/// or `sessions` drops the triggers on it.
fn ledger_triggers() -> String {
    format!(
        "CREATE TRIGGER IF NOT EXISTS fork_activity_message_inserted
             AFTER INSERT ON messages
             WHEN NEW.streaming = 0 AND NEW.role IN ('user', 'assistant')
         BEGIN {RECORD_MESSAGE}
         END;
         CREATE TRIGGER IF NOT EXISTS fork_activity_message_finished
             AFTER UPDATE OF streaming ON messages
             WHEN OLD.streaming <> 0 AND NEW.streaming = 0
              AND NEW.role IN ('user', 'assistant')
         BEGIN {RECORD_MESSAGE}
         END;
         CREATE TRIGGER IF NOT EXISTS fork_activity_session_model
             AFTER UPDATE OF model ON sessions
             WHEN NEW.model IS NOT NULL AND NEW.model IS NOT OLD.model
         BEGIN
             UPDATE fork_activity_sessions SET model = NEW.model WHERE session_id = NEW.id;
         END;"
    )
}

/// Make sure the ledger exists and is kept. Called by the store each time it
/// opens the database, after the migrations.
pub fn ensure_ledger(connection: &Connection) -> io::Result<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(io::Error::other)?;
    if !ledger_exists(&transaction)? {
        transaction
            .execute_batch(LEDGER_TABLES)
            .map_err(io::Error::other)?;
    }
    transaction
        .execute_batch(&ledger_triggers())
        .map_err(io::Error::other)?;
    transaction.commit().map_err(io::Error::other)
}

fn ledger_exists(connection: &Connection) -> io::Result<bool> {
    connection
        .query_row(
            "SELECT COUNT(*) = 2 FROM sqlite_master
              WHERE type = 'table'
                AND name IN ('fork_activity_messages', 'fork_activity_sessions')",
            [],
            |row| row.get(0),
        )
        .map_err(io::Error::other)
}

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

/// Every recorded message with its session's last known model.
const LEDGER_MESSAGES: &str = "
    SELECT fork_activity_messages.created_at, fork_activity_sessions.model
      FROM fork_activity_messages
      LEFT JOIN fork_activity_sessions
        ON fork_activity_sessions.session_id = fork_activity_messages.session_id";
const LEDGER_SESSIONS: &str = "SELECT created_at, model FROM fork_activity_sessions";

/// Without a ledger — a database this build has not opened for writing yet —
/// the history as it stands: every finished user and assistant message with
/// its session's current model, and every session that holds one.
const LIVE_MESSAGES: &str = "
    SELECT messages.created_at, sessions.model
      FROM messages
      INNER JOIN sessions ON sessions.id = messages.session_id
     WHERE messages.streaming = 0
       AND messages.role IN ('user', 'assistant')";
const LIVE_SESSIONS: &str = "
    SELECT sessions.created_at, sessions.model
      FROM sessions
     WHERE EXISTS (
           SELECT 1 FROM messages
            WHERE messages.session_id = sessions.id
              AND messages.streaming = 0
              AND messages.role IN ('user', 'assistant'))";

fn load_from<Tz: TimeZone>(connection: &Connection, zone: &Tz) -> io::Result<ActivityRecords> {
    let (messages_query, sessions_query) = if ledger_exists(connection)? {
        (LEDGER_MESSAGES, LEDGER_SESSIONS)
    } else {
        (LIVE_MESSAGES, LIVE_SESSIONS)
    };
    let local = |seconds: i64| {
        zone.timestamp_opt(seconds, 0)
            .single()
            .map(|time| time.naive_local())
    };
    let mut models = Models::default();

    let mut statement = connection
        .prepare(messages_query)
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
        .prepare(sessions_query)
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
    use std::fs;
    use std::path::PathBuf;

    use chrono::{FixedOffset, NaiveDate};
    use uuid::Uuid;

    use super::*;
    use crate::model::{MessageRole, ProviderKind, ProviderResumeCursor, TurnStatus};
    use crate::persistence::{PersistedState, StateStore};

    // 2026-10-03 23:30 UTC is 2026-10-04 07:30 in UTC+8.
    const LATE: i64 = 1_791_070_200;

    /// The tables as far as the overview reads them.
    fn history() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, model TEXT, created_at INTEGER NOT NULL);
                 CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL,
                                        created_at INTEGER NOT NULL, streaming INTEGER NOT NULL);
                 INSERT INTO sessions VALUES ('a', 'deepseek::deepseek-v4-flash', {LATE});
                 INSERT INTO sessions VALUES ('b', NULL, {LATE});
                 INSERT INTO sessions VALUES ('empty', 'gpt-6-sol', {LATE});
                 INSERT INTO messages VALUES ('1', 'a', 'user', {LATE}, 0);
                 INSERT INTO messages VALUES ('2', 'a', 'assistant', {LATE}, 0);
                 INSERT INTO messages VALUES ('3', 'a', 'system', {LATE}, 0);
                 INSERT INTO messages VALUES ('4', 'a', 'assistant', {LATE}, 1);
                 INSERT INTO messages VALUES ('5', 'b', 'user', {LATE}, 0);
                 INSERT INTO messages VALUES ('6', 'empty', 'system', {LATE}, 0);"
            ))
            .unwrap();
        connection
    }

    fn zone() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// Messages and sessions the overview counts.
    fn totals(records: &ActivityRecords) -> (u32, usize) {
        (
            records.buckets.iter().map(|bucket| bucket.messages).sum(),
            records.sessions.len(),
        )
    }

    fn assert_history_counts(records: &ActivityRecords) {
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
        assert_eq!(totals(records), (3, 2));
    }

    #[test]
    fn without_a_ledger_the_history_on_disk_is_read() {
        let connection = history();
        assert_history_counts(&load_from(&connection, &zone()).unwrap());
    }

    #[test]
    fn the_ledger_is_backfilled_once_and_outlives_the_history() {
        let connection = history();
        // A fork's copy: older than the session it was copied into.
        connection
            .execute_batch(&format!(
                "INSERT INTO sessions VALUES ('fork', 'gpt-6-sol', {});
                 INSERT INTO messages VALUES ('7', 'fork', 'user', {LATE}, 0);",
                LATE + 600
            ))
            .unwrap();
        ensure_ledger(&connection).unwrap();
        ensure_ledger(&connection).unwrap();
        assert_history_counts(&load_from(&connection, &zone()).unwrap());

        // The streaming answer finishes, then everything is deleted.
        connection
            .execute("UPDATE messages SET streaming = 0 WHERE id = '4'", [])
            .unwrap();
        connection
            .execute_batch("DELETE FROM messages; DELETE FROM sessions;")
            .unwrap();
        let records = load_from(&connection, &zone()).unwrap();
        assert_eq!(totals(&records), (4, 2));
        assert!(records.models.iter().any(|model| model == "deepseek-v4-flash"));
    }

    fn temporary_store() -> (PathBuf, StateStore, PersistedState) {
        let directory = std::env::temp_dir().join(format!("waku-activity-{}", Uuid::new_v4()));
        let store = StateStore::daemon(directory.join("app.db"));
        (
            directory,
            store,
            PersistedState::fresh(PathBuf::from("/tmp/project")),
        )
    }

    fn stored_totals(directory: &Path) -> (u32, usize) {
        totals(&load(&directory.join("app.db")).unwrap())
    }

    fn index_of(state: &PersistedState, id: Uuid) -> usize {
        state
            .sessions
            .iter()
            .position(|session| session.id == id)
            .unwrap()
    }

    #[test]
    fn rewound_and_deleted_tasks_keep_their_activity() {
        let (directory, store, mut state) = temporary_store();
        let session = &mut state.sessions[0];
        session.begin_turn("keep");
        session.push_message(MessageRole::Assistant, "kept");
        session.finish_active_turn(TurnStatus::Completed);
        let mut extra = state.new_session(state.projects[0].id, ProviderKind::Codex);
        extra.begin_turn("first");
        extra.push_message(MessageRole::Assistant, "one");
        extra.finish_active_turn(TurnStatus::Completed);
        extra.begin_turn("second");
        extra.push_message(MessageRole::Assistant, "two");
        extra.finish_active_turn(TurnStatus::Completed);
        let extra_id = extra.id;
        state.push_session(extra);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (6, 2));

        // Saving again rewrites the rows; nothing counts twice.
        state.mark_session_dirty(extra_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (6, 2));

        let index = index_of(&state, extra_id);
        state.sessions[index].truncate_after_turn(1);
        state.mark_session_dirty(extra_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (6, 2));

        state.sessions.retain(|session| session.id != extra_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (6, 2));
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_message_counts_once_it_finishes_streaming() {
        let (directory, store, mut state) = temporary_store();
        let session_id = state.sessions[0].id;
        state.sessions[0].begin_turn("ask");
        state.sessions[0].push_message(MessageRole::Assistant, "part");
        state.sessions[0].messages.last_mut().unwrap().streaming = true;
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (1, 1));

        let answer = state.sessions[0].messages.last_mut().unwrap();
        answer.content = "part of the answer".into();
        answer.streaming = false;
        state.sessions[0].finish_active_turn(TurnStatus::Completed);
        state.mark_session_dirty(session_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));

        state.mark_session_dirty(session_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));

        // Finishing the same row again neither counts nor fails the save.
        for streaming in [true, false] {
            state.sessions[0].messages.last_mut().unwrap().streaming = streaming;
            state.mark_session_dirty(session_id);
            store.save(&mut state).unwrap();
        }
        assert_eq!(stored_totals(&directory), (2, 1));
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_task_restored_with_the_same_ids_counts_once() {
        let (directory, store, mut state) = temporary_store();
        state.sessions[0].begin_turn("ask");
        state.sessions[0].push_message(MessageRole::Assistant, "answer");
        state.sessions[0].finish_active_turn(TurnStatus::Completed);
        store.save(&mut state).unwrap();
        let saved = state.sessions.remove(0);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));

        state.push_session(saved);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn a_fork_does_not_count_the_messages_it_copied() {
        let (directory, store, mut state) = temporary_store();
        let session = &mut state.sessions[0];
        session.begin_turn("ask");
        session.push_message(MessageRole::Assistant, "answer");
        session.finish_active_turn(TurnStatus::Completed);
        // The conversation happened an hour before the fork.
        session.created_at -= 3600;
        for message in &mut session.messages {
            message.created_at -= 3600;
        }
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));

        let fork = state.sessions[0]
            .fork_through_turn(
                1,
                ProviderResumeCursor::Codex {
                    thread_id: "forked".into(),
                },
                "Fork",
            )
            .unwrap();
        let fork_id = fork.id;
        state.push_session(fork);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (2, 1));

        let index = index_of(&state, fork_id);
        state.sessions[index].begin_turn("follow-up");
        state.mark_session_dirty(fork_id);
        store.save(&mut state).unwrap();
        assert_eq!(stored_totals(&directory), (3, 2));
        fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn messages_follow_the_session_model_and_keep_it_after_deletion() {
        let (directory, store, mut state) = temporary_store();
        let session_id = state.sessions[0].id;
        state.sessions[0].model = Some("anthropic::claude-opus-5-5".into());
        state.sessions[0].begin_turn("ask");
        store.save(&mut state).unwrap();
        let model_of_messages = || {
            let records = load(&directory.join("app.db")).unwrap();
            records
                .buckets
                .iter()
                .map(|bucket| records.models[bucket.model as usize].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(model_of_messages(), ["claude-opus-5-5"]);

        state.sessions[0].model = Some("gpt-6-sol".into());
        state.mark_session_dirty(session_id);
        store.save(&mut state).unwrap();
        assert_eq!(model_of_messages(), ["gpt-6-sol"]);

        state.sessions.clear();
        store.save(&mut state).unwrap();
        assert_eq!(model_of_messages(), ["gpt-6-sol"]);
        fs::remove_dir_all(directory).ok();
    }
}
