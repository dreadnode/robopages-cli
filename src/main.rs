#[macro_use]
extern crate anyhow;

mod book;
mod cli;
mod runtime;

use clap::Parser;
use cli::Arguments;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Arguments::parse();

    if std::env::var_os("RUST_LOG").is_none() {
        // set `RUST_LOG=debug` to see debug logs
        // NOTE: actix_server is waaaay too verbose at the info level -.-
        std::env::set_var("RUST_LOG", "info,actix_server=warn");
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_module_path(false)
        .format_target(false)
        .init();

    let result = match args.command {
        cli::Command::Install(args) => cli::install(args).await,
        cli::Command::Create(args) => cli::create(args).await,
        cli::Command::View(args) => cli::view(args).await,
        cli::Command::Serve(args) => cli::serve(args).await,
        cli::Command::Run(args) => cli::run(args).await,
    };

    if let Err(e) = result {
        log::error!("{:?}", e);
        std::process::exit(1);
    }

    Ok(())
}
