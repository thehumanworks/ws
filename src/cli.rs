//! Command-line interface: argument parsing and command dispatch.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::backend::Backend;
use crate::brave;
use crate::client::{Client, DEFAULT_BASE_URL, SearchRequest};
use crate::config::{self, Env, FileConfig, Overrides};
use crate::error::{EXIT_OK, EXIT_USAGE, Error};
use crate::extract::Document;
use crate::fetch::{self, Access, ContentKind, Fetcher, Page};
use crate::lightpanda::{self, Browser, Dump, Rendered, Request};
use crate::markdown;
use crate::output::{self, Treatment};
use crate::provider::Provider;
use crate::render::Renderer;
use crate::validate;

/// Test-only override of the API root; honoured only for loopback addresses so
/// the bearer token can never be sent to another host.
pub const ENV_API_BASE_URL: &str = "WS_API_BASE_URL";

const AFTER_HELP: &str = "\
Environment:
  WS_BACKEND             lightpanda | cloudflare                 [platform default]
  CLOUDFLARE_API_TOKEN   API token. search: Workers AI: Read + AI Gateway: Read;
                         fetch --render: Browser Rendering: Edit; plain fetch: unused
  CLOUDFLARE_ACCOUNT_ID  Cloudflare account ID          [search and fetch --render]
  WS_GATEWAY_ID          AI Gateway ID                                     [default: default]
  WS_PROVIDER            ceramic | exa | linkup                            [default: ceramic]
  WS_LIMIT               Results per search, 1-10                          [default: 10]
  WS_BYOK_ALIAS          Stored provider key to bill instead of credits
  WS_TIMEOUT_SECS        Request timeout in seconds, 1-300                 [default: 30]
  WS_CONFIG              Config file path

Precedence: flag > environment > config file > default.
Exit status: 0 success, 1 request failed, 2 usage or configuration error.";

/// Search with the bundled Lightpanda browser or Cloudflare, and read pages
/// as Markdown, HTML, JSON or a text-only PNG.
#[derive(Debug, Parser)]
#[command(name = "ws", version, about, after_help = AFTER_HELP)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search Brave locally, or a billed provider with --backend cloudflare.
    Search(SearchArgs),
    /// Read a page locally or via direct HTTP; Cloudflare --render is metered.
    Fetch(FetchArgs),
    /// List the available search providers and which one is selected.
    Providers {
        /// Runtime whose providers to list.
        #[arg(long)]
        backend: Option<Backend>,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Show or change saved defaults (backend and Cloudflare preferences).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// Retrieval runtime (Lightpanda uses Brave; Cloudflare uses --provider).
    #[arg(long)]
    backend: Option<Backend>,
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

#[derive(Debug, Args)]
struct FetchArgs {
    /// Retrieval runtime: bundled local browser or Cloudflare/direct HTTP.
    #[arg(long)]
    backend: Option<Backend>,
    /// The page to fetch: an absolute http:// or https:// URL.
    #[arg(value_name = "URL")]
    url: String,
    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Markdown, value_name = "FORMAT")]
    format: Format,
    /// Keep the whole page: do not drop navigation, headers, footers, sidebars,
    /// hidden elements or controls, and do not narrow to the main content.
    #[arg(long)]
    raw: bool,
    /// On Cloudflare, opt into metered Browser Run (requires credentials).
    /// Lightpanda already renders JavaScript, so this flag is redundant there.
    #[arg(long)]
    render: bool,
    /// Allow hosts that are not on the public internet (localhost, private
    /// networks, link-local). Refused by default.
    #[arg(long)]
    allow_private: bool,
    /// Request timeout in seconds (1-300), redirects included.
    #[arg(long, value_name = "SECS")]
    timeout: Option<u64>,
}

/// Below this many visible characters a page is reported as nearly empty.
const SPARSE_PAGE_CHARS: usize = 20;

/// How `ws fetch` prints a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// The main content converted to Markdown.
    Markdown,
    /// Main content as HTML (--raw: whole rendered DOM or direct HTTP body).
    Html,
    /// One JSON object: URLs, status, content type, title and Markdown.
    Json,
    /// Whole-page text-only PNG (Lightpanda only); writes binary bytes.
    Png,
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
    /// Default retrieval runtime.
    Backend,
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
    run_with_browser(args, env, stdout, stderr, &lightpanda::Bundled::new(env))
}

/// Runs the CLI with an injected browser, for deterministic credential-free
/// integration tests. Normal execution always uses [`lightpanda::Bundled`].
pub fn run_with_browser<I, T>(
    args: I,
    env: Env<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    browser: &dyn Browser,
) -> u8
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
    match dispatch(cli.command, env, stdout, stderr, browser) {
        Ok(()) => EXIT_OK,
        // A closed pipe (`ws search x | head -1`) is not a failure.
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe => EXIT_OK,
        Err(error) => {
            let _ = writeln!(stderr, "ws: {error}");
            error.exit_code()
        }
    }
}

