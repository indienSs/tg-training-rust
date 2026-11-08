use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Utc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::db::{Booking, Database};
use crate::handlers::fmt_range;

const TICK: StdDuration = StdDuration::from_secs(30);

pub fn spawn(bot: Bot, db: Arc<Database>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            if let Err(e) = tick(&bot, &db).await {
                log::error!("reminder tick failed: {e}");
            }
        }
    })
}

async fn tick(bot: &Bot, db: &Database) -> Result<(), Box<dyn StdError + Send + Sync>> {
    match db.complete_past_bookings() {
        Ok(n) if n > 0 => log::info!("completed {n} past booking(s)"),
        Ok(_) => {}
        Err(e) => log::error!("failed to complete past bookings: {e}"),
    }
    for b in db.bookings_to_remind()? {
        send_reminder(bot, db, &b).await;
    }
    Ok(())
}

async fn send_reminder(bot: &Bot, db: &Database, b: &Booking) {
    let mins = (b.starts_at - Utc::now()).num_minutes().max(0);
    let range = fmt_range(&b.starts_at, b.duration_min);
    let text = reminder_text(mins, &range);
    let kb = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Приду", format!("cf:{}:y", b.id)),
        InlineKeyboardButton::callback("❌ Не приду", format!("cf:{}:n", b.id)),
    ]]);
    if let Err(e) = bot
        .send_message(ChatId(b.user_chat_id), text)
        .reply_markup(kb)
        .await
    {
        log::warn!("failed to send reminder to {}: {e}", b.user_chat_id);
    }
    if let Err(e) = db.mark_reminded(b.id) {
        log::error!("failed to mark reminder for booking {}: {e}", b.id);
    }
}

fn reminder_text(mins: i64, range: &str) -> String {
    let until = if mins >= 60 {
        format!("~{} ч {} мин", mins / 60, mins % 60)
    } else {
        format!("~{mins} мин")
    };
    format!("⏰ Напоминание о тренировке\n🕐 {range}\nДо начала {until}.\n\nВы придете?")
}
