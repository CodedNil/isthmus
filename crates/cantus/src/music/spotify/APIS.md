# Spotify APIs used by Cantus

## JSON HTTP requests

| Method | Route | Request and response | Cantus usage |
| --- | --- | --- | --- |
| GET | `https://apresolve.spotify.com/?type=dealer&type=spclient` | JSON response | Provides the Dealer and spclient hosts. Cantus retries discovery if it cannot connect. |
| POST | `https://accounts.spotify.com/api/token` | URL-encoded form → JSON | Exchanges the browser authorization code. Later launches use the stored refresh token. |
| GET | `/audio-attributes/v1/audio-features?format=json&ids={ids}` | JSON response | Fetches audio features for a comma-separated batch of track IDs. |
| POST | `/connect-state/v1/player/command/from/{device_id}/to/{target}` | Gzip-compressed JSON request with an `X-Spotify-Connection-Id` header. Response body ignored. | Sends one playback command at a time over HTTP. Dealer pushes can update playback state while the request is in flight. |

## Protobuf HTTP requests

| Method | Route | Request and response | Cantus usage |
| --- | --- | --- | --- |
| GET | `/color-lyrics/v3/track/{id}?prototype=true` | `LyricsWrapperResponse` | Fetches line and fragment timing using the `Linux` app-platform header. |
| POST | `/collection/v2/write` | `WriteRequest`. Response body ignored. | Adds or removes a liked song. Uses the Dealer connection ID and the `application/vnd.collection-v2.spotify.proto` media type. |
| PUT | `/connect-state/v1/devices/{device_id}` | `PutStateRequest` → `Cluster` | Publishes Cantus device state and reads the current Connect cluster. Uses the Dealer connection ID. |
| POST | `/extended-metadata/v0/extended-metadata` | `BatchedEntityRequest` → `BatchedExtensionResponse` | Fetches batched track metadata and decodes the `TRACK_V4` extension. |
| POST | `/playlist/spotify.list.v1.ListReadService/GetLists` | `GetListsRequest` → `GetListsResponse` | Starts loading the root list alongside Connect registration, then batches reads for playlists with missing or changed revisions. |
| POST | `/playlist/v2/playlist/{id}/changes` | `ListChanges`. Response body ignored. | Batches ordered edits per playlist and sends writes to different playlists concurrently. Uses the Dealer connection ID. |

## Other HTTP requests

| Method | Route | Cantus usage |
| --- | --- | --- |
| GET | `https://accounts.spotify.com/authorize` | Opens browser sign-in with PKCE when no usable refresh token is available. Spotify redirects the browser to Cantus's loopback callback. |
| GET | `https://i.scdn.co/image/{id}` | Downloads artwork as image bytes. Cantus constructs this CDN URL from Spotify image IDs and can also fetch artwork URLs supplied in metadata. |

## WebSocket API usage

The `hm://` entries are incoming Dealer message topics. The client matches each topic exactly or as a slash-delimited prefix and ignores any query string. Cantus requires gzip-compressed protobuf payloads wrapped in base64.

| Route or topic | Direction | Cantus usage |
| --- | --- | --- |
| `wss://{dealer_host}:{port}/?access_token={token}` | Bidirectional | Opens the Dealer socket using the host returned by AP Resolve. Sends JSON ping messages, receives pong messages, and replies to incoming request envelopes with `success: false`. |
| `hm://pusher/v1/connections` | Incoming | Reads the `Spotify-Connection-Id` header for Connect state, playback commands, and library writes. |
| `hm://connect-state/v1/cluster` | Incoming | Decodes a protobuf `ClusterUpdate` and updates playback and device state. |
| `hm://playlist/v2` | Incoming | Decodes protobuf playlist notifications to refresh changed playlists or the root list. |
| `hm://playlist/user` | Incoming | Reads the new root-list revision from the first protobuf field. |
