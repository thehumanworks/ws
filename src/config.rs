//! Configuration: where each setting comes from and which source wins.
//!
//! Precedence, highest first: command-line flag, environment variable,
//! config file, built-in default. [`pick`] is the whole rule; it is modelled
//! and proved in `lean/WsSpec/Precedence.lean`.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::provider::Provider;
use crate::validate;

/// Environment variable holding the Cloudflare API token.
pub const ENV_API_TOKEN: &str = "CLOUDFLARE_API_TOKEN";
/// Environment variable holding the Cloudflare account ID.
pub const ENV_ACCOUNT_ID: &str = "CLOUDFLARE_ACCOUNT_ID";
/// Environment variable selecting the AI Gateway.
pub const ENV_GATEWAY_ID: &str = "WS_GATEWAY_ID";
/// Environment variable overriding the default provider.
pub const ENV_PROVIDER: &str = "WS_PROVIDER";
/// Environment variable setting the result limit.
pub const ENV_LIMIT: &str = "WS_LIMIT";
/// Environment variable naming a stored provider key (BYOK).
pub const ENV_BYOK_ALIAS: &str = "WS_BYOK_ALIAS";
/// Environment variable setting the request timeout in seconds.
pub const ENV_TIMEOUT_SECS: &str = "WS_TIMEOUT_SECS";
/// Environment variable overriding the config file location.
pub const ENV_CONFIG: &str = "WS_CONFIG";

/// Gateway used when none is configured; every account has one named `default`.
pub const DEFAULT_GATEWAY_ID: &str = "default";
/// Request timeout used when none is configured.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Read access to environment variables, injected so tests never touch the
/// real process environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Which configuration layer supplied a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A command-line flag.
    Flag,
    /// An environment variable.
    Env,
    /// The config file.
    File,
    /// The built-in default.
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Flag => "flag",
            Self::Env => "environment",
            Self::File => "config file",
            Self::Default => "default",
        })
    }
}

/// The precedence rule: flag, then environment, then file, then default.
pub fn pick<T>(flag: Option<T>, env: Option<T>, file: Option<T>, default: T) -> (T, Source) {
    match (flag, env, file) {
        (Some(value), _, _) => (value, Source::Flag),
        (None, Some(value), _) => (value, Source::Env),
        (None, None, Some(value)) => (value, Source::File),
        (None, None, None) => (default, Source::Default),
    }
}

/// Reads an environment variable, refusing a set-but-empty value so that a
/// typo such as `WS_PROVIDER=` never silently falls through to another layer.
///
/// # Errors
/// [`Error::Config`] when the variable is set to the empty string.
pub fn env_value(env: Env<'_>, key: &str) -> Result<Option<String>, Error> {
    match env(key) {
        Some(value) if value.is_empty() => Err(Error::Config(format!(
            "{key} is set but empty; unset it or give it a value"
        ))),
        other => Ok(other),
    }
}

/// Persistent user preferences. Secrets never live here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Default provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// Default AI Gateway ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_id: Option<String>,
}

impl FileConfig {
    /// Parses config file text.
    ///
    /// # Errors
    /// [`Error::Config`] on malformed TOML, unknown keys or invalid values.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let config: Self =
            toml::from_str(text).map_err(|e| Error::Config(format!("config file: {e}")))?;
        if let Some(id) = &config.gateway_id {
            let _: &str = validate::gateway_id(id)?;
        }
        Ok(config)
    }

    /// Loads the config file; a missing file is an empty configuration.
    ///
    /// # Errors
    /// [`Error::Io`] if the file cannot be read, [`Error::Config`] if it is invalid.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Writes the config file, creating parent directories as needed.
    ///
    /// # Errors
    /// [`Error::Io`] on filesystem failure.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let text = toml::to_string(self).map_err(|e| Error::Config(format!("config file: {e}")))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
        Ok(())
    }
}

/// Where the config file lives.
///
/// `$WS_CONFIG`, else `%APPDATA%\ws\config.toml`
/// on Windows, else `$XDG_CONFIG_HOME/ws/config.toml`, else
/// `$HOME/.config/ws/config.toml`. `None` when no base directory is known.
#[must_use]
pub fn config_path(env: Env<'_>, windows: bool) -> Option<PathBuf> {
    let non_empty = |key: &str| env(key).filter(|v| !v.is_empty());
    if let Some(explicit) = non_empty(ENV_CONFIG) {
        return Some(PathBuf::from(explicit));
    }
    let base = if windows {
        non_empty("APPDATA").map(PathBuf::from)
    } else {
        non_empty("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| non_empty("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|dir| dir.join("ws").join("config.toml"))
}

/// Values given on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `--provider`
    pub provider: Option<Provider>,
    /// `--gateway`
    pub gateway_id: Option<String>,
    /// `--limit`
    pub limit: Option<u64>,
    /// `--byok-alias`
    pub byok_alias: Option<String>,
    /// `--account-id`
    pub account_id: Option<String>,
    /// `--timeout`
    pub timeout_secs: Option<u64>,
}

