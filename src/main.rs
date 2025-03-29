//! Fetches all osu! scores from the api and sends them through websockets.
//!
//! ## Usage
//!
//! 1. Download the [latest release]
//!
//! 2. Input your client id and secret for the osu!api in `config.toml` and modify
//!    the rest of the config to your liking.
//!
//! 3. Run `scores-ws`
//!
//! 4. Connect to `scores-ws` via websocket at `ws://{ip addr of your config}:{port of your config}`
//!    and listen for scores. Check out the [examples] folder for some examples.
//!
//! ## How it works
//!
//! `scores-ws` uses your osu!api client id & secret to fetch from the [scores endpoint].
//! Cursor management, score deduplication, rate limiting, and everything else is
//! handled automatically!
//!
//! To connect to the websocket, `scores-ws` requires you to send an initial message.
//! That message may either contain
//! - the string `"connect"` in which case it'll start off sending you all scores it
//!   has fetched so far (in its history).
//! - a score id in which case it'll send you all scores from that score id onwards.
//!
//! At any point you can send the string `"disconnect"` to the websocket. This will
//! make the websocket respond with a score id and close the connection. This score
//! id may be used later on to resume from with a new websocket connection.
//! Alternatively, you can just use a score id from a score you recently received
//! from the websocket and ignore this disconnect-message hassle.
//!
//! Since `scores-ws` runs separately, it allows you to have downtime on your actual
//! app without missing any scores; at least assuming there won't be more scores than the
//! configured history length during the downtime.
//!
//! [latest release]: https://github.com/MaxOhn/scores-ws/releases/latest
//! [examples]: https://github.com/MaxOhn/scores-ws/tree/main/examples
//! [scores endpoint]: https://osu.ppy.sh/docs/index.html#scores

#![warn(clippy::pedantic, clippy::missing_const_for_fn)]

#[macro_use]
extern crate eyre;

#[macro_use]
extern crate tracing;

use std::{net::SocketAddr, sync::Arc};

use eyre::{Context as _, Result};
use osu::Osu;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::{config::Config, context::Context};

mod config;
mod context;
mod event;
mod osu;

#[tokio::main]
async fn main() -> Result<()> {
    let Config { setup, osu } = Config::parse();

    let filter = EnvFilter::new(format!("scores_ws={},off", setup.log));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let osu = Osu::new(osu).context("Failed to create osu! client")?;
    let ctx = Arc::new(Context::new(&setup));

    let addr = SocketAddr::new(setup.ip_addr, setup.port);
    let listener = TcpListener::bind(addr).await.unwrap();
    info!("Listening on {addr}...");

    tokio::spawn(Context::fetch_scores(
        Arc::clone(&ctx),
        osu,
        setup.interval,
        setup.resume_score_id,
    ));

    while let Ok(conn) = listener.accept().await {
        tokio::spawn(Context::handle_connection(Arc::clone(&ctx), conn));
    }

    Ok(())
}
