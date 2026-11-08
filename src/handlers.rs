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
    Рабочий график:\n\
    /setworkday 08:00 20:00 60 — стандартный ежедневный график (с, до, шаг в минутах)\n\
    /addworkday — слоты на сегодня по графику\n\
    /addworkdays 7 — слоты на 7 дней, начиная с сегодня\n\
    /delworkday 2026-11-08 — удалить слоты за дату\n\
    /delworkdays 7 — удалить слоты за 7 дней с сегодня\n\n\
    Точечно:\n\
    /addslot 2026-11-08 10:00 — добавить один слот\n\
    /addday 2026-11-08 09:00 18:00 60 — слоты на день (с—по, шаг)\n\
    /slots — будущие слоты (id и занятость)\n\
    /delslot <id> — удалить слот\n\n\
    Клиенты:\n\
    /bookings — активные записи\n\
    /users — список клиентов (id для /ban)\n\
    /ban <id или @username> — заблокировать клиента\n\
    /unban <id или @username> — разблокировать клиента\n\
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
    #[command(description = "тренер: стандартный график, напр. /setworkday 08:00 20:00 60")]
    SetWorkDay,
    #[command(description = "тренер: слоты на сегодня по стандартному графику")]
    AddWorkDay,
    #[command(description = "тренер: слоты на N дней с сегодня, напр. /addworkdays 7")]
    AddWorkDays,
    #[command(description = "тренер: удалить слоты за дату, напр. /delworkday 2026-11-08")]
    DelWorkDay,
    #[command(description = "тренер: удалить слоты за N дней с сегодня")]
    DelWorkDays,
    #[command(description = "тренер: удалить слот по id")]
    DelSlot,
    #[command(description = "тренер: список слотов")]
    Slots,
    #[command(description = "тренер: активные записи")]
    Bookings,
    #[command(description = "тренер: список клиентов")]
    Users,
    #[command(description = "тренер: заблокировать: /ban id_или_@username")]
    Ban,
    #[command(description = "тренер: разблокировать: /unban id_или_@username")]
    Unban,
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

fn today_local() -> NaiveDate {
    Local::now().date_naive()
}

fn minutes_to_time(m: i64) -> Option<NaiveTime> {
    NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0)
}

fn time_to_minutes(t: NaiveTime) -> i64 {
    (t.num_seconds_from_midnight() / 60) as i64
}

fn fmt_minutes(m: i64) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn gen_work_slots(dates: &[NaiveDate], from_m: i64, to_m: i64, step: i64) -> Vec<DateTime<Utc>> {
    let mut out = Vec::new();
    for &d in dates {
        let mut m = from_m;
        while m + step <= to_m {
            if let Some(t) = minutes_to_time(m)
                && let Some(dt) = local_to_utc(d, t)
            {
                out.push(dt);
            }
            m += step;
        }
    }
    out
}

fn day_bounds_utc(d: NaiveDate) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let lo_naive = d.and_hms_opt(0, 0, 0)?;
    let hi_naive = d.succ_opt()?.and_hms_opt(0, 0, 0)?;
    let lo = Local.from_local_datetime(&lo_naive).single()?;
    let hi = Local.from_local_datetime(&hi_naive).single()?;
    Some((lo.with_timezone(&Utc), hi.with_timezone(&Utc)))
}

async fn add_slots_and_report(
    bot: &Bot,
    chat_id: ChatId,
    db: &Database,
    dts: Vec<DateTime<Utc>>,
    label: &str,
) -> HResult {
    let now = Utc::now();
    let past = dts.iter().filter(|dt| **dt <= now).count();
    let future: Vec<(DateTime<Utc>, i64)> = dts
        .into_iter()
        .filter(|dt| *dt > now)
        .map(|dt| (dt, 60))
        .collect();
    let total = future.len();
    let created = db.insert_slots_batch(&future)?;
    let mut text =
        format!("✅ Добавлено новых слотов: {created} (промежутков: {total}) — {label}.");
    if past > 0 {
        text.push_str(&format!("\n⏳ Пропущено прошедших: {past}."));
    }
    bot.send_message(chat_id, text).await?;
    Ok(())
}

