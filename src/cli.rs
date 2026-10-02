//! Command-line interface: argument parsing and command dispatch.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::client::{Client, DEFAULT_BASE_URL, SearchRequest};
use crate::config::{self, Env, FileConfig, Overrides};
use crate::error::{EXIT_OK, EXIT_USAGE, Error};
use crate::output;
use crate::provider::Provider;
use crate::validate;

/// Test-only override of the API root; honoured only for loopback addresses so
/// the bearer token can never be sent to another host.
pub const ENV_API_BASE_URL: &str = "WS_API_BASE_URL";

const AFTER_HELP: &str = "\
Environment:
  CLOUDFLARE_API_TOKEN   API token (Workers AI: Read + AI Gateway: Read)   [required]
  CLOUDFLARE_ACCOUNT_ID  Cloudflare account ID                             [required]
  WS_GATEWAY_ID          AI Gateway ID                                     [default: default]
  WS_PROVIDER            ceramic | exa | linkup                            [default: ceramic]
  WS_LIMIT               Results per search, 1-10                          [default: 10]
  WS_BYOK_ALIAS          Stored provider key to bill instead of credits
  WS_TIMEOUT_SECS        Request timeout in seconds, 1-300                 [default: 30]
  WS_CONFIG              Config file path

Precedence: flag > environment > config file > default.
Exit status: 0 success, 1 request failed, 2 usage or configuration error.";

/// Search the web from your terminal via Cloudflare's AI Gateway Web Search API.
#[derive(Debug, Parser)]
#[command(name = "ws", version, about, after_help = AFTER_HELP)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a web search (each search is billed by the provider).
    Search(SearchArgs),
    /// List the available search providers and which one is selected.
    Providers {
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Show or change saved defaults (provider and gateway).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// The search query (1-1,024 characters); several words are joined with spaces.
    #[arg(required = true, value_name = "QUERY")]
    query: Vec<String>,
    /// Search provider: ceramic, exa or linkup.
    #[arg(short, long, value_name = "NAME")]
    provider: Option<Provider>,
    /// AI Gateway ID.
    #[arg(short, long, value_name = "ID")]
    gateway: Option<String>,
    /// Maximum number of results (1-10).
    #[arg(short = 'n', long, value_name = "N")]
    limit: Option<u64>,
    /// Alias of a provider key stored on the gateway (bring your own key).
    #[arg(long, value_name = "ALIAS")]
    byok_alias: Option<String>,
    /// Cloudflare account ID (overrides `CLOUDFLARE_ACCOUNT_ID`).
    #[arg(long, value_name = "ID")]
    account_id: Option<String>,
    /// Request timeout in seconds (1-300).
    #[arg(long, value_name = "SECS")]
    timeout: Option<u64>,
    /// Emit one JSON object with complete results instead of text.
    #[arg(long)]
    json: bool,
    /// Show whole snippets in text output instead of the first 300 characters.
    #[arg(long, conflicts_with = "json")]
    full: bool,
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Show the settings in effect and where each comes from.
    Show,
    /// Print the config file path.
    Path,
    /// Save a default: `ws config set provider exa`.
    Set {
        /// Which default to set.
        key: ConfigKey,
        /// The new value.
        value: String,
    },
    /// Remove a saved default.
    Unset {
        /// Which default to remove.
        key: ConfigKey,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ConfigKey {
    /// Default search provider.
    Provider,
    /// Default AI Gateway ID.
    Gateway,
}

/// Runs `ws` with injected arguments, environment and output streams and
/// returns the process exit status.
pub fn run<I, T>(args: I, env: Env<'_>, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let rendered = error.render();
            return if error.use_stderr() {
                let _ = write!(stderr, "{rendered}");
                EXIT_USAGE
            } else {
                let _ = write!(stdout, "{rendered}");
                EXIT_OK
            };
        }
    };
    match dispatch(cli.command, env, stdout) {
        Ok(()) => EXIT_OK,
        // A closed pipe (`ws search x | head -1`) is not a failure.
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(error) => {
            let _ = writeln!(stderr, "ws: {error}");
            error.exit_code()
        }
    }
}

fn dispatch(command: Command, env: Env<'_>, stdout: &mut dyn Write) -> Result<(), Error> {
    match command {
        Command::Search(args) => search(args, env, stdout),
        Command::Providers { json } => {
            let preferences =
                config::resolve_preferences(&Overrides::default(), env, &load_file(env)?)?;
            Ok(output::write_providers(stdout, preferences.provider, json)?)
        }
        Command::Config { action } => configure(action, env, stdout),
    }
}

fn load_file(env: Env<'_>) -> Result<FileConfig, Error> {
    config::config_path(env, cfg!(windows))
        .map_or_else(|| Ok(FileConfig::default()), |path| FileConfig::load(&path))
}

