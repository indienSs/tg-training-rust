use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

pub struct Database {
    conn: Mutex<Connection>,
}

pub struct UserRow {
    pub chat_id: i64,
    pub username: Option<String>,
    pub full_name: String,
}

pub struct Slot {
    pub id: i64,
    pub starts_at: DateTime<Utc>,
    pub duration_min: i64,
}

pub struct Booking {
    pub id: i64,
    pub user_chat_id: i64,
    pub starts_at: DateTime<Utc>,
    pub duration_min: i64,
    pub status: String,
}

pub struct SlotView {
    pub slot: Slot,
    pub booking_id: Option<i64>,
    pub status: Option<String>,
    pub client_name: Option<String>,
    pub client_username: Option<String>,
}

pub enum DeleteSlotResult {
    Deleted,
    NotFound,
    HasActiveBooking(i64),
}

const DDL: &str = "
CREATE TABLE IF NOT EXISTS users (
    chat_id    INTEGER PRIMARY KEY,
    username   TEXT,
    full_name  TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS slots (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    starts_at    TEXT NOT NULL UNIQUE,
    duration_min INTEGER NOT NULL DEFAULT 60
);
CREATE TABLE IF NOT EXISTS bookings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    slot_id      INTEGER NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    user_chat_id INTEGER NOT NULL REFERENCES users(chat_id),
    status       TEXT NOT NULL DEFAULT 'booked',
    reminded_at  TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_bookings_slot ON bookings(slot_id);
CREATE INDEX IF NOT EXISTS idx_slots_starts ON slots(starts_at);
";

fn to_sql(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn from_sql(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|e| panic!("invalid datetime in db: {s}: {e}"))
        .and_utc()
}

impl Database {
    pub fn open(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(parent) = Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(DDL)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_user(
        &self,
        chat_id: i64,
        username: Option<&str>,
        full_name: &str,
    ) -> rusqlite::Result<()> {
        self.conn.lock().expect("db mutex poisoned").execute(
            "INSERT INTO users (chat_id, username, full_name, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(chat_id) DO UPDATE SET username = ?2, full_name = ?3",
            params![chat_id, username, full_name, to_sql(Utc::now())],
        )?;
        Ok(())
    }

    pub fn get_user(&self, chat_id: i64) -> rusqlite::Result<Option<UserRow>> {
        self.conn
            .lock()
            .expect("db mutex poisoned")
            .query_row(
                "SELECT chat_id, username, full_name FROM users WHERE chat_id = ?1",
                [chat_id],
                |r| {
                    Ok(UserRow {
                        chat_id: r.get(0)?,
                        username: r.get(1)?,
                        full_name: r.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn all_users(&self) -> rusqlite::Result<Vec<UserRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare("SELECT chat_id, username, full_name FROM users")?;
        let rows = stmt.query_map([], |r| {
            Ok(UserRow {
                chat_id: r.get(0)?,
                username: r.get(1)?,
                full_name: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn insert_slot(
        &self,
        starts_at: DateTime<Utc>,
        duration_min: i64,
    ) -> rusqlite::Result<bool> {
        let changed = self.conn.lock().expect("db mutex poisoned").execute(
            "INSERT OR IGNORE INTO slots (starts_at, duration_min) VALUES (?1, ?2)",
            params![to_sql(starts_at), duration_min],
        )?;
        Ok(changed == 1)
    }

    pub fn delete_slot(&self, slot_id: i64) -> rusqlite::Result<DeleteSlotResult> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let busy: Option<i64> = conn
            .query_row(
                "SELECT id FROM bookings
                 WHERE slot_id = ?1 AND status IN ('booked', 'confirmed')",
                [slot_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(booking_id) = busy {
            return Ok(DeleteSlotResult::HasActiveBooking(booking_id));
        }
        let changed = conn.execute("DELETE FROM slots WHERE id = ?1", [slot_id])?;
        Ok(if changed == 1 {
            DeleteSlotResult::Deleted
        } else {
            DeleteSlotResult::NotFound
        })
    }

    pub fn free_slots(&self) -> rusqlite::Result<Vec<Slot>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.starts_at, s.duration_min
             FROM slots s
             WHERE s.starts_at > ?1
               AND NOT EXISTS (
                   SELECT 1 FROM bookings b
                   WHERE b.slot_id = s.id AND b.status IN ('booked', 'confirmed')
               )
             ORDER BY s.starts_at
             LIMIT 200",
        )?;
        let rows = stmt.query_map(params![to_sql(Utc::now())], |r| {
            Ok(Slot {
                id: r.get(0)?,
                starts_at: from_sql(&r.get::<_, String>(1)?),
                duration_min: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn future_slots_with_bookings(&self) -> rusqlite::Result<Vec<SlotView>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.starts_at, s.duration_min, b.id, b.status, u.full_name, u.username
             FROM slots s
             LEFT JOIN bookings b
                    ON b.slot_id = s.id AND b.status IN ('booked', 'confirmed')
             LEFT JOIN users u ON u.chat_id = b.user_chat_id
             WHERE s.starts_at > ?1
             ORDER BY s.starts_at
             LIMIT 100",
        )?;
        let rows = stmt.query_map(params![to_sql(Utc::now())], |r| {
            Ok(SlotView {
                slot: Slot {
                    id: r.get(0)?,
                    starts_at: from_sql(&r.get::<_, String>(1)?),
                    duration_min: r.get(2)?,
                },
                booking_id: r.get(3)?,
                status: r.get(4)?,
                client_name: r.get(5)?,
                client_username: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn user_bookings(&self, user_chat_id: i64) -> rusqlite::Result<Vec<Booking>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT b.id, b.slot_id, b.user_chat_id, s.starts_at, s.duration_min, b.status
             FROM bookings b JOIN slots s ON s.id = b.slot_id
             WHERE b.user_chat_id = ?1
               AND b.status IN ('booked', 'confirmed')
               AND s.starts_at > ?2
             ORDER BY s.starts_at",
        )?;
        let rows = stmt.query_map(params![user_chat_id, to_sql(Utc::now())], |r| {
            Ok(Booking {
                id: r.get(0)?,
                user_chat_id: r.get(2)?,
                starts_at: from_sql(&r.get::<_, String>(3)?),
                duration_min: r.get(4)?,
                status: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn trainer_bookings(&self) -> rusqlite::Result<Vec<(Booking, UserRow)>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT b.id, b.slot_id, b.user_chat_id, s.starts_at, s.duration_min, b.status,
                    u.chat_id, u.username, u.full_name
             FROM bookings b
             JOIN slots s ON s.id = b.slot_id
             JOIN users u ON u.chat_id = b.user_chat_id
             WHERE b.status IN ('booked', 'confirmed') AND s.starts_at > ?1
             ORDER BY s.starts_at",
        )?;
        let rows = stmt.query_map(params![to_sql(Utc::now())], |r| {
            Ok((
                Booking {
                    id: r.get(0)?,
                    user_chat_id: r.get(2)?,
                    starts_at: from_sql(&r.get::<_, String>(3)?),
                    duration_min: r.get(4)?,
                    status: r.get(5)?,
                },
                UserRow {
                    chat_id: r.get(6)?,
                    username: r.get(7)?,
                    full_name: r.get(8)?,
                },
            ))
        })?;
        rows.collect()
    }

    pub fn create_booking(
        &self,
        slot_id: i64,
        user_chat_id: i64,
    ) -> rusqlite::Result<Option<Booking>> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT s.starts_at, s.duration_min
                 FROM slots s
                 WHERE s.id = ?1 AND s.starts_at > ?2
                   AND NOT EXISTS (
                       SELECT 1 FROM bookings b
                       WHERE b.slot_id = s.id AND b.status IN ('booked', 'confirmed')
                   )",
                params![slot_id, to_sql(Utc::now())],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((starts_at, duration_min)) = row else {
            return Ok(None);
        };
        tx.execute(
            "INSERT INTO bookings (slot_id, user_chat_id, status, created_at)
             VALUES (?1, ?2, 'booked', ?3)",
            params![slot_id, user_chat_id, to_sql(Utc::now())],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some(Booking {
            id,
            user_chat_id,
            starts_at: from_sql(&starts_at),
            duration_min,
            status: "booked".to_string(),
        }))
    }

    pub fn update_booking_status(
        &self,
        booking_id: i64,
        user_chat_id: Option<i64>,
        new_status: &str,
    ) -> rusqlite::Result<Option<Booking>> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let row: Option<(i64, String, i64)> = match user_chat_id {
            Some(uid) => tx
                .query_row(
                    "SELECT b.user_chat_id, s.starts_at, s.duration_min
                     FROM bookings b JOIN slots s ON s.id = b.slot_id
                     WHERE b.id = ?1 AND b.user_chat_id = ?2
                       AND b.status IN ('booked', 'confirmed')",
                    params![booking_id, uid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?,
            None => tx
                .query_row(
                    "SELECT b.user_chat_id, s.starts_at, s.duration_min
                     FROM bookings b JOIN slots s ON s.id = b.slot_id
                     WHERE b.id = ?1 AND b.status IN ('booked', 'confirmed')",
                    params![booking_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?,
        };
        let Some((owner_chat_id, starts_at, duration_min)) = row else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE bookings SET status = ?1 WHERE id = ?2",
            params![new_status, booking_id],
        )?;
        tx.commit()?;
        Ok(Some(Booking {
            id: booking_id,
            user_chat_id: owner_chat_id,
            starts_at: from_sql(&starts_at),
            duration_min,
            status: new_status.to_string(),
        }))
    }

    pub fn bookings_to_remind(&self) -> rusqlite::Result<Vec<Booking>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT b.id, b.slot_id, b.user_chat_id, s.starts_at, s.duration_min
             FROM bookings b JOIN slots s ON s.id = b.slot_id
             WHERE b.status = 'booked'
               AND b.reminded_at IS NULL
               AND s.starts_at > ?1
               AND s.starts_at <= ?2
             ORDER BY s.starts_at",
        )?;
        let now = Utc::now();
        let soon = now + chrono::Duration::hours(2);
        let rows = stmt.query_map(params![to_sql(now), to_sql(soon)], |r| {
            Ok(Booking {
                id: r.get(0)?,
                user_chat_id: r.get(2)?,
                starts_at: from_sql(&r.get::<_, String>(3)?),
                duration_min: r.get(4)?,
                status: "booked".to_string(),
            })
        })?;
        rows.collect()
    }

    pub fn mark_reminded(&self, booking_id: i64) -> rusqlite::Result<()> {
        self.conn.lock().expect("db mutex poisoned").execute(
            "UPDATE bookings SET reminded_at = ?2 WHERE id = ?1",
            params![booking_id, to_sql(Utc::now())],
        )?;
        Ok(())
    }
}
