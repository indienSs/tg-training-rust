mod config;
mod db;
mod handlers;
mod scheduler;

use std::sync::Arc;

use config::Config;
use db::Database;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = Arc::new(Config::from_env());
    let db = Arc::new(Database::open(&cfg.db_path).expect("failed to open database"));
    let bot = Bot::new(cfg.bot_token.clone());

    if let Err(e) = bot.set_my_commands(handlers::Command::bot_commands()).await {
        log::warn!("failed to set bot commands: {e}");
    }

    scheduler::spawn(bot.clone(), Arc::clone(&db));

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<handlers::Command>()
                .endpoint(handlers::handle_command),
        )
        .branch(Update::filter_message().endpoint(handlers::handle_text))
        .branch(Update::filter_callback_query().endpoint(handlers::handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![db, cfg])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
