import asyncio
from websockets.asyncio.client import connect
import json

# While `scores-ws` is already running...

async def run():
    # Create the websocket stream
    async with connect("ws://127.0.0.1:7727") as websocket:
        # Send the initial message within 5 seconds of connecting the websocket.
        # Must be either "connect" or a score id to resume from
        await websocket.send("connect")

        # Let's run it for a bit until we disconnect manually
        listening = asyncio.create_task(process_scores(websocket))
        await asyncio.sleep(10)
        listening.cancel()
        try:
            await listening
        except asyncio.CancelledError:
            pass

        # Let the websocket know we are about to disconnect.
        # This is not necessary but will allow us to resume later on.
        await websocket.send("disconnect")

        # As response, we'll receive a score id
        score_id = await websocket.recv()
        await websocket.close()

        await asyncio.sleep(10)

        # If we connect again later on...
        async with connect("ws://127.0.0.1:7727") as websocket:
            # ... we can use that score id. This way we only receive the scores that
            # were fetched in the meanwhile that we don't already know about.
            await websocket.send(score_id)
            await process_scores(websocket)

async def process_scores(websocket):
    try:
        async for event in websocket:
            # `event` consists of JSON bytes of a score as sent by the osu!api
            score = json.loads(event)
            print(f"{score['user_id']} got {score['pp']}pp on {score['beatmap_id']}")
    except asyncio.CancelledError:
        raise

if __name__ == "__main__":
    asyncio.run(run())
