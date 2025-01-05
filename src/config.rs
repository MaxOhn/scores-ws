use std::{fs::File, io::Read};

use eyre::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ConfigToml {
    pub config: Config,
    pub osu: OsuConfig,
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "Config::default_interval")]
    pub interval: u64,
    #[serde(default = "Config::default_history_length")]
    pub history_length: usize,
    #[serde(default = "Config::default_port")]
    pub port: u16,
}

#[derive(Deserialize)]
pub struct OsuConfig {
    pub client_id: u64,
    pub client_secret: Box<str>,
}

impl ConfigToml {
    pub fn parse() -> Result<Self> {
        let mut file = File::open("./config.toml").map_err(|_| {
            eyre!("Be sure a file `config.toml` is in the same directory as this binary")
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .context("Failed to read file `config.toml`")?;

        toml::from_str(&content).context("Failed to deserialize file `config.toml`")
    }
}

impl Config {
    fn default_interval() -> u64 {
        30
    }

    fn default_history_length() -> usize {
        100
    }

    fn default_port() -> u16 {
        7277
    }
}
