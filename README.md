# scores-ws

<!-- cargo-rdme start -->

Fetches all osu! scores from the api and sends them through websockets.

### Usage

1. Download the [latest release]

2. Input your client id and secret for the osu!api in `config.toml` and modify
the rest of the config to your liking.

3. Run `scores-ws`

4. Connect to `scores-ws` via websocket at `ws://127.0.0.1:{port of your config}`
and listen for scores. Check out the [examples] folder for some examples.

### How it works

`scores-ws` uses your osu!api client id & secret to fetch from the [scores endpoint].
Cursor management, score deduplication, rate limiting, and everything else is
handled automatically!

To connect to the websocket, `scores-ws` requires you to send an initial message.
That message may either contain
- the string `"connect"` in which case it'll start off sending you all scores it
has fetched so far (in its history).
- a score id in which case it'll send you all scores from that score id onwards.

At any point you can send the string `"disconnect"` to the websocket. This will
make the websocket respond with a score id and close the connection. This score
id may be used later on to resume from with a new websocket connection.

[latest release]: https://github.com/MaxOhn/scores-ws/releases/latest
[examples]: https://github.com/MaxOhn/scores-ws/tree/main/examples
[scores endpoint]: https://osu.ppy.sh/docs/index.html#scores

<!-- cargo-rdme end -->