fn dispatch(
    command: Command,
    env: Env<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    browser: &dyn Browser,
) -> Result<(), Error> {
    match command {
        Command::Search(args) => search(args, env, stdout, browser),
        Command::Fetch(args) => fetch_page(&args, env, stdout, stderr, browser),
        Command::Providers { backend, json } => {
            let file = load_file(env)?;
            if config::resolve_backend(backend, env, &file)?.0 == Backend::Lightpanda {
                return Ok(output::write_local_providers(stdout, json)?);
            }
            let preferences = config::resolve_preferences(&Overrides::default(), env, &file)?;
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

fn api_base_url(env: Env<'_>) -> Result<String, Error> {
    Ok(match config::env_value(env, ENV_API_BASE_URL)? {
        Some(value) => loopback_base_url(&value)?.to_owned(),
        None => DEFAULT_BASE_URL.to_owned(),
    })
}

fn search(
    args: SearchArgs,
    env: Env<'_>,
    stdout: &mut dyn Write,
    browser: &dyn Browser,
) -> Result<(), Error> {
    let query = args.query.join(" ");
    let query = validate::query(&query)?;
    let file = load_file(env)?;
    let backend = config::resolve_backend(args.backend, env, &file)?.0;
    if backend == Backend::Lightpanda {
        if args.provider.is_some()
            || args.gateway.is_some()
            || args.byok_alias.is_some()
            || args.account_id.is_some()
        {
            return Err(Error::Config("--provider, --gateway, --byok-alias and --account-id require --backend cloudflare; Lightpanda searches Brave".to_owned()));
        }
        let limit = config::resolve_limit(args.limit, env)?;
        let start = Instant::now();
        let result = browser.retrieve(&Request {
            url: brave::search_url(query),
            dump: Dump::Html,
            access: Access::PublicOnly,
            timeout: config::resolve_timeout(args.timeout, env)?,
        })?;
        let Rendered::Html(page) = result else {
            return Err(Error::UnexpectedResponse(
                "browser returned PNG for a search".to_owned(),
            ));
        };
        let items = brave::extract(&page.body, limit)?;
        output::write_local_search(stdout, &items, start.elapsed(), args.json, args.full)?;
        return Ok(());
    }
    let overrides = Overrides {
        provider: args.provider,
        gateway_id: args.gateway,
        limit: args.limit,
        byok_alias: args.byok_alias,
        account_id: args.account_id,
        timeout_secs: args.timeout,
    };
    let settings = config::resolve(&overrides, env, &file)?;
    let base_url = api_base_url(env)?;

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

/// Gets the page: one credential-free request, or one Browser Run call.
fn retrieve(args: &FetchArgs, env: Env<'_>) -> Result<Page, Error> {
    let url = validate::url(&args.url)?;
    let timeout = config::resolve_timeout(args.timeout, env)?;
    if args.render {
        if args.allow_private {
            return Err(Error::Config(
                "--allow-private cannot be used with --render on the cloudflare backend".to_owned(),
            ));
        }
        let credentials = config::resolve_credentials(&Overrides::default(), env)?;
        return Renderer::new(&api_base_url(env)?, timeout).render(&credentials, url);
    }
    let accept = match args.format {
        Format::Html => fetch::ACCEPT_HTML,
        Format::Markdown | Format::Json | Format::Png => fetch::ACCEPT_MARKDOWN,
    };
    let access = if args.allow_private {
        Access::AllowPrivate
    } else {
        Access::PublicOnly
    };
    Fetcher::new(timeout, access).get(url, accept)
}

/// Says on stderr why a page came out nearly empty and which flag would help.
fn note_if_sparse(args: &FetchArgs, rendered: bool, content: &str, stderr: &mut dyn Write) {
    if content.chars().filter(|c| !c.is_whitespace()).count() >= SPARSE_PAGE_CHARS {
        return;
    }
    let advice = match (args.raw, rendered) {
        (false, false) => "try --raw to keep the whole page, or --render if it needs JavaScript",
        (true, false) => "try --render if it needs JavaScript",
        (false, true) => "try --raw to keep the whole page",
        (true, true) => return,
    };
    let _ = writeln!(
        stderr,
        "ws: note: almost no readable text was found; {advice}"
    );
}

/// `ws fetch`: validate locally, retrieve once, select the content, render.
fn fetch_page(
    args: &FetchArgs,
    env: Env<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    browser: &dyn Browser,
) -> Result<(), Error> {
    let backend = config::resolve_backend(args.backend, env, &load_file(env)?)?.0;
    if backend == Backend::Cloudflare && args.format == Format::Png {
        return Err(Error::Config("--format png requires --backend lightpanda; PNG never starts a billed Cloudflare render".to_owned()));
    }
    let page = if backend == Backend::Lightpanda {
        let url = validate::url(&args.url)?;
        let result = browser.retrieve(&Request {
            url: url.to_owned(),
            dump: if args.format == Format::Png {
                Dump::Png
            } else {
                Dump::Html
            },
            access: if args.allow_private {
                Access::AllowPrivate
            } else {
                Access::PublicOnly
            },
            timeout: config::resolve_timeout(args.timeout, env)?,
        })?;
        match result {
            Rendered::Png(bytes) if args.format == Format::Png => {
                stdout.write_all(&bytes)?;
                return Ok(());
            }
            Rendered::Html(page) if args.format != Format::Png => page,
            Rendered::Html(_) | Rendered::Png(_) => {
                return Err(Error::UnexpectedResponse(
                    "browser returned the wrong output format".to_owned(),
                ));
            }
        }
    } else {
        retrieve(args, env)?
    };
    let mut treatment = Treatment {
        extracted: false,
        rendered: args.render || backend == Backend::Lightpanda,
    };
    if page.kind != ContentKind::Html {
        // Markdown and other text are shown as served.
        return Ok(match args.format {
            Format::Json => output::write_page_json(stdout, &page, None, treatment, &page.body),
            Format::Markdown | Format::Html | Format::Png => output::write_page(stdout, &page.body),
        }?);
    }
    if args.raw && args.format == Format::Html {
        return Ok(output::write_page(stdout, &page.body)?);
    }

    let document = Document::parse(&page.body)?;
    let title = document.title();
    let document = if args.raw {
        document
    } else {
        treatment.extracted = true;
        document.main_content()
    };
    let content = match args.format {
        Format::Html => document.to_html()?,
        Format::Markdown | Format::Json | Format::Png => markdown::from_node(document.node()),
    };
    note_if_sparse(args, treatment.rendered, &content, stderr);
    if args.format == Format::Json {
        output::write_page_json(stdout, &page, title.as_deref(), treatment, &content)?;
    } else {
        output::write_page(stdout, &content)?;
    }
    Ok(())
}

fn configure(action: ConfigAction, env: Env<'_>, stdout: &mut dyn Write) -> Result<(), Error> {
    match action {
        ConfigAction::Path => {
            writeln!(stdout, "{}", required_config_path(env)?.display())?;
        }
        ConfigAction::Show => {
            let file = load_file(env)?;
            let (backend, source) = config::resolve_backend(None, env, &file)?;
            writeln!(stdout, "backend      {backend} ({source})")?;
            let preferences = if backend == Backend::Cloudflare {
                config::resolve_preferences(&Overrides::default(), env, &file)?
            } else {
                let (provider, provider_source) = config::pick(
                    None,
                    env(config::ENV_PROVIDER),
                    file.provider.clone(),
                    Provider::default().to_string(),
                );
                let (gateway_id, gateway_source) = config::pick(
                    None,
                    env(config::ENV_GATEWAY_ID),
                    file.gateway_id,
                    config::DEFAULT_GATEWAY_ID.to_owned(),
                );
                writeln!(stdout, "search       brave (Lightpanda)")?;
                writeln!(
                    stdout,
                    "provider     {} ({provider_source}; saved Cloudflare preference)",
                    output::sanitize(&provider)
                )?;
                writeln!(
                    stdout,
                    "gateway      {} ({gateway_source}; saved Cloudflare preference)",
                    output::sanitize(&gateway_id)
                )?;
                config::Preferences {
                    provider: Provider::default(),
                    provider_source,
                    gateway_id,
                    gateway_source,
                }
            };
            let set = |key: &str| {
                if env(key).is_some_and(|v| !v.is_empty()) {
                    "set"
                } else {
                    "not set"
                }
            };
            let path = config::config_path(env, cfg!(windows))
                .map_or_else(|| "(none)".to_owned(), |p| p.display().to_string());
            if backend == Backend::Cloudflare {
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
            }
            writeln!(stdout, "account id   {}", set(config::ENV_ACCOUNT_ID))?;
            writeln!(stdout, "api token    {}", set(config::ENV_API_TOKEN))?;
            writeln!(stdout, "config file  {path}")?;
        }
        ConfigAction::Set { key, value } => {
            let path = required_config_path(env)?;
            let mut file = FileConfig::load(&path)?;
            match key {
                ConfigKey::Backend => file.backend = Some(value.parse()?),
                ConfigKey::Provider => file.provider = Some(value.parse::<Provider>()?.to_string()),
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
                ConfigKey::Backend => file.backend = None,
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
