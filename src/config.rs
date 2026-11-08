use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub bot_token: String,
    pub trainer_chat_id: i64,
    pub db_path: String,
}

fn env_or_embedded(name: &str, embedded: Option<&'static str>) -> String {
    if let Ok(v) = env::var(name)
        && !v.trim().is_empty()
    {
        return v;
    }
    embedded
        .map(str::to_string)
        .unwrap_or_else(|| panic!("{name} is not set: pass it as env var or embed at build time"))
}

fn default_db_path() -> String {
    env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|d| d.join("data").join("bot.db").to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "data/bot.db".to_string())
}

impl Config {
    pub fn from_env() -> Config {
        let trainer_raw = env_or_embedded("TRAINER_CHAT_ID", option_env!("TRAINER_CHAT_ID"));
        let trainer_chat_id = trainer_raw
            .trim()
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("TRAINER_CHAT_ID must be an integer chat id"));

        Config {
            bot_token: env_or_embedded("BOT_TOKEN", option_env!("BOT_TOKEN")),
            trainer_chat_id,
            db_path: env::var("DATABASE_PATH").unwrap_or_else(|_| default_db_path()),
        }
    }
}