/// The provider and gateway in effect, with the layer each came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// Provider in effect.
    pub provider: Provider,
    /// Layer that chose the provider.
    pub provider_source: Source,
    /// Gateway ID in effect.
    pub gateway_id: String,
    /// Layer that chose the gateway.
    pub gateway_source: Source,
}

/// Cloudflare credentials. `Debug` never prints the token.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// Cloudflare account ID (32 hex characters).
    pub account_id: String,
    /// Cloudflare API token.
    pub token: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("account_id", &self.account_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Everything needed to run one search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Provider and gateway.
    pub preferences: Preferences,
    /// Account and token.
    pub credentials: Credentials,
    /// Maximum number of results (1..=10).
    pub limit: u8,
    /// Stored provider key to bill, if any.
    pub byok_alias: Option<String>,
    /// Overall request deadline.
    pub timeout: Duration,
}

fn parse_number(key: &str, value: &str) -> Result<u64, Error> {
    value
        .parse()
        .map_err(|_| Error::Config(format!("{key} must be a whole number, got '{value}'")))
}

/// Resolves the provider and gateway. Only the winning layer is consulted,
/// except the config file, which is validated when loaded.
///
/// # Errors
/// [`Error::Config`], [`Error::Provider`] or [`Error::Invalid`] for bad values.
pub fn resolve_preferences(
    overrides: &Overrides,
    env: Env<'_>,
    file: &FileConfig,
) -> Result<Preferences, Error> {
    let env_provider = env_value(env, ENV_PROVIDER)?
        .map(|name| name.parse::<Provider>())
        .transpose()?;
    let (provider, provider_source) = pick(
        overrides.provider,
        env_provider,
        file.provider,
        Provider::default(),
    );

    let (gateway_id, gateway_source) = pick(
        overrides.gateway_id.clone(),
        env_value(env, ENV_GATEWAY_ID)?,
        file.gateway_id.clone(),
        DEFAULT_GATEWAY_ID.to_owned(),
    );
    let _: &str = validate::gateway_id(&gateway_id)?;

    Ok(Preferences {
        provider,
        provider_source,
        gateway_id,
        gateway_source,
    })
}

/// Resolves the Cloudflare credentials.
///
/// # Errors
/// [`Error::Config`] when a credential is missing, [`Error::Invalid`] when malformed.
pub fn resolve_credentials(overrides: &Overrides, env: Env<'_>) -> Result<Credentials, Error> {
    let account_id = match overrides.account_id.clone() {
        Some(id) => id,
        None => env_value(env, ENV_ACCOUNT_ID)?.ok_or_else(|| {
            Error::Config(format!(
                "{ENV_ACCOUNT_ID} is not set (or pass --account-id)"
            ))
        })?,
    };
    let _: &str = validate::account_id(&account_id)?;

    let token = env_value(env, ENV_API_TOKEN)?
        .ok_or_else(|| Error::Config(format!("{ENV_API_TOKEN} is not set")))?;
    let _: &str = validate::token(&token)?;

    Ok(Credentials { account_id, token })
}

