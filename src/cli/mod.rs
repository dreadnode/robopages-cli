use std::error::Error;

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

mod create;
mod install;
mod run;
mod serve;
mod view;

pub(crate) use create::*;
pub(crate) use install::*;
pub(crate) use run::*;
pub(crate) use serve::*;
pub(crate) use view::*;

use crate::book::templates::Template;

const DEFAULT_REPO: &str = "dreadnode/robopages";
const DEFAULT_PATH: &str = "~/.robopages/";

#[derive(Debug, Parser)]
#[clap(name = "robopages", version)]
pub(crate) struct Arguments {
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Install robopages from a given repository or ZIP archive.
    Install(InstallArgs),
    /// Create a new robopage file.
    Create(CreateArgs),
    /// View currently installed robopages.
    View(ViewArgs),
    /// Serve the robopages as a local API.
    Serve(ServeArgs),
    /// Execute a function from the robopages.
    Run(RunArgs),
}

#[derive(Debug, Args)]
pub(crate) struct InstallArgs {
    /// Repository user/name, URL or ZIP archive path.
    #[clap(long, short = 'S', default_value = DEFAULT_REPO)]
    source: String,
    /// Destination path.
    #[clap(long, short = 'P', default_value = DEFAULT_PATH)]
    path: Utf8PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct CreateArgs {
    /// Template name.
    #[clap(long, short = 'T', value_enum, default_value = "basic")]
    template: Template,
    /// File name.
    #[clap(long, short = 'N', default_value = "robopage.yml")]
    name: Utf8PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct ViewArgs {
    /// Base path to search for robopages.
    #[clap(long, short = 'P', default_value = DEFAULT_PATH)]
    path: Utf8PathBuf,
    /// Filter results by this string.
    #[clap(long, short = 'F')]
    filter: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    /// Base path to search for robopages.
    #[clap(long, short = 'P', default_value = DEFAULT_PATH)]
    path: Utf8PathBuf,
    /// Filter results by this string.
    #[clap(long, short = 'F')]
    filter: Option<String>,
    /// Address to bind to.
    #[clap(long, short = 'A', default_value = "127.0.0.1:8000")]
    address: String,
    /// If set, the tool will not attempt to pre build and pull all containers.
    #[clap(long)]
    lazy: bool,
    /// Maximum number of parallel calls to execute. Leave to 0 to use all available cores.
    #[clap(long, default_value = "0")]
    workers: usize,
}

#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// Base path to search for robopages.
    #[clap(long, short = 'P', default_value = DEFAULT_PATH)]
    path: Utf8PathBuf,
    /// Function name.
    #[clap(long, short = 'F')]
    function: String,
    /// Define one or more variables as key=value pairs.
    #[clap(long = "define", short = 'D', value_parser = parse_key_val::<String, String>, number_of_values = 1)]
    defines: Vec<(String, String)>,
    /// Execute the function without user interaction.
    #[clap(long, short = 'A')]
    auto: bool,
}

/// Parse a single key-value pair
fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: Error + Send + Sync + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}
