use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use teloxide::macros::BotCommands;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, MessageId,
};

use crate::config::Config;
use crate::db::{Database, DeleteSlotResult, Slot};

pub type HResult = Result<(), Box<dyn StdError + Send + Sync>>;

const MONTHS: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];
const WEEKDAYS: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];

const CLIENT_WELCOME: &str =
    "Привет! 👋\nЯ бот для записи на тренировки.\n\nВыберите действие в меню ниже:";
const CLIENT_HINT: &str = "Воспользуйтесь меню ниже 👇 Или /help";
const CLIENT_HELP: &str = "Этот бот помогает записываться на тренировки.\n\n\
    📅 Записаться — выбрать свободное время\n\
    📋 Мои записи — посмотреть и отменить записи\n\n\
    За 2 часа до тренировки я пришлю напоминание и уточню, придете ли вы.";

const TRAINER_HELP: &str = "Вы вошли как тренер. Команды:\n\n\
    /addslot 2026-11-08 10:00 — добавить слот\n\
    /addday 2026-11-08 09:00 18:00 60 — слоты на день (с—по, шаг в минутах)\n\
    /slots — будущие слоты (id и занятость)\n\
    /delslot <id> — удалить слот\n\
    /bookings — активные записи клиентов\n\
    /users — список клиентов\n\
    /broadcast <текст> — рассылка всем клиентам\n\n\
    О каждой записи, подтверждении и отмене я буду присылать уведомления.";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Команды бота")]
pub enum Command {
    #[command(description = "начать работу с ботом")]
    Start,
    #[command(description = "помощь")]
    Help,
    #[command(description = "записаться на тренировку")]
    Book,
    #[command(description = "мои записи")]
    My,
    #[command(description = "тренер: добавить слот, напр. /addslot 2026-11-08 10:00")]
    AddSlot,
    #[command(description = "тренер: слоты на день, напр. /addday 2026-11-08 09:00 18:00 60")]
    AddDay,
    #[command(description = "тренер: удалить слот по id")]
    DelSlot,
    #[command(description = "тренер: список слотов")]
    Slots,
    #[command(description = "тренер: активные записи")]
    Bookings,
    #[command(description = "тренер: список клиентов")]
    Users,
    #[command(description = "тренер: рассылка, напр. /broadcast текст")]
    Broadcast,
}

fn to_local(dt: &DateTime<Utc>) -> DateTime<Local> {
    dt.with_timezone(&Local)
}

pub fn local_date_key(dt: &DateTime<Utc>) -> String {
    to_local(dt).format("%Y-%m-%d").to_string()
}

fn fmt_hm(dt: &DateTime<Utc>) -> String {
    to_local(dt).format("%H:%M").to_string()
}

fn day_month(l: &DateTime<Local>) -> String {
    let wd = WEEKDAYS[l.weekday().num_days_from_sunday() as usize];
    format!("{} {} {}", wd, l.day(), MONTHS[l.month0() as usize])
}

pub fn fmt_range(dt: &DateTime<Utc>, duration_min: i64) -> String {
    let start = to_local(dt);
    let end = start + chrono::Duration::minutes(duration_min);
    format!(
        "{}, {:02}:{:02}–{:02}:{:02}",
        day_month(&start),
        start.hour(),
        start.minute(),
        end.hour(),
        end.minute()
    )
}

fn fmt_date_key(key: &str) -> String {
    NaiveDate::parse_from_str(key, "%Y-%m-%d")
        .map(|d| {
            let wd = WEEKDAYS[d.weekday().num_days_from_sunday() as usize];
            format!("{} {} {}", wd, d.day(), MONTHS[d.month0() as usize])
        })
        .unwrap_or_else(|_| key.to_string())
}

fn client_menu() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![vec![
        KeyboardButton::new("📅 Записаться"),
        KeyboardButton::new("📋 Мои записи"),
    ]])
    .resize_keyboard()
}

fn trainer_menu() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![vec![
        KeyboardButton::new("📋 Записи"),
        KeyboardButton::new("🕐 Слоты"),
        KeyboardButton::new("👥 Клиенты"),
    ]])
    .resize_keyboard()
}

fn empty_kb() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new())
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    ["%Y-%m-%d", "%d.%m.%Y"]
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    ["%H:%M", "%H:%M:%S"]
        .iter()
        .find_map(|f| NaiveTime::parse_from_str(s, f).ok())
}

