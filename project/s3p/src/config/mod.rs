use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use miette::{miette, Context, IntoDiagnostic, Result};
use schematic::{Config, ConfigEnum, ConfigLoader, ValidateError};
use serde::{Deserialize, Serialize};

pub(crate) fn generate(file: impl AsRef<Path>) -> Result<()> {
    let path = file.as_ref();
    let file = match path.is_dir() {
        true => path.join("config.toml"),
        false => {
            if path.exists() {
                return Err(
                    miette!("Could not create file {:?}", path).context("File already exists")
                );
            }
            path.to_path_buf()
        }
    };

    let mut config = AppConfig::default();

    // Add some middlewares to show config format
    config
        .middlewares
        .push(MiddlewareType::Cache(CacheMiddlewareConfig::default()));
    config.middlewares.push(MiddlewareType::Identity);

    // Add client credentials to show config format
    match config.client {
        ClientType::S3(ref mut c) => c.credentials = Some(S3Credentials::default()),
        _ => unimplemented!(),
    }

    let mut f = File::create(file.as_path())
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to open file {:?}", file))?;

    let config_str = toml::to_string_pretty(&config)
        .into_diagnostic()
        .wrap_err_with(|| "Failed to serialize default configuration")?;

    f.write_all(config_str.as_bytes())
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to write file {:?}", file))?;

    Ok(())
}

#[allow(unused)]
pub(crate) fn load(file: impl AsRef<Path>) -> Result<AppConfig> {
    let path = file.as_ref();

    let file = match path.is_dir() {
        true => path.join("config.toml"),
        _ => path.to_path_buf(),
    };

    let config = ConfigLoader::<AppConfig>::new()
        .file(file.as_path())?
        .load()?
        .config;

    Ok(config)
}

#[allow(unused)]
pub(crate) fn load_or_generate(file: impl AsRef<Path>) -> Result<AppConfig> {
    let path = file.as_ref();

    let file = match path.is_dir() {
        true => path.join("config.toml"),
        _ => path.to_path_buf(),
    };

    if path.exists() && path.is_file() {
        return load(file);
    }

    generate(file.as_path())?;
    load(file.as_path())
}

#[derive(Config, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub log_level: LogLevel,
    pub server: ServerType,
    pub middlewares: Vec<MiddlewareType>,
    pub client: ClientType,
}

#[derive(ConfigEnum, Clone, Default, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Debug,
    Warn,
    #[default]
    Critical,
    Off,
}

#[derive(Default)]
#[allow(unused)]
pub struct ServerContext {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerType {
    S3(S3ServerConfig),
}

impl Default for ServerType {
    fn default() -> Self {
        Self::S3(S3ServerConfig::default())
    }
}

#[derive(Config, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[config(context = ServerContext)]
#[serde(rename_all = "camelCase")]
pub struct S3ServerConfig {
    #[setting(default = "127.0.0.1")]
    pub host: String,
    #[setting(default = 4356)]
    pub port: u16,
    pub base_domain: Option<String>,
    #[setting(default = true, validate = validate_credentials)]
    pub validate_credentials: bool,
    pub credentials: Option<S3Credentials>,
}

fn validate_credentials(
    value: &bool,
    partial: &PartialS3ServerConfig,
    _context: &ServerContext,
) -> Result<(), ValidateError> {
    let has_creds = partial.credentials.as_ref().is_some();
    if *value && !has_creds {
        return Err(ValidateError::new(
            "validate_credentials was set to true but no credentials were given",
        ));
    }

    Ok(())
}

#[derive(Config, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Credentials {
    #[setting(default = "")]
    pub access_key_id: String,
    #[setting(default = "")]
    pub secret_key: String,
}

#[derive(Default)]
pub struct MiddlewareConfig {}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MiddlewareType {
    Identity,
    Cache(CacheMiddlewareConfig),
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = MiddlewareConfig)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct CacheMiddlewareConfig {
    #[setting(default = 500_000)]
    pub cache_size: u64,
    pub max_entry_size: Option<usize>,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
    pub ops: CacheOpsConfig,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(rename_all = "PascalCase")]
#[serde(rename_all = "PascalCase")]
pub struct CacheOpsConfig {
    pub get_object: GetObjectSetting,
    pub head_object: HeadObjectSetting,
    pub list_objects: ListObjectsSetting,
    pub list_object_versions: ListObjectVersionsSetting,
    pub head_bucket: HeadBucketSetting,
    pub list_buckets: ListBucketsSetting,
}

#[derive(Default)]
#[allow(unused)]
pub struct CacheOpSetting {
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct GetObjectSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct HeadObjectSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct ListObjectsSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct ListObjectVersionsSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct HeadBucketSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = CacheOpSetting)]
#[serde(rename_all = "camelCase")]
pub struct ListBucketsSetting {
    #[setting(default = true)]
    pub enabled: bool,
    pub ttl: Option<u64>,
    pub tti: Option<u64>,
}

#[derive(Default)]
pub struct ClientConfig {}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientType {
    S3(S3ClientConfig),
    Stub,
}

impl Default for ClientType {
    fn default() -> Self {
        Self::S3(S3ClientConfig::default())
    }
}

#[derive(Config, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[config(context = ClientConfig)]
#[serde(rename_all = "camelCase")]
pub struct S3ClientConfig {
    #[setting(validate = schematic::validate::url, default = "http://localhost:9000")]
    pub endpoint_url: String,
    #[setting(default = false)]
    pub force_path_style: bool,
    pub credentials: Option<S3Credentials>,
}

#[cfg(test)]
mod tests {
    use ctor::ctor;
    use miette::{Context, IntoDiagnostic, Result};
    use schematic::ConfigLoader;
    use tempfile::tempdir;
    use tracing::debug;

    use super::*;

    #[ctor]
    fn prepare() {
        let _ = crate::try_init_tracing();
    }

    #[test]
    fn generate_and_read_config() -> Result<()> {
        let temp_dir = tempdir()
            .into_diagnostic()
            .wrap_err_with(|| "Failed to create temporary directory")?;

        let config_file = temp_dir.path().join("config.toml");

        generate(config_file.as_path())?;

        let config = ConfigLoader::<AppConfig>::new()
            .file(config_file)?
            .load()?
            .config;

        debug!("{:#?}", config);

        Ok(())
    }
}
