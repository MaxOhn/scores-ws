use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use eyre::Result;
use futures_util::{stream::SplitSink, SinkExt, StreamExt, TryStreamExt};
use papaya::HashMap;
use tokio::{net::TcpStream, sync::mpsc};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

use crate::{
    config::Setup,
    event::Event,
    osu::{FetchResult, Osu, Score, Scores},
};

type Sender = mpsc::UnboundedSender<Message>;
type Outgoing = SplitSink<WebSocketStream<TcpStream>, Message>;

const SECOND: Duration = Duration::from_secs(1);

pub struct Context {
    clients: HashMap<SocketAddr, Sender>,
    history: Mutex<Scores>,
    max_history_len: usize,
}

impl Context {
    pub fn new(setup: &Setup) -> Self {
        Self {
            history: Mutex::new(Scores::new()),
            clients: HashMap::new(),
            max_history_len: setup.history_length,
        }
    }

    pub async fn fetch_scores(ctx: Arc<Self>, osu: Osu, interval: u64, mut cursor_id: Option<u64>) {
        let Context {
            clients,
            history,
            max_history_len,
        } = &*ctx;

        info!("Fetching scores every {interval} seconds...");

        let mut interval = tokio::time::interval(Duration::from_secs(interval));
        let mut scores = Scores::new();

        loop {
            interval.tick().await;

            let prev_cursor_id = cursor_id;

            if let FetchResult::CursorTooOld = osu.fetch_scores(&mut scores, cursor_id).await {
                if cursor_id.take().is_none() {
                    // This should never happen; bug in osu! api
                    error!("\"cursor too old\" but no cursor specified");

                    continue;
                }

                tokio::time::sleep(SECOND).await;

                if let FetchResult::CursorTooOld = osu.fetch_scores(&mut scores, cursor_id).await {
                    // We took the cursor id out previously so this is the same case as above
                    error!("\"cursor too old\" but no cursor specified");

                    continue;
                }
            }

            loop {
                const ID_THRESHOLD: u64 = 900;

                let next_cursor_id = scores.last().map(Score::id);
                debug!(?next_cursor_id);

                let Some(next_cursor_id) = next_cursor_id else {
                    cursor_id = None;

                    break;
                };

                if cursor_id
                    .replace(next_cursor_id)
                    .is_none_or(|prev_cursor_id| next_cursor_id < prev_cursor_id + ID_THRESHOLD)
                {
                    // If either `cursor_id` was `None` or we did not receive
                    // at least `ID_THRESHOLD` many new scores*, we stop
                    // fetching more scores.
                    //
                    // In other words: our `ID_THRESHOLD` needs to be large
                    // enough so that within our sleep interval (1 second),
                    // it's very unlikely that the difference to the next score
                    // id will be greater than our threshold. Additionally,
                    // the threshold may not be larger than the maximum amount
                    // of scores sent by the endpoint which is 1000.
                    //
                    // *: We don't really count new scores but consider the
                    // difference in score id instead which should be roughly
                    // proportional to score count.
                    break;
                }

                tokio::time::sleep(SECOND).await;

                if let FetchResult::CursorTooOld = osu.fetch_scores(&mut scores, cursor_id).await {
                    // This should never happen
                    error!("The newly fetched cursor id {next_cursor_id} was too old");

                    break;
                }
            }

            let range = scores.range(Score::only_id(prev_cursor_id.map_or(0, |id| id + 1))..);

            let pin = clients.pin();
            let mut sent = 0;

            for score in range {
                sent += 1;

                for tx in pin.values() {
                    let _: Result<_, _> = tx.send(score.as_message());
                }
            }

            trace!("Sent {sent} scores to {} client(s)", clients.len());

            let mut history = history.lock().unwrap();
            history.append(&mut scores);

            while history.len() > *max_history_len {
                history.pop_first();
            }

            debug!(history_len = history.len());
        }
    }

    pub async fn handle_connection(ctx: Arc<Self>, (stream, addr): (TcpStream, SocketAddr)) {
        trace!(%addr, "Incoming TCP connection from");

        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(stream) => stream,
            Err(err) => return error!(?err, "Error during the websocket handshake"),
        };

        trace!(%addr, "WebSocket connection established");

        let (tx, mut rx) = mpsc::unbounded_channel();
        ctx.clients.pin().insert(addr, tx.clone());

        let (mut outgoing, mut incoming) = ws_stream.split();

        let initial_fut = tokio::time::timeout(Duration::from_secs(5), incoming.next());

        let Ok(initial) = initial_fut.await else {
            let err = "Require initial message containing either `\"connect\"` \
                or a score id to resume from";
            let _: Result<_, _> = outgoing.send(Message::Text(err.into())).await;
            info!("Disconnecting from {addr} due to missing initial message");

            return;
        };

        let resume_id = match initial.map(|res| res.map(Event::try_from)) {
            Some(Ok(Ok(Event::Connect))) => {
                info!(%addr, "Connect");

                None
            }
            Some(Ok(Ok(Event::Resume { score_id }))) => {
                info!(score_id, %addr, "Resume");

                Some(score_id)
            }
            Some(Ok(Err(err))) => {
                let _: Result<_, _> = outgoing.send(Message::Text(err.to_string().into())).await;

                return;
            }
            Some(Err(err)) => return error!(?err, "Failed to receive initial message"),
            None => return,
        };

        ctx.send_history(resume_id, addr, tx);

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
                    ctx.process_disconnect(&mut outgoing).await;
                }
            },
        }

        info!("{addr} disconnected");
        ctx.clients.pin().remove(&addr);
    }

    fn send_history(&self, resume_id: Option<u64>, addr: SocketAddr, tx: Sender) {
        let range = Score::only_id(resume_id.map_or(0, |id| id + 1))..;
        let mut sent = 0;

        for score in self.history.lock().unwrap().range(range) {
            sent += 1;
            let _: Result<_, _> = tx.send(score.as_message());
        }

        info!(%addr, "Sent {sent} scores from the history");
    }

    async fn process_disconnect(&self, outgoing: &mut Outgoing) {
        info!("Processing disconnect...");

        let id = self.history.lock().unwrap().last().map_or(0, Score::id);
        let msg = Message::Text(itoa::Buffer::new().format(id).into());

        if let Err(err) = outgoing.send(msg).await {
            warn!(?err, "Failed to send score id {id} on disconnect");
        }
    }
}