fn local_to_utc(d: NaiveDate, t: NaiveTime) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&d.and_hms_opt(t.hour(), t.minute(), t.second())?)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
}

fn user_desc(db: &Database, chat_id: i64) -> String {
    match db.get_user(chat_id) {
        Ok(Some(u)) => match u.username {
            Some(un) => format!("{} (@{})", u.full_name, un),
            None => u.full_name,
        },
        _ => format!("id {chat_id}"),
    }
}

async fn notify_trainer(bot: &Bot, cfg: &Config, text: &str) {
    if let Err(e) = bot.send_message(ChatId(cfg.trainer_chat_id), text).await {
        log::warn!("failed to notify trainer: {e}");
    }
}

async fn edit_msg(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    text: &str,
    kb: InlineKeyboardMarkup,
) -> HResult {
    if let Err(e) = bot
        .edit_message_text(chat_id, msg_id, text)
        .reply_markup(kb)
        .await
    {
        if e.to_string().contains("message is not modified") {
            return Ok(());
        }
        return Err(e.into());
    }
    Ok(())
}

fn dates_view(db: &Database) -> rusqlite::Result<Option<(String, InlineKeyboardMarkup)>> {
    let slots = db.free_slots()?;
    if slots.is_empty() {
        return Ok(None);
    }
    let mut by_date: BTreeMap<String, Vec<&Slot>> = BTreeMap::new();
    for s in &slots {
        by_date
            .entry(local_date_key(&s.starts_at))
            .or_default()
            .push(s);
    }
    let rows: Vec<Vec<InlineKeyboardButton>> = by_date
        .into_iter()
        .map(|(date, group)| {
            vec![InlineKeyboardButton::callback(
                format!("{} · свободно: {}", fmt_date_key(&date), group.len()),
                format!("pick:{date}"),
            )]
        })
        .collect();
    Ok(Some((
        "📅 Выберите день тренировки:".to_string(),
        InlineKeyboardMarkup::new(rows),
    )))
}

fn times_view(
    db: &Database,
    date: &str,
) -> rusqlite::Result<Option<(String, InlineKeyboardMarkup)>> {
    let slots: Vec<Slot> = db
        .free_slots()?
        .into_iter()
        .filter(|s| local_date_key(&s.starts_at) == date)
        .collect();
    if slots.is_empty() {
        return Ok(None);
    }
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in slots.chunks(3) {
        rows.push(
            chunk
                .iter()
                .map(|s| {
                    InlineKeyboardButton::callback(fmt_hm(&s.starts_at), format!("bk:{}", s.id))
                })
                .collect(),
        );
    }
    rows.push(vec![InlineKeyboardButton::callback("◀️ Назад", "back")]);
    Ok(Some((
        format!("🕐 Свободное время на {}:", fmt_date_key(date)),
        InlineKeyboardMarkup::new(rows),
    )))
}

async fn show_dates(bot: &Bot, chat_id: ChatId, db: &Database) -> HResult {
    match dates_view(db)? {
        Some((text, kb)) => {
            bot.send_message(chat_id, text).reply_markup(kb).await?;
        }
        None => {
            bot.send_message(chat_id, "😔 Свободного времени пока нет. Загляните позже.")
                .await?;
        }
    }
    Ok(())
}

async fn show_my(bot: &Bot, chat_id: ChatId, db: &Database) -> HResult {
    let bookings = db.user_bookings(chat_id.0)?;
    if bookings.is_empty() {
        bot.send_message(
            chat_id,
            "У вас нет активных записей.\nНажмите «📅 Записаться», чтобы выбрать время.",
        )
        .await?;
        return Ok(());
    }
    let mut text = String::from("📋 Ваши записи:\n\n");
    let mut rows = Vec::new();
    for (i, b) in bookings.iter().enumerate() {
        text.push_str(&format!(
            "{}. {}\n",
            i + 1,
            fmt_range(&b.starts_at, b.duration_min)
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            format!("❌ Отменить №{}", i + 1),
            format!("cx:{}", b.id),
        )]);
    }
    bot.send_message(chat_id, text)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