/// Resolves every setting for a search from flags, environment and config file.
///
/// # Errors
/// [`Error::Config`], [`Error::Provider`] or [`Error::Invalid`]; nothing is sent
/// to Cloudflare until all of them pass.
pub fn resolve(overrides: &Overrides, env: Env<'_>, file: &FileConfig) -> Result<Settings, Error> {
    let preferences = resolve_preferences(overrides, env, file)?;

    let env_limit = env_value(env, ENV_LIMIT)?
        .map(|v| parse_number(ENV_LIMIT, &v))
        .transpose()?;
    let (limit, _) = pick(
        overrides.limit,
        env_limit,
        None,
        u64::from(validate::DEFAULT_LIMIT),
    );
    let limit = validate::limit(limit)?;

    let byok_alias = match overrides.byok_alias.clone() {
        Some(alias) => Some(alias),
        None => env_value(env, ENV_BYOK_ALIAS)?,
    };
    if let Some(alias) = &byok_alias {
        let _: &str = validate::alias(alias)?;
    }

    let env_timeout = env_value(env, ENV_TIMEOUT_SECS)?
        .map(|v| parse_number(ENV_TIMEOUT_SECS, &v))
        .transpose()?;
    let (timeout, _) = pick(
        overrides.timeout_secs,
        env_timeout,
        None,
        DEFAULT_TIMEOUT_SECS,
    );
    let timeout = Duration::from_secs(validate::timeout_secs(timeout)?);

    let credentials = resolve_credentials(overrides, env)?;

    Ok(Settings {
        preferences,
        credentials,
        limit,
        byok_alias,
        timeout,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::validate::ValidationError;

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    fn creds() -> Vec<(&'static str, &'static str)> {
        vec![(ENV_ACCOUNT_ID, ACCOUNT), (ENV_API_TOKEN, "tok")]
    }

    #[test]
    fn pick_follows_precedence() {
        assert_eq!(pick(Some(1), Some(2), Some(3), 4), (1, Source::Flag));
        assert_eq!(pick(None, Some(2), Some(3), 4), (2, Source::Env));
        assert_eq!(pick(None, None, Some(3), 4), (3, Source::File));
        assert_eq!(pick(None, None, None, 4), (4, Source::Default));
    }

    #[test]
    fn defaults_are_ceramic_and_default_gateway() {
        let env = env_of(&creds());
        let s = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap();
        assert_eq!(s.preferences.provider, Provider::Ceramic);
        assert_eq!(s.preferences.provider_source, Source::Default);
        assert_eq!(s.preferences.gateway_id, "default");
        assert_eq!(s.preferences.gateway_source, Source::Default);
        assert_eq!(s.limit, 10);
        assert_eq!(s.byok_alias, None);
        assert_eq!(s.timeout, Duration::from_secs(30));
        assert_eq!(s.credentials.account_id, ACCOUNT);
    }

    #[test]
    fn file_overrides_default() {
        let env = env_of(&creds());
        let file = FileConfig {
            provider: Some(Provider::Linkup),
            gateway_id: Some("file-gw".into()),
        };
        let s = resolve(&Overrides::default(), &env, &file).unwrap();
        assert_eq!(
            (s.preferences.provider, s.preferences.provider_source),
            (Provider::Linkup, Source::File)
        );
        assert_eq!(
            (
                s.preferences.gateway_id.as_str(),
                s.preferences.gateway_source
            ),
            ("file-gw", Source::File)
        );
    }

    #[test]
    fn env_overrides_file() {
        let mut pairs = creds();
        pairs.extend([
            (ENV_PROVIDER, "exa"),
            (ENV_GATEWAY_ID, "env-gw"),
            (ENV_LIMIT, "3"),
        ]);
        let env = env_of(&pairs);
        let file = FileConfig {
            provider: Some(Provider::Linkup),
            gateway_id: Some("file-gw".into()),
        };
        let s = resolve(&Overrides::default(), &env, &file).unwrap();
        assert_eq!(
            (s.preferences.provider, s.preferences.provider_source),
            (Provider::Exa, Source::Env)
        );
        assert_eq!(
            (
                s.preferences.gateway_id.as_str(),
                s.preferences.gateway_source
            ),
            ("env-gw", Source::Env)
        );
        assert_eq!(s.limit, 3);
    }

    #[test]
    fn flag_overrides_env_and_file() {
        let mut pairs = creds();
        pairs.extend([
            (ENV_PROVIDER, "exa"),
            (ENV_GATEWAY_ID, "env-gw"),
            (ENV_LIMIT, "3"),
            (ENV_BYOK_ALIAS, "env-alias"),
            (ENV_TIMEOUT_SECS, "5"),
        ]);
        let env = env_of(&pairs);
        let file = FileConfig {
            provider: Some(Provider::Linkup),
            gateway_id: Some("file-gw".into()),
        };
        let overrides = Overrides {
            provider: Some(Provider::Ceramic),
            gateway_id: Some("flag-gw".into()),
            limit: Some(7),
            byok_alias: Some("flag-alias".into()),
            account_id: None,
            timeout_secs: Some(9),
        };
        let s = resolve(&overrides, &env, &file).unwrap();
        assert_eq!(
            (s.preferences.provider, s.preferences.provider_source),
            (Provider::Ceramic, Source::Flag)
        );
        assert_eq!(
            (
                s.preferences.gateway_id.as_str(),
                s.preferences.gateway_source
            ),
            ("flag-gw", Source::Flag)
        );
        assert_eq!(s.limit, 7);
        assert_eq!(s.byok_alias.as_deref(), Some("flag-alias"));
        assert_eq!(s.timeout, Duration::from_secs(9));
    }

    #[test]
    fn env_alias_and_timeout_apply_without_flags() {
        let mut pairs = creds();
        pairs.extend([(ENV_BYOK_ALIAS, "env-alias"), (ENV_TIMEOUT_SECS, "5")]);
        let env = env_of(&pairs);
        let s = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap();
        assert_eq!(s.byok_alias.as_deref(), Some("env-alias"));
        assert_eq!(s.timeout, Duration::from_secs(5));
    }

    #[test]
    fn missing_credentials_are_config_errors() {
        let env = env_of(&[(ENV_API_TOKEN, "tok")]);
        let err = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap_err();
        assert!(
            matches!(&err, Error::Config(m) if m.contains(ENV_ACCOUNT_ID)),
            "{err}"
        );

        let env = env_of(&[(ENV_ACCOUNT_ID, ACCOUNT)]);
        let err = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap_err();
        assert!(
            matches!(&err, Error::Config(m) if m.contains(ENV_API_TOKEN)),
            "{err}"
        );
    }

    #[test]
    fn account_id_flag_overrides_env() {
        let env = env_of(&creds());
        let other = "ffffffffffffffffffffffffffffffff";
        let overrides = Overrides {
            account_id: Some(other.into()),
            ..Overrides::default()
        };
        let s = resolve(&overrides, &env, &FileConfig::default()).unwrap();
        assert_eq!(s.credentials.account_id, other);
    }

    #[test]
    fn invalid_values_are_rejected() {
        let cases: [(&str, &str); 7] = [
            (ENV_PROVIDER, "google"),
            (ENV_GATEWAY_ID, "two words"),
            (ENV_LIMIT, "11"),
            (ENV_LIMIT, "many"),
            (ENV_BYOK_ALIAS, "bad alias"),
            (ENV_TIMEOUT_SECS, "0"),
            (ENV_ACCOUNT_ID, "not-hex"),
        ];
        for (key, value) in cases {
            let mut pairs = creds();
            pairs.retain(|(k, _)| *k != key);
            pairs.push((key, value));
            let env = env_of(&pairs);
            let err = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap_err();
            assert_eq!(
                err.exit_code(),
                crate::error::EXIT_USAGE,
                "{key}={value}: {err}"
            );
        }
    }

    #[test]
    fn empty_environment_values_are_errors_not_fallthrough() {
        for key in [
            ENV_PROVIDER,
            ENV_GATEWAY_ID,
            ENV_LIMIT,
            ENV_BYOK_ALIAS,
            ENV_TIMEOUT_SECS,
            ENV_API_TOKEN,
        ] {
            let mut pairs = creds();
            pairs.retain(|(k, _)| *k != key);
            pairs.push((key, ""));
            let env = env_of(&pairs);
            let err = resolve(&Overrides::default(), &env, &FileConfig::default()).unwrap_err();
            assert!(
                matches!(&err, Error::Config(m) if m.contains("set but empty")),
                "{key}: {err}"
            );
        }
    }

    #[test]
    fn credentials_debug_redacts_token() {
        let c = Credentials {
            account_id: ACCOUNT.into(),
            token: "super-secret".into(),
        };
        let text = format!("{c:?}");
        assert!(!text.contains("super-secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn file_config_parsing() {
        assert_eq!(FileConfig::parse("").unwrap(), FileConfig::default());
        let parsed = FileConfig::parse("provider = \"exa\"\ngateway_id = \"gw\"\n").unwrap();
        assert_eq!(
            parsed,
            FileConfig {
                provider: Some(Provider::Exa),
                gateway_id: Some("gw".into())
            }
        );
        assert!(FileConfig::parse("provider = \"google\"").is_err());
        assert!(FileConfig::parse("token = \"secret\"").is_err());
        assert!(FileConfig::parse("gateway_id = \"\"").is_err());
        assert!(matches!(
            FileConfig::parse("gateway_id = \"a b\""),
            Err(Error::Invalid(ValidationError::InvalidGatewayId))
        ));
    }

    #[test]
    fn file_config_save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        assert_eq!(FileConfig::load(&path).unwrap(), FileConfig::default());
        let config = FileConfig {
            provider: Some(Provider::Linkup),
            gateway_id: Some("gw-1".into()),
        };
        config.save(&path).unwrap();
        assert_eq!(FileConfig::load(&path).unwrap(), config);
    }

    #[test]
    fn config_path_resolution() {
        let env = env_of(&[(ENV_CONFIG, "/x/custom.toml"), ("HOME", "/home/u")]);
        assert_eq!(
            config_path(&env, false),
            Some(PathBuf::from("/x/custom.toml"))
        );

        let env = env_of(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
        assert_eq!(
            config_path(&env, false),
            Some(PathBuf::from("/xdg/ws/config.toml"))
        );

        let env = env_of(&[("HOME", "/home/u")]);
        assert_eq!(
            config_path(&env, false),
            Some(PathBuf::from("/home/u/.config/ws/config.toml"))
        );

        let env = env_of(&[
            ("APPDATA", "C:/Users/u/AppData/Roaming"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            config_path(&env, true),
            Some(
                PathBuf::from("C:/Users/u/AppData/Roaming")
                    .join("ws")
                    .join("config.toml")
            )
        );

        let env = env_of(&[]);
        assert_eq!(config_path(&env, false), None);
        assert_eq!(config_path(&env, true), None);
    }
}
