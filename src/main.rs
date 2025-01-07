//! Fetches all osu! scores from the api and sends them through websockets.
//!
//! TODO: more docs

#[macro_use]
extern crate eyre;

#[macro_use]
extern crate tracing;

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

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

    let filter = EnvFilter::new(format!("scores_ws={},info", setup.log));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let osu = Osu::new(osu, setup.resume_score_id).context("Failed to create osu! client")?;
    let ctx = Arc::new(Context::new(&setup));

    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, setup.port);
    let listener = TcpListener::bind(addr).await.unwrap();
    info!("Listening on {addr}...");

    tokio::spawn(Context::fetch_scores(Arc::clone(&ctx), osu, setup.interval));

    while let Ok(conn) = listener.accept().await {
        tokio::spawn(Context::handle_connection(Arc::clone(&ctx), conn));
    }

    Ok(())
}