fn status_label(status: &str) -> &'static str {
    match status {
        "confirmed" => "✅ подтверждена",
        _ => "⏳ ожидает ответа клиента",
    }
}

async fn trainer_slots_view(bot: &Bot, chat_id: ChatId, db: &Database) -> HResult {
    let views = db.future_slots_with_bookings()?;
    if views.is_empty() {
        bot.send_message(
            chat_id,
            "Будущих слотов нет.\nДобавьте: /addslot 2026-11-08 10:00\nИли на весь день: /addday 2026-11-08 09:00 18:00 60",
        )
        .await?;
        return Ok(());
    }
    let mut text = String::from("🕐 Будущие слоты:\n\n");
    for v in views {
        let line = match (&v.booking_id, &v.client_name) {
            (Some(_), Some(name)) => format!(
                "#{} · {} · 👤 {}{}\n",
                v.slot.id,
                fmt_range(&v.slot.starts_at, v.slot.duration_min),
                name,
                v.client_username
                    .as_deref()
                    .map(|u| format!(" (@{u})"))
                    .unwrap_or_default()
            ),
            _ => format!(
                "#{} · {} · свободен\n",
                v.slot.id,
                fmt_range(&v.slot.starts_at, v.slot.duration_min)
            ),
        };
        text.push_str(&line);
        if let Some(st) = &v.status {
            text.push_str(&format!("    {}\n", status_label(st)));
        }
    }
    bot.send_message(chat_id, text).await?;
    Ok(())
}

async fn trainer_bookings_view(bot: &Bot, chat_id: ChatId, db: &Database) -> HResult {
    let bookings = db.trainer_bookings()?;
    if bookings.is_empty() {
        bot.send_message(chat_id, "Активных записей нет.").await?;
        return Ok(());
    }
    let mut text = String::from("📋 Активные записи:\n\n");
    for (i, (b, u)) in bookings.iter().enumerate() {
        text.push_str(&format!(
            "{}. {} · {}{}\n    {}\n",
            i + 1,
            fmt_range(&b.starts_at, b.duration_min),
            u.full_name,
            u.username
                .as_deref()
                .map(|un| format!(" (@{un})"))
                .unwrap_or_default(),
            status_label(&b.status)
        ));
    }
    bot.send_message(chat_id, text).await?;
    Ok(())
}

async fn trainer_users_view(bot: &Bot, chat_id: ChatId, db: &Database, cfg: &Config) -> HResult {
    let users: Vec<_> = db
        .all_users()?
        .into_iter()
        .filter(|u| u.chat_id != cfg.trainer_chat_id)
        .collect();
    if users.is_empty() {
        bot.send_message(chat_id, "Клиентов пока нет.").await?;
        return Ok(());
    }
    let mut text = format!("👥 Клиентов: {}\n\n", users.len());
    for u in &users {
        text.push_str(&format!(
            "• {}{}\n",
            u.full_name,
            u.username
                .as_deref()
                .map(|un| format!(" (@{un})"))
                .unwrap_or_default()
        ));
    }
    bot.send_message(chat_id, text).await?;
    Ok(())
}

