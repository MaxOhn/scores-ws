// While `scores-ws` is already running...

const WebSocket = require("ws");

// Create the websocket stream
const socket = new WebSocket("ws://127.0.0.1:7727");

// Send the initial message within 5 seconds of connecting the websocket.
// Must be either "connect" or a score id to resume from
socket.on("open", _ => socket.send("connect"));
socket.on("message", onMessage);

function onMessage(event) {
    // `event` may consist of three different things:
    //   - JSON bytes of score data as sent by the osu!api
    //   - a plain error message after 5 seconds of not sending the initial message
    //   - a score id after sending "disconnect" into the websocket
    const parsed = JSON.parse(event);

    if (isNaN(parsed)) {
        console.log(`${parsed.user_id} got ${parsed.pp}pp on ${parsed.beatmap_id}`);
    } else {
        reconnect(parsed);
    }
}

// Let's run it for a bit until we disconnect manually
setTimeout(_ => {
    // Let the websocket know we are about to disconnect.
    socket.send("disconnect");
}, 10_000);

function reconnect(scoreId) {
    // If we connect again later on...
    setTimeout(_ => {
        const socket = new WebSocket("ws://127.0.0.1:7727");

        // ... we can use that score id. This way we only receive the scores that
        // were fetched in the meanwhile that we don't already know about.
        socket.on("open", _ => socket.send(scoreId));
        socket.on("message", onMessage);
    }, 10_000);
}