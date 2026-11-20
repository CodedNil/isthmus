# Spotify APIs used by Cantus

Linux uses the APIs below. The browser demo uses bundled data. [Desktop inventory and probe results](API_INVENTORY.md)

## Sign-in and discovery

| Request | Format | Use |
|---|---|---|
| `GET https://accounts.spotify.com/authorize` → `GET http://127.0.0.1:8898/login` | Browser OAuth with PKCE and state | First sign-in or rejected refresh token. Scopes are `streaming app-remote-control`. |
| `POST https://accounts.spotify.com/api/token` | Form → JSON | Exchange the browser code. Renew at session start or when an access token expires. |
| `GET https://apresolve.spotify.com/?type=dealer&type=spclient` | JSON | Find and cache regional hosts. Anonymous. Fixed hosts are available if discovery fails. |

`spotify-oauth.json` stores the username and refresh token. Refresh-token updates are atomic. Access tokens stay in memory.

## Playback, playlists, and media

Relative paths use the discovered HTTP host. **Bearer** means OAuth. **Connection ID** means the Dealer announcement ID sent as `x-spotify-connection-id`.

| Request | Format | Auth | Use |
|---|---|---|---|
| `wss://{dealer-host}/?access_token=…` | JSON envelopes with protobuf or protobuf JSON cluster payloads. Binary payloads can be gzipped. | OAuth in URL | Live playback and playlist updates. |
| `PUT /connect-state/v1/devices/{device-id}` | Protobuf | Bearer + connection ID | Register the observer and get playback state on connect or reconnect. |
| `POST /connect-state/v1/player/command/from/{source}/to/{target}` | Gzipped JSON request | Bearer + connection ID | Play, pause, seek, skip. Scrubbing commits on release. A queue jump sends the target URI and UID. One wheel notch sends a relative skip. |
| `POST /playlist/spotify.list.v1.ListReadService/GetLists` | Protobuf | Bearer | Read the root and selected lists on connect or when updates require snapshots. Selected lists are batched. No pagination. |
| `POST /playlist/v2/playlist/{id}/changes` | Protobuf | Bearer + connection ID | Write a playlist edit. Apply a clean receipt locally, or batch a read of the edited lists and their artwork if Spotify asks for resync. |
| `POST /collection/v2/write` | Protobuf request | Bearer + connection ID | Ratings of five or higher save a track. Lower ratings remove it. |
| `POST /extended-metadata/v0/extended-metadata` | Protobuf, `TRACK_V4` | Bearer | One batch of distinct uncached queue tracks. Per-track 404s are cached until the track leaves the queue. |
| `GET /audio-attributes/v1/audio-features?format=json&ids=…` | JSON | Bearer | Up to 240 uncached IDs per request for visible and nearby tracks. Errors can retry after 30 seconds. Omitted rows stay unavailable until the track leaves the queue. |
| `GET /color-lyrics/v2/track/{id}?format=json&vocalRemoval=false&market=from_token` | JSON | Bearer | Fetch lyrics near the playhead when enabled. Musixmatch is a separate provider and receives the Spotify URI when available. |
| `GET https://i.scdn.co/image/{id}` or the URL from metadata | Image bytes | Anonymous | Fetch uncached covers for visible and nearby tracks and playlist buttons. Tiles show placeholders while covers load. |

Liked Songs plays `spotify:user:{username}:collection`. Cantus gets rating markers from configured playlists and sends no saved-track membership reads.

## Dealer and retries

| Message | Handling |
|---|---|
| `hm://pusher/v1/connections` | Read the connection ID from the announcement header. |
| `hm://connect-state/v1/cluster` | Update playback and queue. |
| `hm://playlist/v2`, `hm://playlist/user` | Apply supported edits against the matching parent revision. When idle, skip known revisions and unselected track-only edits, and read just the selected list for a gap. Overlapping, root, and unknown changes share a root refresh. |
| JSON heartbeat and WebSocket control ping | Send JSON ping every 30 seconds and expect pong within three seconds. Answer control pings. |
| Dealer `request` | Send a keyed reply with `success: false` for unsupported requests. |

Each socket has 30 seconds to announce its connection ID. Disconnect clears the old ID. Failed Dealer reconnects wait 10 seconds. A rejected refresh token returns to browser sign-in. Playback uses push between registrations.

Playlist refreshes run one at a time. New notifications can schedule one follow-up. Revisions identify which lists need a read. Local writes cancel stale reads first.

Artwork and audio features share the same track selection. Enrichment runs when visible tracks change, on playlist or playback updates, and about once a second for retries. Queue metadata supplies the durations needed for layout.

HTTP requests use an eight-second timeout and ask for gzip responses. Protobuf requests are plain. GET and PUT can try up to three backends on connection errors or timeouts. A 429 can retry once if Spotify gives a numeric `Retry-After` of at most ten seconds. The Spotify and OAuth clients disable redirects.
