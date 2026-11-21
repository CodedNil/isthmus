# Spotify APIs used by Cantus

## Connections

| Endpoint | Transport | Purpose |
|---|---|---|
| `GET accounts.spotify.com/authorize` | Browser HTTPS | OAuth authorization with PKCE |
| `GET 127.0.0.1:8898/login` | Loopback HTTP/1.1 | OAuth callback |
| `POST accounts.spotify.com/api/token` | HTTP/3 | Exchange and refresh tokens |
| `GET apresolve.spotify.com/?type=dealer&type=spclient` | HTTP/3 | Discover Dealer and API hosts |
| `wss://{dealer-host}:{port}/?access_token=…` | TLS WebSocket | Receive Dealer events |

## Requests

All use HTTP/3. Relative paths use the discovered `spclient` host.

| Endpoint | Purpose |
|---|---|
| `PUT /connect-state/v1/devices/{device-id}` | Register Cantus as a playback observer and read the initial state |
| `POST /connect-state/v1/player/command/from/{source}/to/{target}` | Control playback and jump within the queue |
| `POST /playlist/spotify.list.v1.ListReadService/GetLists` | Read the root list and playlist snapshots |
| `POST /playlist/v2/playlist/{id}/changes` | Edit playlist membership, including Cantus rating playlists |
| `POST /collection/v2/write` | Add or remove tracks from Liked Songs |
| `POST /extended-metadata/v0/extended-metadata` | Read track metadata (`TRACK_V4`) |
| `GET /audio-attributes/v1/audio-features?format=json&ids=…` | Read track audio features |
| `GET /color-lyrics/v2/track/{id}?format=json&vocalRemoval=false&market=from_token` | Read timed lyrics |
| `GET image-cdn-fa.spotifycdn.com/image/{id}` or supplied artwork URL | Fetch artwork |

## Dealer topics

| Topic | Purpose |
|---|---|
| `hm://pusher/v1/connections` | Announce the connection ID |
| `hm://connect-state/v1/cluster` | Report playback and queue changes |
| `hm://playlist/v2`, `hm://playlist/user` | Report playlist and root-list changes |
