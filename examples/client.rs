use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{protocol::Message, Error};

#[tokio::main]
async fn main() {
    // While `scores-ws` is already running...

    let url = "ws://127.0.0.1:7727";

    // Create the websocket stream
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("Failed to connect");

    let (mut write, mut read) = ws_stream.split();

    // Send the initial message within 5 seconds of connecting the websocket.
    // Must be either "connect" or a score id to resume from
    write.send(Message::from("connect")).await.unwrap();

    // Let's run it for a bit until we disconnect manually
    tokio::select! {
        _ = process_scores(&mut read) => {},
        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
    }

    // Let the websocket know we are about to disconnect
    write.send(Message::from("disconnect")).await.unwrap();

    // As response, we'll receive a score id
    let Some(Ok(Message::Text(data))) = read.next().await else {
        panic!()
    };

    let score_id: u64 = data.parse().unwrap();

    tokio::time::sleep(Duration::from_secs(10)).await;

    // If we connect again later on...
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("Failed to connect");

    let (mut write, mut read) = ws_stream.split();

    // ... we can use that score id. This way we only receive the scores that
    // were fetched in the meanwhile that we don't already know about.
    write
        .send(Message::from(score_id.to_string()))
        .await
        .unwrap();

    process_scores(&mut read).await;
}

async fn process_scores<S: StreamExt<Item = Result<Message, Error>> + Unpin>(stream: &mut S) {
    while let Some(res) = stream.next().await {
        let Ok(Message::Text(data)) = res else {
            panic!()
        };

        // `data` is a JSON string of a score as sent by the osu!api
        println!("{data}");
    }
}
