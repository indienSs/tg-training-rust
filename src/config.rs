use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub bot_token: String,
    pub trainer_chat_id: i64,
    pub db_path: String,
}

impl Config {
    pub fn from_env() -> Config {
        let trainer_chat_id = env::var("TRAINER_CHAT_ID")
            .unwrap_or_else(|_| panic!("TRAINER_CHAT_ID must be set"))
            .parse::<i64>()
            .unwrap_or_else(|_| panic!("TRAINER_CHAT_ID must be an integer chat id"));

        Config {
            bot_token: env::var("BOT_TOKEN").unwrap_or_else(|_| panic!("BOT_TOKEN must be set")),
            trainer_chat_id,
            db_path: env::var("DATABASE_PATH").unwrap_or_else(|_| "data/bot.db".to_string()),
        }
    }
}