fn user_desc(db: &Database, chat_id: i64) -> String {
    match db.get_user(chat_id) {
        Ok(Some(u)) => {
            let un = u
                .username
                .as_deref()
                .map(|x| format!(" (@{x})"))
                .unwrap_or_default();
            format!("{}{un} · id {}", u.full_name, u.chat_id)
        }
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
            "Будущих слотов нет.\nДобавьте по стандартному графику: /addworkday (сегодня) или /addworkdays 7\nИли точечно: /addslot 2026-11-08 10:00, /addday 2026-11-08 09:00 18:00 60\nГрафик задаётся командой /setworkday 08:00 20:00 60",
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
        let un = u
            .username
            .as_deref()
            .map(|x| format!(" (@{x})"))
            .unwrap_or_default();
        let mark = if u.banned { " [⛔]" } else { "" };
        text.push_str(&format!("• {}{un} · id {}{mark}\n", u.full_name, u.chat_id));
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
        .filter(|u| !u.banned && u.chat_id != cfg.trainer_chat_id && u.chat_id != chat_id.0)
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
    if !is_trainer && db.is_banned(chat_id.0)? {
        bot.send_message(chat_id, "⛔ Доступ к боту закрыт.")
            .await?;
        return Ok(());
    }
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
            let from_m = time_to_minutes(from);
            let to_m = time_to_minutes(to);
            if to_m <= from_m {
                bot.send_message(chat_id, "Время «до» должно быть позже времени «с».")
                    .await?;
                return Ok(());
            }
            let dts = gen_work_slots(&[d], from_m, to_m, step);
            add_slots_and_report(
                &bot,
                chat_id,
                &db,
                dts,
                &fmt_date_key(&d.format("%Y-%m-%d").to_string()),
            )
            .await?;
        }
        Command::SetWorkDay => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            if args.len() != 4 {
                bot.send_message(chat_id, "Использование: /setworkday 08:00 20:00 60")
                    .await?;
                return Ok(());
            }
            let (Some(from), Some(to)) = (parse_time(args[1]), parse_time(args[2])) else {
                bot.send_message(
                    chat_id,
                    "Не понял время. Пример: /setworkday 08:00 20:00 60",
                )
                .await?;
                return Ok(());
            };
            let Ok(step) = args[3].parse::<i64>() else {
                bot.send_message(chat_id, "Шаг должен быть числом минут, например 60.")
                    .await?;
                return Ok(());
            };
            if !(15..=240).contains(&step) {
                bot.send_message(chat_id, "Шаг должен быть от 15 до 240 минут.")
                    .await?;
                return Ok(());
            }
            let from_m = time_to_minutes(from);
            let to_m = time_to_minutes(to);
            if to_m <= from_m {
                bot.send_message(chat_id, "Время «до» должно быть позже времени «с».")
                    .await?;
                return Ok(());
            }
            db.set_workday(from_m, to_m, step)?;
            bot.send_message(
                chat_id,
                format!(
                    "✅ Стандартный график сохранён: {}–{}, шаг {step} мин.\nДобавляйте слоты: /addworkday (сегодня) или /addworkdays N (N дней).",
                    fmt_minutes(from_m),
                    fmt_minutes(to_m)
                ),
            )
            .await?;
        }
        Command::AddWorkDay => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let Some((from_m, to_m, step)) = db.get_workday()? else {
                bot.send_message(
                    chat_id,
                    "Сначала задайте стандартный график: /setworkday 08:00 20:00 60",
                )
                .await?;
                return Ok(());
            };
            let today = today_local();
            let dts = gen_work_slots(&[today], from_m, to_m, step);
            add_slots_and_report(&bot, chat_id, &db, dts, "сегодня").await?;
        }
        Command::AddWorkDays => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            let Some(days) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
                bot.send_message(chat_id, "Использование: /addworkdays 7 (от 1 до 90 дней)")
                    .await?;
                return Ok(());
            };
            if !(1..=90).contains(&days) {
                bot.send_message(chat_id, "Количество дней — от 1 до 90.")
                    .await?;
                return Ok(());
            }
            let Some((from_m, to_m, step)) = db.get_workday()? else {
                bot.send_message(
                    chat_id,
                    "Сначала задайте стандартный график: /setworkday 08:00 20:00 60",
                )
                .await?;
                return Ok(());
            };
            let today = today_local();
            let dates: Vec<NaiveDate> = (0..days)
                .filter_map(|i| today.checked_add_signed(chrono::Duration::days(i as i64)))
                .collect();
            let dts = gen_work_slots(&dates, from_m, to_m, step);
            add_slots_and_report(
                &bot,
                chat_id,
                &db,
                dts,
                &format!("{} дн., начиная с сегодня", dates.len()),
            )
            .await?;
        }
        Command::DelWorkDay => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            if args.len() != 2 {
                bot.send_message(chat_id, "Использование: /delworkday 2026-11-08")
                    .await?;
                return Ok(());
            }
            let Some(d) = parse_date(args[1]) else {
                bot.send_message(chat_id, "Не понял дату. Пример: /delworkday 2026-11-08")
                    .await?;
                return Ok(());
            };
            let Some((lo, hi)) = day_bounds_utc(d) else {
                bot.send_message(chat_id, "Не удалось обработать эту дату.")
                    .await?;
                return Ok(());
            };
            let (deleted, booked) = db.delete_free_slots_range(lo, hi)?;
            bot.send_message(
                chat_id,
                format!("🗑 Удалено слотов за {}: {deleted}.", fmt_date_key(args[1])),
            )
            .await?;
            if booked > 0 {
                bot.send_message(chat_id, format!("⚠️ Пропущено занятых слотов: {booked}."))
                    .await?;
            }
        }
        Command::DelWorkDays => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            let Some(days) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
                bot.send_message(chat_id, "Использование: /delworkdays 7 (от 1 до 90 дней)")
                    .await?;
                return Ok(());
            };
            if !(1..=90).contains(&days) {
                bot.send_message(chat_id, "Количество дней — от 1 до 90.")
                    .await?;
                return Ok(());
            }
            let today = today_local();
            let Some(end_date) = today.checked_add_signed(chrono::Duration::days(days as i64))
            else {
                bot.send_message(chat_id, "Не удалось обработать диапазон.")
                    .await?;
                return Ok(());
            };
            let Some((lo, _)) = day_bounds_utc(today) else {
                bot.send_message(chat_id, "Не удалось обработать дату.")
                    .await?;
                return Ok(());
            };
            let Some((hi, _)) = day_bounds_utc(end_date) else {
                bot.send_message(chat_id, "Не удалось обработать дату.")
                    .await?;
                return Ok(());
            };
            let (deleted, booked) = db.delete_free_slots_range(lo, hi)?;
            bot.send_message(
                chat_id,
                format!("🗑 Удалено слотов на {days} дн. с сегодня: {deleted}."),
            )
            .await?;
            if booked > 0 {
                bot.send_message(chat_id, format!("⚠️ Пропущено занятых слотов: {booked}."))
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
        Command::Ban | Command::Unban => {
            if let Some(res) = guard(is_trainer).await {
                return res;
            }
            let ban = matches!(cmd, Command::Ban);
            let args: Vec<&str> = msg
                .text()
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            if args.len() != 2 {
                bot.send_message(
                    chat_id,
                    "Использование: /ban <id или @username> (id — в /users)",
                )
                .await?;
                return Ok(());
            }
            let target = args[1];
            let target_id = if let Ok(id) = target.parse::<i64>() {
                Some(id)
            } else {
                target
                    .strip_prefix('@')
                    .and_then(|un| db.get_user_by_username(un).ok().flatten())
                    .map(|u| u.chat_id)
            };
            let Some(target_id) = target_id else {
                bot.send_message(
                    chat_id,
                    "Не нашёл пользователя. Укажите id или @username (см. /users).",
                )
                .await?;
                return Ok(());
            };
            if target_id == cfg.trainer_chat_id {
                bot.send_message(chat_id, "Тренера забанить нельзя 😄")
                    .await?;
                return Ok(());
            }
            if !db.set_banned(target_id, ban)? {
                bot.send_message(chat_id, "Пользователь не найден — он ещё не писал боту.")
                    .await?;
                return Ok(());
            }
            let action = if ban {
                "⛔ Заблокирован"
            } else {
                "✅ Разблокирован"
            };
            bot.send_message(chat_id, format!("{action}: {}", user_desc(&db, target_id)))
                .await?;
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
    if !is_trainer && db.is_banned(chat_id.0)? {
        bot.send_message(chat_id, "⛔ Доступ к боту закрыт.")
            .await?;
        return Ok(());
    }
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
    if user_id != cfg.trainer_chat_id && db.is_banned(user_id)? {
        bot.answer_callback_query(q.id.clone())
            .text("⛔ Доступ к боту закрыт.")
            .show_alert(true)
            .await?;
        return Ok(());
    }
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
                            "✅ Вы записаны: {range}.\nЗа 2 часа до тренировки пришлю напоминание."
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