async fn broadcast(
    bot: &Bot,
    chat_id: ChatId,
    db: &Database,
    cfg: &Config,
    msg: &Message,
) -> HResult {
    let Some((_, text)) = msg.text().and_then(|t| t.split_once(' ')) else {
        bot.send_message(chat_id, "Использование: /broadcast <текст сообщения>")
            .await?;
        return Ok(());
    };
    let text = text.trim();
    if text.is_empty() {
        bot.send_message(chat_id, "Использование: /broadcast <текст сообщения>")
            .await?;
        return Ok(());
    }
    let users: Vec<_> = db
        .all_users()?
        .into_iter()
        .filter(|u| u.chat_id != cfg.trainer_chat_id && u.chat_id != chat_id.0)
        .collect();
    let mut sent = 0usize;
    let mut failed = 0usize;
    for u in &users {
        match bot.send_message(ChatId(u.chat_id), text).await {
            Ok(_) => sent += 1,
            Err(e) => {
                failed += 1;
                log::warn!("broadcast to {} failed: {e}", u.chat_id);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bot.send_message(
        chat_id,
        format!("📣 Рассылка завершена: доставлено {sent}, не доставлено {failed}."),
    )
    .await?;
    Ok(())
}

pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    db: Arc<Database>,
    cfg: Arc<Config>,
) -> HResult {
    let chat_id = msg.chat.id;
    let is_trainer = chat_id.0 == cfg.trainer_chat_id;
    if let Some(u) = msg.from.as_ref() {
        db.upsert_user(u.id.0 as i64, u.username.as_deref(), &u.full_name())?;
    }
    let guard = async |allowed: bool| -> Option<HResult> {
        if allowed {
            return None;
        }
        if let Err(e) = bot
            .send_message(chat_id, "Эта команда доступна только тренеру.")
            .await
        {
            log::warn!("failed to send trainer-only notice: {e}");
        }
        Some(Ok(()))
    };

    match cmd {
        Command::Start => {
            if is_trainer {
                bot.send_message(chat_id, "Добро пожаловать, тренер! 💪")
                    .reply_markup(trainer_menu())
                    .await?;
                bot.send_message(chat_id, TRAINER_HELP).await?;
            } else {
                bot.send_message(chat_id, CLIENT_WELCOME)
                    .reply_markup(client_menu())
                    .await?;
            }
        }
        Command::Help => {
            let text = if is_trainer {
                TRAINER_HELP
            } else {
                CLIENT_HELP
            };
            bot.send_message(chat_id, text).await?;
        }
        Command::Book => show_dates(&bot, chat_id, &db).await?,
        Command::My => show_my(&bot, chat_id, &db).await?,
        Command::AddSlot => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            if args.len() != 3 {
                bot.send_message(chat_id, "Использование: /addslot 2026-11-08 10:00")
                    .await?;
                return Ok(());
            }
            let (Some(d), Some(t)) = (parse_date(args[1]), parse_time(args[2])) else {
                bot.send_message(
                    chat_id,
                    "Не понял дату или время. Пример: /addslot 2026-11-08 10:00",
                )
                .await?;
                return Ok(());
            };
            let Some(dt) = local_to_utc(d, t) else {
                bot.send_message(chat_id, "Не удалось распознать время.")
                    .await?;
                return Ok(());
            };
            if dt <= Utc::now() {
                bot.send_message(chat_id, "Слот в прошлом — не добавил.")
                    .await?;
                return Ok(());
            }
            if db.insert_slot(dt, 60)? {
                bot.send_message(chat_id, format!("✅ Слот добавлен: {}", fmt_range(&dt, 60)))
                    .await?;
            } else {
                bot.send_message(chat_id, "Такой слот уже есть.").await?;
            }
        }
        Command::AddDay => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            if args.len() != 5 {
                bot.send_message(chat_id, "Использование: /addday 2026-11-08 09:00 18:00 60")
                    .await?;
                return Ok(());
            }
            let (Some(d), Some(from), Some(to)) = (
                parse_date(args[1]),
                parse_time(args[2]),
                parse_time(args[3]),
            ) else {
                bot.send_message(
                    chat_id,
                    "Не понял дату или время. Пример: /addday 2026-11-08 09:00 18:00 60",
                )
                .await?;
                return Ok(());
            };
            let Ok(step) = args[4].parse::<i64>() else {
                bot.send_message(chat_id, "Шаг должен быть числом минут, например 60.")
                    .await?;
                return Ok(());
            };
            if !(15..=240).contains(&step) {
                bot.send_message(chat_id, "Шаг должен быть от 15 до 240 минут.")
                    .await?;
                return Ok(());
            }
            let from_m = (from.num_seconds_from_midnight() / 60) as i64;
            let to_m = (to.num_seconds_from_midnight() / 60) as i64;
            if to_m <= from_m {
                bot.send_message(chat_id, "Время «до» должно быть позже времени «с».")
                    .await?;
                return Ok(());
            }
            let now = Utc::now();
            let mut created = 0usize;
            let mut total = 0usize;
            let mut m = from_m;
            while m + step <= to_m {
                let Some(t) = NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0) else {
                    m += step;
                    continue;
                };
                if let Some(dt) = local_to_utc(d, t)
                    && dt > now
                {
                    total += 1;
                    if db.insert_slot(dt, 60)? {
                        created += 1;
                    }
                }
                m += step;
            }
            if total == 0 {
                bot.send_message(chat_id, "Все слоты этого дня уже в прошлом.")
                    .await?;
            } else {
                bot.send_message(
                    chat_id,
                    format!("✅ Добавлено новых слотов: {created} (промежутков: {total})."),
                )
                .await?;
            }
        }
        Command::DelSlot => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            let Some(slot_id) = args.get(1).and_then(|s| s.parse::<i64>().ok()) else {
                bot.send_message(chat_id, "Использование: /delslot <id> (id см. в /slots)")
                    .await?;
                return Ok(());
            };
            match db.delete_slot(slot_id)? {
                DeleteSlotResult::Deleted => {
                    bot.send_message(chat_id, "✅ Слот удалён.").await?;
                }
                DeleteSlotResult::NotFound => {
                    bot.send_message(chat_id, "Слот с таким id не найден.")
                        .await?;
                }
                DeleteSlotResult::HasActiveBooking(booking_id) => {
                    bot.send_message(
                        chat_id,
                        format!("Слот занят (запись #{booking_id}). Сначала дождитесь отмены или удалите её через клиента."),
                    )
                    .await?;
                }
            }
        }
        Command::Slots => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            trainer_slots_view(&bot, chat_id, &db).await?;
        }
        Command::Bookings => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            trainer_bookings_view(&bot, chat_id, &db).await?;
        }
        Command::Users => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            trainer_users_view(&bot, chat_id, &db, &cfg).await?;
        }
        Command::Broadcast => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            broadcast(&bot, chat_id, &db, &cfg, &msg).await?;
        }
    }
    Ok(())
}