fn required_config_path(env: Env<'_>) -> Result<PathBuf, Error> {
    config::config_path(env, cfg!(windows)).ok_or_else(|| {
        Error::Config(format!(
            "cannot locate a config directory; set {} to a file path",
            config::ENV_CONFIG
        ))
    })
}

/// Accepts only `http://127.0.0.1:PORT`, `http://localhost:PORT` or `http://[::1]:PORT`.
fn loopback_base_url(value: &str) -> Result<&str, Error> {
    let port = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix));
    match port {
        Some(port) if port.parse::<u16>().is_ok() => Ok(value),
        _ => Err(Error::Config(format!(
            "{ENV_API_BASE_URL} is for local testing only and must be http://127.0.0.1:PORT, \
             http://localhost:PORT or http://[::1]:PORT"
        ))),
    }
}

fn search(args: SearchArgs, env: Env<'_>, stdout: &mut dyn Write) -> Result<(), Error> {
    let overrides = Overrides {
        provider: args.provider,
        gateway_id: args.gateway,
        limit: args.limit,
        byok_alias: args.byok_alias,
        account_id: args.account_id,
        timeout_secs: args.timeout,
    };
    let query = args.query.join(" ");
    let query = validate::query(&query)?;
    let settings = config::resolve(&overrides, env, &load_file(env)?)?;
    let base_url = match config::env_value(env, ENV_API_BASE_URL)? {
        Some(value) => loopback_base_url(&value)?.to_owned(),
        None => DEFAULT_BASE_URL.to_owned(),
    };

    let request = SearchRequest::new(
        query,
        settings.preferences.provider,
        settings.limit,
        settings.byok_alias.as_deref(),
        &settings.preferences.gateway_id,
    );
    let outcome =
        Client::new(&base_url, settings.timeout).search(&settings.credentials, &request)?;

    if args.json {
        output::write_json(stdout, &settings.preferences, &outcome)?;
    } else {
        output::write_text(stdout, &settings.preferences, &outcome, args.full)?;
    }
    Ok(())
}

fn configure(action: ConfigAction, env: Env<'_>, stdout: &mut dyn Write) -> Result<(), Error> {
    match action {
        ConfigAction::Path => {
            writeln!(stdout, "{}", required_config_path(env)?.display())?;
        }
        ConfigAction::Show => {
            let preferences =
                config::resolve_preferences(&Overrides::default(), env, &load_file(env)?)?;
            let set = |key: &str| {
                if env(key).is_some_and(|v| !v.is_empty()) {
                    "set"
                } else {
                    "not set"
                }
            };
            let path = config::config_path(env, cfg!(windows))
                .map_or_else(|| "(none)".to_owned(), |p| p.display().to_string());
            writeln!(
                stdout,
                "provider     {} ({})",
                preferences.provider, preferences.provider_source
            )?;
            writeln!(
                stdout,
                "gateway      {} ({})",
                output::sanitize(&preferences.gateway_id),
                preferences.gateway_source
            )?;
            writeln!(stdout, "account id   {}", set(config::ENV_ACCOUNT_ID))?;
            writeln!(stdout, "api token    {}", set(config::ENV_API_TOKEN))?;
            writeln!(stdout, "config file  {path}")?;
        }
        ConfigAction::Set { key, value } => {
            let path = required_config_path(env)?;
            let mut file = FileConfig::load(&path)?;
            match key {
                ConfigKey::Provider => file.provider = Some(value.parse::<Provider>()?),
                ConfigKey::Gateway => {
                    file.gateway_id = Some(validate::gateway_id(&value)?.to_owned());
                }
            }
            file.save(&path)?;
            writeln!(stdout, "saved to {}", path.display())?;
        }
        ConfigAction::Unset { key } => {
            let path = required_config_path(env)?;
            let mut file = FileConfig::load(&path)?;
            match key {
                ConfigKey::Provider => file.provider = None,
                ConfigKey::Gateway => file.gateway_id = None,
            }
            file.save(&path)?;
            writeln!(stdout, "saved to {}", path.display())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn base_url_override_is_loopback_only() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://localhost:1",
            "http://[::1]:65535",
        ] {
            assert_eq!(loopback_base_url(ok).unwrap(), ok);
        }
        for bad in [
            "https://evil.example",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:80@evil.example",
            "http://127.0.0.1:80/path",
            "http://127.0.0.1.evil.example:80",
            "http://localhost.evil.example:80",
            "https://127.0.0.1:443",
            "http://127.0.0.1:99999",
        ] {
            assert!(loopback_base_url(bad).is_err(), "{bad}");
        }
    }
}
