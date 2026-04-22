use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

const DEFAULT_PORTAL_URL: &str = "http://172.18.3.3/0.htm";
const DEFAULT_PROBE_URL: &str = "http://www.baidu.com/";
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 8;
const DEFAULT_ONLINE_CHECK_INTERVAL_SECS: u64 = 300;
const DEFAULT_OFFLINE_CHECK_INTERVAL_SECS: u64 = 15;
const DEFAULT_CAMPUS_CIDRS: &[&str] = &[];
const DEFAULT_CAMPUS_GATEWAYS: &[&str] = &["172.18.3.3", "172.18.2.2"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub auth: AuthConfig,
    pub detect: DetectConfig,
    pub daemon: DaemonConfig,
    pub campus: CampusConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub username: String,
    pub password: String,
    pub portal_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectConfig {
    pub probe_url: String,
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub online_check_interval_secs: u64,
    pub offline_check_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CampusConfig {
    pub ipv4_cidrs: Vec<String>,
    pub gateway_hosts: Vec<String>,
}

impl Default for CampusConfig {
    fn default() -> Self {
        Self {
            ipv4_cidrs: DEFAULT_CAMPUS_CIDRS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            gateway_hosts: DEFAULT_CAMPUS_GATEWAYS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auth: AuthConfig {
                username: String::new(),
                password: String::new(),
                portal_url: DEFAULT_PORTAL_URL.to_owned(),
            },
            detect: DetectConfig {
                probe_url: DEFAULT_PROBE_URL.to_owned(),
                request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            },
            daemon: DaemonConfig {
                online_check_interval_secs: DEFAULT_ONLINE_CHECK_INTERVAL_SECS,
                offline_check_interval_secs: DEFAULT_OFFLINE_CHECK_INTERVAL_SECS,
            },
            campus: CampusConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::app_dir()?.join("config.toml"))
    }

    pub fn log_path() -> Result<PathBuf> {
        Ok(Self::app_dir()?.join("daemon.log"))
    }

    fn app_dir() -> Result<PathBuf> {
        let dirs = BaseDirs::new()
            .ok_or_else(|| anyhow!("could not resolve a platform config directory"))?;
        Ok(dirs.config_dir().join("campus-network"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        Self::load_from_path(&path)
    }

    pub fn load_required() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            bail!(
                "config not found at {}; run without arguments to open the setup wizard",
                path.display()
            );
        }
        Self::load_from_path(&path)
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&path, contents)
            .with_context(|| format!("failed to write config file {}", path.display()))
    }

    pub fn validate(&self) -> Result<()> {
        if self.auth.username.trim().is_empty() {
            bail!("username must not be empty");
        }
        if self.auth.password.is_empty() {
            bail!("password must not be empty");
        }
        validate_url(&self.auth.portal_url, "portal_url")?;
        validate_url(&self.detect.probe_url, "probe_url")?;
        if self.detect.request_timeout_secs == 0 {
            bail!("request_timeout_secs must be greater than zero");
        }
        if self.daemon.online_check_interval_secs == 0 {
            bail!("online_check_interval_secs must be greater than zero");
        }
        if self.daemon.offline_check_interval_secs == 0 {
            bail!("offline_check_interval_secs must be greater than zero");
        }
        for cidr in &self.campus.ipv4_cidrs {
            cidr.parse::<ipnet::Ipv4Net>()
                .with_context(|| format!("invalid campus IPv4 CIDR: {cidr}"))?;
        }
        if self.campus.gateway_hosts.is_empty() {
            bail!("campus.gateway_hosts must not be empty");
        }
        for gateway in &self.campus.gateway_hosts {
            if gateway.trim().is_empty() {
                bail!("campus.gateway_hosts must not contain empty values");
            }
        }
        Ok(())
    }

    pub fn redacted_toml(&self) -> Result<String> {
        let mut clone = self.clone();
        if !clone.auth.password.is_empty() {
            clone.auth.password = "***redacted***".to_owned();
        }
        toml::to_string_pretty(&clone).context("failed to serialize config")
    }

    fn load_from_path(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }
}

fn validate_url(value: &str, field_name: &str) -> Result<()> {
    if reqwest::Url::parse(value).is_err() {
        bail!("{field_name} must be a valid absolute URL");
    }
    Ok(())
}
