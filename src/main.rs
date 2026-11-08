mod config;
mod db;
mod handlers;
mod scheduler;

use std::sync::Arc;

use config::Config;
use db::Database;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, BotCommandScope, Recipient};
use teloxide::utils::command::BotCommands;

const CLIENT_COMMANDS: [&str; 4] = ["start", "help", "book", "my"];

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = Arc::new(Config::from_env());
    let db = Arc::new(Database::open(&cfg.db_path).expect("failed to open database"));
    let bot = Bot::new(cfg.bot_token.clone());

    let client_commands: Vec<BotCommand> = handlers::Command::bot_commands()
        .into_iter()
        .filter(|c| CLIENT_COMMANDS.contains(&c.command.as_str()))
        .collect();
    if let Err(e) = bot.set_my_commands(client_commands).await {
        log::warn!("failed to set default bot commands: {e}");
    }
    if let Err(e) = bot
        .set_my_commands(handlers::Command::bot_commands())
        .scope(BotCommandScope::Chat {
            chat_id: Recipient::Id(ChatId(cfg.trainer_chat_id)),
        })
        .await
    {
        log::warn!("failed to set trainer bot commands: {e}");
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
