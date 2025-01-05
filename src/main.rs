#[macro_use]
extern crate eyre;

#[macro_use]
extern crate tracing;

use std::{
    collections::VecDeque,
    fmt::{Display, Formatter, Result as FmtResult},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::Duration,
};

use config::ConfigToml;
use eyre::Result;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use fxhash::FxHashSet;
use osu::{ScoreId, Scores};
use papaya::HashMap;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_tungstenite::tungstenite::Message;

use crate::{config::Config, osu::Osu};

mod config;
mod osu;

struct Context {
    config: Config,
    osu: Osu,
    clients: HashMap<SocketAddr, mpsc::UnboundedSender<Message>>,
    history: Mutex<VecDeque<Scores>>,
}

impl Context {
    fn new() -> Result<Self> {
        let ConfigToml { config, osu } = ConfigToml::parse()?;

        Ok(Self {
            history: Mutex::new(VecDeque::with_capacity(config.history_length)),
            osu: Osu::new(osu),
            clients: HashMap::new(),
            config,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let ctx = Arc::new(Context::new()?);

    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, ctx.config.port);
    let listener = TcpListener::bind(addr).await.unwrap();
    info!("Listening on: {addr}...");

    tokio::spawn(fetch_scores(Arc::clone(&ctx)));

    while let Ok(conn) = listener.accept().await {
        tokio::spawn(handle_connection(Arc::clone(&ctx), conn));
    }

    Ok(())
}

async fn fetch_scores(ctx: Arc<Context>) {
    let Context {
        config,
        osu,
        clients,
        history,
    } = &*ctx;

    info!("Fetching scores every {} seconds", config.interval);

    let mut interval = tokio::time::interval(Duration::from_secs(config.interval));

    loop {
        interval.tick().await;

        let scores = osu.fetch_scores().await.unwrap();
        let pin = clients.pin();

        for (addr, tx) in pin.iter() {
            trace!("Sending to {addr}...");

            for score in scores.iter() {
                let _: Result<_, _> = tx.send(Message::Text(score.as_str().into()));
            }
        }

        trace!("Sent to all clients");

        {
            let mut history = history.lock().unwrap();

            history.push_back(scores);

            if history.len() > config.history_length {
                history.pop_front();
            }
        }
    }
}

async fn handle_connection(ctx: Arc<Context>, (stream, addr): (TcpStream, SocketAddr)) {
    info!("Incoming TCP connection from: {addr}");

    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Error during the websocket handshake occurred");

    info!("WebSocket connection established: {addr}");

    let (tx, mut rx) = mpsc::unbounded_channel();
    ctx.clients.pin().insert(addr, tx.clone());

    let (mut outgoing, mut incoming) = ws_stream.split();

    let initial_fut = tokio::time::timeout(Duration::from_secs(5), incoming.next());

    let Ok(initial) = initial_fut.await else {
        let err = "Require initial message containing either `\"connect\"` \
            or a score id to resume from";
        let _ = outgoing.send(Message::Text(err.into())).await;

        return;
    };

    let resume_id = match initial.map(|res| res.map(Event::try_from)) {
        Some(Ok(Ok(Event::Connect))) => {
            info!("Fresh connect");

            None
        }
        Some(Ok(Ok(Event::Resume(ScoreId(id))))) => {
            info!("Resume on {id}");

            Some(id)
        }
        Some(Ok(Err(err))) => {
            let _ = outgoing.send(Message::Text(err.to_string().into())).await;

            return;
        }
        Some(Err(err)) => {
            error!("Failed to receive initial message: {err}");

            return;
        }
        None => return,
    };

    {
        let history = ctx.history.lock().unwrap();
        let mut sent = FxHashSet::default();

        // FIXME: we reverse to send everything until we reach the
        // resume id which means the scores after the resume id will
        // be sent in reverse order which is not deseriable. perhaps
        // read from front-to-back instead and skip until the resume
        // id is found. this has the downside that we'll have to
        // iterate over everything twice if the resume id is not
        // present at all.
        'outer: for scores in history.iter().rev() {
            for score in scores.iter() {
                let Some(ScoreId(id)) = score.id else {
                    continue;
                };

                if Some(id) == resume_id {
                    break 'outer;
                } else if sent.insert(id) {
                    let text = score.as_str().into();
                    tx.send(Message::Text(text)).unwrap();
                } else {
                    continue 'outer;
                }
            }
        }
    }

    let forward_fut = futures_util::stream::poll_fn(|cx| rx.poll_recv(cx))
        .map(Ok)
        .forward(&mut outgoing);

    let await_disconnect = incoming.try_any(|msg| {
        let bytes = match msg {
            Message::Text(ref bytes) => bytes.as_bytes(),
            Message::Binary(ref bytes) => bytes,
            _ => return futures_util::future::ready(false),
        };

        futures_util::future::ready(bytes == b"disconnect")
    });

    tokio::select! {
        _ = forward_fut => {},
        res = await_disconnect => {
            if matches!(res, Ok(true)) {
                info!("Processing disconnect...");

                let id = ctx
                    .history
                    .lock()
                    .unwrap()
                    .back()
                    .and_then(|scores| scores.last())
                    .and_then(|score| score.id)
                    .map_or(0, |ScoreId(id)| id);

                let msg = Message::Text(id.to_string().into());

                if let Err(err) = outgoing.send(msg).await {
                    warn!("Failed to send score id {id} on disconnect: {err}");
                }
            }
        },
    }

    info!("{addr} disconnected");
    ctx.clients.pin().remove(&addr);
}

enum Event {
    Connect,
    Resume(ScoreId),
}

impl TryFrom<Message> for Event {
    type Error = EventError;

    fn try_from(msg: Message) -> Result<Self, Self::Error> {
        let bytes: &[u8] = match msg {
            Message::Text(ref bytes) => bytes.as_bytes(),
            Message::Binary(ref bytes) => bytes,
            _ => return Err(EventError::Variant),
        };

        if bytes == b"connect" {
            Ok(Self::Connect)
        } else if let Some(id) = ScoreId::try_parse(bytes) {
            Ok(Self::Resume(id))
        } else {
            Err(EventError::Bytes)
        }
    }
}

#[derive(Debug)]
enum EventError {
    Bytes,
    Variant,
}

impl Display for EventError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            EventError::Bytes => {
                f.write_str("message must be either `\"connect\"` \r a score id to resume from")
            }
            EventError::Variant => f.write_str("message must contain text data"),
        }
    }
}
