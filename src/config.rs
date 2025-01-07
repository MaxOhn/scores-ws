use std::{fs::File, io::Read};

use eyre::Context;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub setup: Setup,
    pub osu: OsuConfig,
}

impl Config {
    pub fn parse() -> Self {
        let mut file = File::open("./config.toml").unwrap_or_else(|_| {
            panic!("Be sure a file `config.toml` is in the same directory as this binary")
        });

        let mut content = String::new();

        file.read_to_string(&mut content)
            .context("Failed to read file `config.toml`")
            .unwrap();

        toml::from_str(&content)
            .context("Failed to deserialize file `config.toml`")
            .unwrap()
    }
}

#[derive(Deserialize)]
pub struct Setup {
    #[serde(default = "Setup::default_log")]
    pub log: Box<str>,
    #[serde(default = "Setup::default_port")]
    pub port: u16,
    #[serde(default = "Setup::default_interval")]
    pub interval: u64,
    #[serde(default = "Setup::default_history_length")]
    pub history_length: usize,
    pub resume_score_id: Option<u64>,
}

#[derive(Deserialize)]
pub struct OsuConfig {
    pub client_id: u64,
    pub client_secret: Box<str>,
    pub ruleset: Option<Box<str>>,
}

impl Setup {
    fn default_log() -> Box<str> {
        Box::from("info")
    }

    fn default_port() -> u16 {
        7277
    }

    fn default_interval() -> u64 {
        30
    }

    fn default_history_length() -> usize {
        100
    }
}