pub async fn handle_text(bot: Bot, msg: Message, db: Arc<Database>, cfg: Arc<Config>) -> HResult {
    let chat_id = msg.chat.id;
    let is_trainer = chat_id.0 == cfg.trainer_chat_id;
    if let Some(u) = msg.from.as_ref() {
        db.upsert_user(u.id.0 as i64, u.username.as_deref(), &u.full_name())?;
    }
    match msg.text() {
        Some("📅 Записаться") => show_dates(&bot, chat_id, &db).await?,
        Some("📋 Мои записи") => show_my(&bot, chat_id, &db).await?,
        Some("📋 Записи") if is_trainer => {
            trainer_bookings_view(&bot, chat_id, &db).await?
        }
        Some("🕐 Слоты") if is_trainer => trainer_slots_view(&bot, chat_id, &db).await?,
        Some("👥 Клиенты") if is_trainer => {
            trainer_users_view(&bot, chat_id, &db, &cfg).await?
        }
        _ => {
            let text = if is_trainer {
                TRAINER_HELP
            } else {
                CLIENT_HINT
            };
            bot.send_message(chat_id, text).await?;
        }
    }
    Ok(())
}

pub async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    db: Arc<Database>,
    cfg: Arc<Config>,
) -> HResult {
    let Some(data) = q.data.clone() else {
        bot.answer_callback_query(q.id.clone()).await?;
        return Ok(());
    };
    let user_id = q.from.id.0 as i64;
    let Some(target) = q.message.as_ref().map(|m| (m.chat().id, m.id())) else {
        bot.answer_callback_query(q.id.clone()).await?;
        return Ok(());
    };
    let (chat_id, msg_id) = target;

    let parts: Vec<&str> = data.split(':').collect();
    match parts.as_slice() {
        ["pick", date] => {
            if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                bot.answer_callback_query(q.id.clone())
                    .text("Неверная дата.")
                    .await?;
                return Ok(());
            }
            match times_view(&db, date)? {
                Some((text, kb)) => edit_msg(&bot, chat_id, msg_id, &text, kb).await?,
                None => {
                    if let Some((text, kb)) = dates_view(&db)? {
                        edit_msg(&bot, chat_id, msg_id, &text, kb).await?;
                    }
                    bot.answer_callback_query(q.id.clone())
                        .text("На этот день свободных слотов нет.")
                        .await?;
                }
            }
        }
        ["back"] => match dates_view(&db)? {
            Some((text, kb)) => edit_msg(&bot, chat_id, msg_id, &text, kb).await?,
            None => {
                edit_msg(
                    &bot,
                    chat_id,
                    msg_id,
                    "Свободного времени больше нет.",
                    empty_kb(),
                )
                .await?
            }
        },
        ["bk", id] => {
            let slot_id = id.parse::<i64>().unwrap_or(-1);
            match db.create_booking(slot_id, user_id)? {
                Some(b) => {
                    let range = fmt_range(&b.starts_at, b.duration_min);
                    edit_msg(
                        &bot,
                        chat_id,
                        msg_id,
                        &format!(
                            "✅ Вы запислены: {range}.\nЗа 2 часа до тренировки пришлю напоминание."
                        ),
                        empty_kb(),
                    )
                    .await?;
                    notify_trainer(
                        &bot,
                        &cfg,
                        &format!(
                            "📥 Новая запись\n👤 {}\n🕐 {range}",
                            user_desc(&db, user_id)
                        ),
                    )
                    .await;
                    bot.answer_callback_query(q.id.clone())
                        .text("Записал ✅")
                        .await?;
                }
                None => {
                    if let Some((text, kb)) = dates_view(&db)? {
                        edit_msg(&bot, chat_id, msg_id, &text, kb).await?;
                    }
                    bot.answer_callback_query(q.id.clone())
                        .text("Это время уже заняли 😔 Выберите другое.")
                        .show_alert(true)
                        .await?;
                }
            }
        }
        ["cx", id] => {
            let booking_id = id.parse::<i64>().unwrap_or(-1);
            match db.update_booking_status(booking_id, Some(user_id), "cancelled")? {
                Some(b) => {
                    let range = fmt_range(&b.starts_at, b.duration_min);
                    edit_msg(
                        &bot,
                        chat_id,
                        msg_id,
                        &format!("❌ Запись на {range} отменена."),
                        empty_kb(),
                    )
                    .await?;
                    notify_trainer(
                        &bot,
                        &cfg,
                        &format!(
                            "🚫 Отмена записи\n👤 {}\n🕐 {range}",
                            user_desc(&db, user_id)
                        ),
                    )
                    .await;
                    bot.answer_callback_query(q.id.clone())
                        .text("Отменил ❌")
                        .await?;
                }
                None => {
                    bot.answer_callback_query(q.id.clone())
                        .text("Запись не найдена или уже отменена.")
                        .show_alert(true)
                        .await?;
                }
            }
        }
        ["cf", id, ans] => {
            let booking_id = id.parse::<i64>().unwrap_or(-1);
            if *ans == "y" {
                match db.update_booking_status(booking_id, Some(user_id), "confirmed")? {
                    Some(b) => {
                        let range = fmt_range(&b.starts_at, b.duration_min);
                        edit_msg(
                            &bot,
                            chat_id,
                            msg_id,
                            &format!("✅ Отлично! Жду вас: {range}."),
                            empty_kb(),
                        )
                        .await?;
                        notify_trainer(
                            &bot,
                            &cfg,
                            &format!(
                                "👍 Подтверждение\n👤 {}\n🕐 {range}",
                                user_desc(&db, user_id)
                            ),
                        )
                        .await;
                        bot.answer_callback_query(q.id.clone())
                            .text("Спасибо! 💪")
                            .await?;
                    }
                    None => {
                        bot.answer_callback_query(q.id.clone())
                            .text("Запись не найдена или уже обработана.")
                            .show_alert(true)
                            .await?;
                    }
                }
            } else {
                match db.update_booking_status(booking_id, Some(user_id), "cancelled")? {
                    Some(b) => {
                        let range = fmt_range(&b.starts_at, b.duration_min);
                        edit_msg(
                            &bot,
                            chat_id,
                            msg_id,
                            &format!("Понял, запись на {range} отменена. Если передумаете — запишитесь снова."),
                            empty_kb(),
                        )
                        .await?;
                        notify_trainer(
                            &bot,
                            &cfg,
                            &format!(
                                "❌ Клиент не придёт\n👤 {}\n🕐 {range}\nСлот снова свободен.",
                                user_desc(&db, user_id)
                            ),
                        )
                        .await;
                        bot.answer_callback_query(q.id.clone())
                            .text("Отменил запись ❌")
                            .await?;
                    }
                    None => {
                        bot.answer_callback_query(q.id.clone())
                            .text("Запись не найдена или уже обработана.")
                            .show_alert(true)
                            .await?;
                    }
                }
            }
        }
        _ => {
            bot.answer_callback_query(q.id.clone()).await?;
        }
    }
    Ok(())
}
