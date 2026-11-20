# Spotify Desktop API inventory

Linux build **1.2.95.453.g0eeebbed**. Live results cover the tested account and samples. Desktop evidence comes from the stock native binary, login and XPUI bundles, and installed code excluding extensions. [Cantus APIs](APIS.md) · [Desktop index](#static-index)

## Accounts and authorization

| Endpoint or check | Live result |
|---|---|
| `GET accounts.spotify.com/authorize` and `POST accounts.spotify.com/api/token` | Browser returned a localhost code. Form exchange returned username, access token, refresh token, and expiry. Refresh returned an access token and omitted username. |
| `POST login5.spotify.com/v4/login` | Protobuf accepted a stored credential. Its access token worked for audio features. |
| `GET api.spotify.com/v1/me`, `GET accounts.spotify.com/oauth2/userinfo` | HTTP 429 and 404, respectively. |
| `GET /user-profile-view/v3/profile/{username}` | JSON. The canonical username found the account. Literal `me` found a different profile. |
| `GET apresolve.spotify.com/?type=dealer&type=spclient` | Anonymous JSON with regional HTTP and Dealer hosts. |
| `POST clienttoken.spotify.com/v1/clienttoken` | Issued a token without an account bearer. Desktop protobuf defines device data and a hashcash challenge. |
| v0 metadata, `GetLists`, audio-feature batches | HTTP 401 anonymously or with ClientToken only. OAuth bearer worked. |
| Observer registration, pause, playlist add/remove, save/unsave | Worked with OAuth and Dealer connection ID. |
| `POST api.spotify.com/v1/me/playlists` | Private creation returned HTTP 429, including a fresh OAuth retry with `playlist-modify-private`. |

Anonymous track lookups returned HTTP 200. None exposed ISRC in the sample.

| `open.spotify.com` path | Data |
|---|---|
| `/oembed?url={track-url}` | JSON title, thumbnail and dimensions, embed markup. |
| `/embed/track/{id}` | HTML with title, artists, release date, duration, artwork. |
| `/track/{id}` | HTML metadata with title, image, duration, album, release date, musicians. |

## Track metadata, audio, and lyrics

| Endpoint | Result |
|---|---|
| `POST /extended-metadata/v0/extended-metadata` | Protobuf batch returned `TRACK_V4` with title, artists, duration, artwork, and ISRC. |
| `POST /extended-metadata/v3/extensions/batch-get` | v0 request and ProtoJSON returned empty HTTP 200. Desktop-local multi-request returned 400. Earlier layouts gave 500. GET gave 405. No track decoded. |
| `GET /metadata/4/track/{id}` | Protobuf. 32-digit hex GID worked. Base62 or comma-separated IDs gave 404. Guessed `POST /metadata/4/multi` also gave 404. |
| `GET api.spotify.com/v1/tracks?ids=…` | JSON API returned 429. |
| `GET /audio-attributes/v1/audio-features/{id}` | JSON even with `format=protobuf`. Includes BPM, energy, danceability, acousticness, valence, instrumentalness. |
| `GET /audio-attributes/v1/audio-features?format=json&ids=…` | JSON array with IDs and nulls for unavailable tracks. All distinct IDs returned at 100, 101, 160, 200, and 240. One 200-ID call and two parallel 100-ID calls each took 109 ms. 240 took 113 ms. |
| Audio-feature controls | Comma-separated IDs in the path gave 400. The public batch gave 429. `GET /audio-attributes/v1/audio-analysis/{id}` gave 404. |
| `GET /color-lyrics/v2/track/{id}` | Earlier 403s with or without ClientToken. Later two tracks returned 60 and 48 lines both ways. A third gave 404 both ways. Generic and regional hosts agreed with identical production headers. |
| `GET /color-lyrics/v2/track/{id}/image/{encoded-image-url}` | Desktop adds `format=json&vocalRemoval=false` and `App-Platform: WebPlayer`. Both the image and bare paths gave empty 403s in the earlier three-track sample. |

Native routine `0x17d6d60` passes the same body and `application/protobuf` headers to both metadata routes. Extensions 10, 142, 219, and 222 returned data through v0 and empty HTTP 200 through v3.

### Metadata extensions

v0 request fields. Batch entities (2), then URI (1) and queries (2). Each query has kind (1) and optional ETag (2).

An 83-track batch returned an ETag for every track. Repeating with matching tags returned 83 per-track 304 statuses and no track payloads. The gzipped response dropped from 33,926 B to 2,794 B. The request grew from 3,652 B to 5,146 B. Wrong tags returned all payloads again. The response header inside each entity carries status (1), ETag (2), and cache lifetime in seconds (4).

One nonexistent track returned a per-track 404 with no payload. On 103 tracks, gzip took 38,878 B and zstd took 38,297 B. Offering Brotli alongside gzip still returned gzip.

Across eight tracks, `IDENTITY_TRAIT` (178) returned 1,161 B of raw payloads with names, contributors, and parent references. Pairing it with `VISUAL_IDENTITY_TRAIT` (179, 7,590 B) exceeded `TRACK_V4` (6,066 B) and left out duration and ISRC. `COVER_IMAGE` (79) returned no payload. `TRACK_DESCRIPTOR` (6) returned descriptive tags. `PUBLISHING_METADATA_TRAIT` (183) returned publishing data and `ENTITY_TYPE_TRAIT` (220) returned the entity type.

| Kind | Result |
|---|---|
| 10 `TRACK_V4` | `spotify.metadata.Track` in `google.protobuf.Any`. |
| 142 `LIST_TUNER_AUDIO_ANALYSIS` | Start time, sample period, repeated loudness. |
| 219 `MIXABILITY` | `spotify.playlistmixing.extensions.mixability.Mixability`. |
| 222 `AUDIO_ATTRIBUTES_V2` | BPM (double field 1, 113.3 in sample). Field 2 describes key, mode, and Camelot value and color. |
| 61 `AUDIO_ATTRIBUTES`, 95 `TRACK_EXTRA_AUDIO_ATTRIBUTES`, 259 `TRACK_AUDIO_FOUNDATIONS` | HTTP 200 with no `Any` payload in the sample. |

## Playlists and saved tracks

`POST /playlist/spotify.list.v1.ListReadService/GetLists` accepted plain protobuf and returned one result per input in order. gRPC framing gave 400. Missing lists returned per-list errors. The legacy `GET /playlist/v2/playlist/{id}` and `/playlist/v2/user/{username}/rootlist` agreed on three track sets (50, 150, 150 items), root order (30 items), and 26 matched names, covers, and revisions. Root alias `@` gave 400.

| Field | Meaning |
|---|---|
| Request | Repeated requests (1). Each has identifier (1) and options (2). Identifier URI (1) accepts `spotify:playlist:{id}` or `spotify:user:{username}:rootlist`. |
| Projection | Options (4) → list response (2) → entity (1) → own attributes (1), items (2), or metadata (3). Item attributes (2), metadata attributes (1). Attribute enums: URI 1, name 3, picture file ID 20, picture sizes 21. |
| Pagination | Option 6 holds length (1) and offset (3). Offset 2 kept legacy order. Length 0 or an offset past the end returned no items. Lengths 100, 250, and 1,000 worked. Omitted, 10,000, and max int32 each returned all 1,351 items. Larger lists remain untested. |
| Response | Results (1), entity (1), info (2). Entity has attributes (1), items (2), and URI-keyed metadata (3). Info has error (1) and revision (7). One of six item snapshots had a different revision from the root metadata. |

- **Writes.** `POST /playlist/v2/playlist/{id}/changes` returned 359-byte receipts with revision (1), sync result (7), resulting revision (8), and resync flag (20). Revisions matched the committed snapshot. Adds set the resync flag and removals cleared it. One add left root metadata and revision unchanged. Its Dealer edit was complete, and a targeted `GetLists` returned the items and revision. Targeted cover URLs matched the root for six playlists.
- **Desktop-local writes.** `PlaylistService.Modify` takes URI (1) and Cosmos modification (2). `RootlistModificationService.Modify` handles the root. No newer remote write route was verified.
- **Saved tracks.** `GetLists` gave 404 for `spotify:user:{username}:collection` and `spotify:collection:tracks`. `POST /collection/v2/contains` returned membership booleans. Its request has username (1), set `collection` (2), and items with track URIs (3). Save and remove returned empty successes and membership went false → true → false. Local `CollectionPlatformService` exposes Add, Remove, Contains, and Get.

### DiffLists

`POST /playlist/spotify.list.v1.ListReadService/DiffLists` uses the `GetLists` projection and identifier revision (2). Response fields are entity (1), diff (2), and info (3).

| Base revision | Response body |
|---|---|
| Current playlist | Empty per-list result, 2 B. |
| Current root | Info with field 2, 8 B. |
| Missing, unknown, or before root deletion | Full root and current revision, 5,112 B. |
| Before adding a track | Full one-item entity, 74 B versus 133 B for projected `GetLists`. Omitting options gave an 81 B per-list error. |

The live addition and deletion returned snapshots with no delta operations.

## Dealer and playback

| Message or check | Evidence |
|---|---|
| `wss://{dealer-host}/?access_token=…` | HTTP 101. No WebSocket extensions negotiated. Payload gzip is separate. |
| JSON ping/pong, `hm://pusher/v1/connections` | Heartbeat answered. Announcement supplied connection ID. |
| `hm://connect-state/v1/cluster` | JSON envelopes in text or binary frames. Payload can be protobuf or protobuf JSON. |
| Dealer request/reply | Desktop replies to inbound requests by key. General requests use HTTP. |
| Playlist topics | One 170-second sample saw six `hm://playlist/v2`, two `hm://playlist/user`, and two `hm://playlist-permission/v1` messages. Each had one payload. |
| Playlist payload | URI (1), 24-byte new revision (2), parent revision (3), ops (4), timestamp (5), server domain (6). Desktop also defines truncated ops (7) and skip publish (8). Observed field 9 remains unidentified. Root notices used fields 1 and 2 on both playlist topics. |
| Live edits | Indexed ADDs at 0 and 1, then REM at 1 with length 1 and `items_as_key` false. One other operation was unidentified. |
| Playback observer | Registration returned a playback cluster and server timestamp. One 82-entry queue had duration, playback position, requested URI, and context, with no title, cover, or ISRC fields. |
| Track-end timing | Two near-end pushes beat the former recovery deadline by 3.832 s and 1.421 s. Two other changes came 160 s and 104 s early. Zero HTTP pulls. |
| Device switch | A 185-second push-only run received the switch. No track change or suspend/resume in that window. |
| Forced Dealer disconnect | The reconnect got a new connection ID. Observer registration restored the active device and track. A pause arrived by push and resume was accepted. OS suspend was untested. |
| Silent Dealer socket | A local socket announced itself and stayed open without answering the heartbeat. Cantus detected the timeout after 33 seconds. A separate test verified that a socket sending pongs still needs a connection announcement. |
| Dealer link outage | A loopback relay stopped forwarding a live Spotify socket. Dealer detected it in 32.6 s. Two reconnect attempts were blocked. Registration and reads completed 10.4 s after restoration, returning an active device, track, queue, and 16 configured playlists. |

Desktop `ConnectService.SendCommand` is a local RPC with a device ID and command JSON inside protobuf. `ProxyTransferCommand` also contains JSON. OPTIONS advertised POST on the known HTTP command route and gave 404 for the guessed remote `SendCommand` path.

A live playlist add requested resync. With a deliberately stale cached revision, its Dealer edit triggered a targeted read that recovered the added item. With the reader marked busy, the same edit requested a root refresh. A subsequent Cantus removal needed no read and restored the empty test playlist.

Desktop `SkipNextRequest` and `SkipPrevRequest` accept a `ContextTrack`. The HTTP `skip_next` and `skip_prev` commands accepted a top-level `track` with URI, UID, and metadata. A 25-track forward jump hit the exact UID, kept the session, and preserved all 54 remaining entries. Spotify reported 20 history entries afterward. A four-track backward jump hit the exact UID and kept the session. A separate Cantus forward jump preserved all 73 remaining entries with shuffle on and both repeat modes off.

## Request options and sizes

Registration, reads, pause, playlist edits, and save/unsave succeeded without `product=0` or `salt`. Lyrics returned 60 lines in all four combinations. Earlier lyrics probes returned 403 with and without salt.

| Response sample | Plain bytes | Gzip bytes |
|---|---:|---:|
| Two-track metadata | 1,809 | 1,309 |
| Root list, all fields | 23,306 | 8,132 |
| Root list, selected fields | 5,174 | 2,451 |

Gzipping the small test requests added 16–23 bytes. With credentials and hosts ready, Dealer plus observer registration took 227 ms.

| Live sample (one HTTP call each) | Request bytes | Gzip response bytes |
|---|---:|---:|
| Observer registration | 192 | 5,158 |
| Root list, 30 entries | 80 | 2,452 |
| Selected playlist, 50 items | 60 | 1,080 |
| Queue metadata, 85 tracks | 3,740 | 32,808 |
| **Four-call subtotal** | **4,072** | **41,498** |
| Metadata, 3 tracks | 132 | 1,824 |
| Audio features, 3 tracks | 0 | 523 |

Body bytes only. Discovery was one additional call with a 111-byte gzip body. Three covers chosen by first available URL took 110,464 bytes. Cantus's actual selector picked twelve 64 × 64 images totaling 29,474 bytes (1,120–4,552 each). Catalog-only selection matched those sizes. The artwork CDN negotiated HTTP/2.

## Static index

Rows below are extracted literals, service stubs, or native descriptors. They identify desktop evidence independently of the live probes. A literal alone does not establish an HTTP method, host, or response format.

<details>
<summary>First-party URL literals (89)</summary>

| Entry | Desktop source |
|---|---|
| `http://i.scdn.co/image` | installed native, native |
| `https://about-recommendations.spotify.com` | installed xpui/xpui-root-dialogs.js, xpui/xpui-root-dialogs.js |
| `https://accounts.spotify.com` | installed login/login.js, installed native +5 |
| `https://accounts.spotify.com/api/token` | installed native, native |
| `https://accounts.spotify.com/login` | installed xpui/xpui-modules.js, installed xpui/xpui-routes-blend.js +3 |
| `https://accounts.spotify.com/login/ott/v2` | installed native, installed xpui/xpui-modules.js +1 |
| `https://accounts.spotify.com/oauth2/device/authorize` | installed native, native |
| `https://accounts.spotify.com/oauth2/v2/auth` | installed native, native |
| `https://admocker.spotify.com` | installed xpui/xpui-modules.js |
| `https://admocker.spotify.com/mock/ad` | installed xpui/xpui-modules.js |
| `https://admocker.spotify.com/mock/display` | installed xpui/6174.js, xpui/6174.js |
| `https://admocker.spotify.com/track` | installed xpui/9409.js, xpui/9409.js |
| `https://ads.spotify.com` | installed xpui/xpui-modules.js |
| `https://adshelp.spotify.com/HelpCenter/s/article/advertising-policies-US` | installed xpui/2489.js, xpui/2489.js |
| `https://age-assurance.spotify.com` | installed xpui/xpui-root-dialogs.js, installed xpui/xpui-routes-desktop-settings.js +2 |
| `https://ap.spotify.com` | installed native, native |
| `https://api-partner.spotify.com` | installed login/login.js, installed xpui/pip-mini-player-snapshot.js +4 |
| `https://api.spotify.com` | installed login/login.js, installed xpui/pip-mini-player-snapshot.js +4 |
| `https://apresolve.spotify.com` | login/index.html, xpui/index.html |
| `https://artifactory.spotify.net/artifactory/api/npm/virtual-npm` | installed xpui/4304.js, xpui/4304.js |
| `https://backstage.spotify.net/docs/default/system/jam-system/guides/jam-tool` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `https://backstage.spotify.net/experimentation/propertiesList` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `https://backstage.spotify.net/experimentation/rollouts/37111` | installed native, native |
| `https://clienttoken.spotify.com/v1/clienttoken` | installed native, native |
| `https://daily-mix.scdn.co` | xpui/index.html |
| `https://encore-web.spotify.net` | installed xpui/5532.js, installed xpui/6174.js +2 |
| `https://exp.wg.spotify.com` | installed login/login.js, installed xpui/pip-mini-player-snapshot.js +4 |
| `https://ghe.spotify.net/js-sdk/betamax-sdk-desktop` | installed xpui/4304.js, xpui/4304.js |
| `https://i.scdn.co` | xpui/index.html |
| `https://i.scdn.co/image` | installed xpui/3704.js, installed xpui/xpui-modules.js +3 |
| `https://image-upload.spotify.com/v4` | installed xpui/xpui-modules.js, installed xpui/xpui-routes-profile.js +1 |
| `https://image-upload.spotify.com/v4/playlist` | installed native, native |
| `https://internal-podcast.spotify.net` | installed native, native |
| `https://jam-tool.spotify.net` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `https://label-page-service-hw25.spotify.com` | installed xpui/1430.js, xpui/1430.js |
| `https://lineup-images.scdn.co` | xpui/index.html |
| `https://login.app.spotify.com` | installed native, native |
| `https://login.app.spotify.com/index.html` | installed native, native |
| `https://login5.spotify.com/v3/login` | installed native, native |
| `https://login5.spotify.com/v4/login` | installed native, native |
| `https://mosaic.scdn.co` | installed xpui/xpui-modules.js |
| `https://newsroom.spotify.com` | installed xpui/xpui-modules.js |
| `https://open.spotifycdn.com` | xpui/index.html |
| `https://open.spotifycdn.com/cdn/templates/display.html` | installed xpui/dwp-panel-section.js, xpui/dwp-panel-section.js |
| `https://open.spotifycdn.com/cdn/templates/leaderboard.html` | installed xpui/xpui-modules.js |
| `https://open.spotifycdn.com/cdn/videos/sc/intro/computer.1572f2d3.webm` | installed xpui/xpui-modules.js |
| `https://open.spotifycdn.com/cdn/videos/welcome-mark.0e5cea81.webm` | installed xpui/xpui-modules.js |
| `https://pixel-static.spotify.com` | xpui/index.html |
| `https://pixel.spotify.com` | xpui/index.html |
| `https://pl.scdn.co` | xpui/index.html |
| `https://podstart.spotify.com/spotify.podstart.v0.PodstartForEnvoy/ShowResumePoint` | installed native, native |
| `https://remote-configuration-resolver.spotify.net/debug/traceback-config` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `https://seektables.spotifycdn.com/v1/seektable` | installed xpui/xpui-modules.js |
| `https://shareables.scdn.co/publish/socialsession` | installed xpui/xpui-modules.js |
| `https://spclient.wg.spotify.com` | installed xpui/xpui-modules.js |
| `https://spclient.wg.spotify.com/audio-attributes/v1/audio-analysis` | installed xpui/helper/spicetifyWrapper.js |
| `https://spclient.wg.spotify.com/colorextractor/v1/extract-presets` | installed xpui/helper/spicetifyWrapper.js |
| `https://spclient.wg.spotify.com/device-auth/v1/refresh` | installed xpui/xpui-modules.js |
| `https://spclient.wg.spotify.com/external-accessory-mapping-serving/v1/lookup` | installed native, native |
| `https://spclient.wg.spotify.com/gabo-receiver-service` | installed native, native |
| `https://spclient.wg.spotify.com/identity/v3/user/username` | installed native, native |
| `https://spclient.wg.spotify.com/listening-activity/v1` | installed xpui/9551.js, xpui/9551.js |
| `https://spclient.wg.spotify.com/melody/unauth` | installed xpui/xpui-modules.js |
| `https://spclient.wg.spotify.com/playlist/v2/playlist` | installed native, native |
| `https://spclient.wg.spotify.com/sneaktables/v2/hls/4` | installed native, native |
| `https://spclient.wg.spotify.com/spotbot-api2` | installed xpui/xpui-modules.js |
| `https://spclient.wg.spotify.com/super-connect-proxy` | installed native, installed xpui/xpui-modules.js +1 |
| `https://spclient.wg.spotify.com/user-customization-service/v1/customize` | installed native, native |
| `https://sponsored-recommendations.spotify.com/desktop` | installed xpui/2774.js, xpui/2774.js |
| `https://vocal-removal.scdn.co/karaoke-masks/spotify_file_` | installed native, native |
| `https://wgint.spotify.net` | installed login/login.js, installed xpui/pip-mini-player-snapshot.js +3 |
| `https://widget-content.spotify.com/v1/network-check` | installed native, native |
| `https://www-testing.spotify.net/us` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/accessibility` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/account/overview` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/download` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/duo` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/family` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/legal` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/legal/cookies-policy` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/legal/notice-at-collection` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/legal/privacy-policy` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/logout` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/premium` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/safetyandprivacy` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/select-your-country-region` | installed xpui/xpui-modules.js |
| `https://www-testing.spotify.net/us/student` | installed xpui/xpui-modules.js |
| `https://xpui.app.spotify.com/index.html` | installed native, native |
| `wss://dealer.g2.spotify.com:443` | installed xpui/xpui-modules.js |

</details>

<details>
<summary>Versioned route fragments (113)</summary>

| Entry | Desktop source |
|---|---|
| `/age-verification/v1` | installed xpui/xpui-modules.js |
| `/algotorial-sharing-service/v1` | installed login/login.js, installed xpui/xpui-modules.js +1 |
| `/api/logout/v1` | installed native, native |
| `/api/masthead/v1/masthead` | installed xpui/xpui-modules.js |
| `/api/permissions/v1/capabilities` | installed xpui/xpui-modules.js |
| `/api/permissions/v1/interactive` | installed xpui/xpui-modules.js |
| `/api/permissions/v1/manage` | installed xpui/xpui-modules.js |
| `/api/permissions/v1/manage/revocations` | installed xpui/xpui-modules.js |
| `/api/routines/v1/manage` | installed xpui/xpui-modules.js |
| `/assisted-curation/v1/recommendations/curation/uri` | installed xpui/xpui-modules.js |
| `/color-lyrics/v2` | installed xpui/xpui-modules.js |
| `/content-filter/v1` | installed xpui/xpui-modules.js |
| `/cultural-moment/v1` | installed xpui/xpui-modules.js |
| `/curiosity/v1` | installed xpui/xpui-modules.js |
| `/gander/v2` | installed xpui/xpui-modules.js |
| `/inspiredby-mix/v2` | installed xpui/xpui-modules.js |
| `/library-import/v1` | installed xpui/xpui-modules.js |
| `/listening-activity/v1/audience` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/feed` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/subscribe` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/unsubscribe` | installed xpui/9551.js, xpui/9551.js |
| `/mix-list-lens/api/v0/duplicate` | installed xpui/dwp-panel-section.js, xpui/dwp-panel-section.js |
| `/pathfinder/v1` | installed xpui/xpui-modules.js |
| `/pathfinder/v2` | installed xpui/xpui-modules.js |
| `/playlist-permission/v1` | installed xpui/xpui-modules.js |
| `/playlist/v2` | installed xpui/xpui-modules.js |
| `/podcast-charts-service/v2/charts/hubs` | installed xpui/xpui-modules.js |
| `/ratings/v1` | installed xpui/xpui-modules.js |
| `/socialgraph/v4` | installed xpui/xpui-modules.js |
| `/speakeasy/v2` | installed xpui/xpui-modules.js |
| `/sponsoredplaylist/v1` | installed xpui/xpui-modules.js |
| `/sponsoredplaylist/v1/mock/sponsored` | installed xpui/xpui-modules.js |
| `/sponsoredplaylist/v1/sponsored` | installed xpui/xpui-modules.js |
| `/the-stage/v1` | installed xpui/xpui-modules.js |
| `/track-error/v1/errors` | installed native, native |
| `/transcript-read-along/v2` | installed xpui/xpui-modules.js |
| `/upsell/v1/contextual-upsell` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `/upsell/v1/should-upsell` | installed xpui/xpui-debug-window-contents.js, xpui/xpui-debug-window-contents.js |
| `/user-profile-view/v3` | installed xpui/xpui-modules.js |
| `ad-logic/v3/confgic/state/config` | installed native, native |
| `adpreview/v1/preview` | installed native, native |
| `auto-download-service/v1/replace-identifiers` | installed native, native |
| `auto-download-service/v1/settings` | installed native, native |
| `auto-download-service/v1/show` | installed native, native |
| `client-tts/v1/fulfill` | installed native, native |
| `clientsettings/api/v1` | installed native, native |
| `collection/v2/initialized` | installed native, native |
| `connect-group/v1/refresh-group-data` | installed native, native |
| `connect-state/v1` | installed native, native |
| `connect-state/v1/cluster/wake-devices` | installed native, native |
| `connect-state/v1/connect/group/add-device/from` | installed native, native |
| `connect-state/v1/connect/group/delete/from` | installed native, native |
| `connect-state/v1/connect/group/remove-device/from` | installed native, native |
| `connect-state/v1/connect/logout/from` | installed native, native |
| `connect-state/v1/connect/rename/from` | installed native, native |
| `connect-state/v1/connect/transfer/from` | installed native, native |
| `connect-state/v1/connect/volume/from` | installed native, native |
| `connect-state/v1/devices` | installed native, native |
| `connect-state/v1/player/command/from` | installed native, native |
| `connect-state/v1/update_local_devices` | installed native, native |
| `crypto/ocsp/v3` | installed native, native |
| `crypto/x509/v3` | installed native, native |
| `desktop-update/v1` | installed native, native |
| `device-capabilities/v1/capabilities` | installed native, native |
| `extended-metadata/v0/extended-metadata` | installed native, native |
| `extended-metadata/v3/extensions/batch-get` | installed native, native |
| `feedback/v1/feedback` | installed native, native |
| `feedback/v1/feedback/dislike` | installed native, native |
| `feedback/v1/feedback/home-promotion/dislike` | installed xpui/2774.js, xpui/2774.js |
| `gabo-receiver-service/public/v3/events` | installed native, native |
| `gabo-receiver-service/v3/events` | installed native, native |
| `hwptp/v1/capabilities` | installed native, native |
| `identity/v3/user/username` | installed native, native |
| `image-resolve/v1/resolve/map` | installed native, native |
| `image-upload/v4/playlist` | installed xpui/xpui-modules.js |
| `image-upload/v4/user-profile` | installed xpui/xpui-routes-profile.js, xpui/xpui-routes-profile.js |
| `istanbuljs/v8` | xpui/ui-licenses.html |
| `lfs-secret-provider/v1/secret` | installed native, native |
| `padme/v2/metadata/fileId` | installed native, native |
| `padme/v2/metadata/manifestId` | installed native, native |
| `played-state/v1/unsubscribe` | installed native, native |
| `played-state/v1/updates` | installed native, native |
| `playlist-permission/v1/album` | installed native, native |
| `playlist-permission/v1/default-owner-capabilities` | installed native, native |
| `playlist-permission/v1/list` | installed native, native |
| `playlist-permission/v1/playlist` | installed native, native |
| `playlist-permission/v1/show` | installed native, native |
| `playlist-publish/v1/subscription` | installed native, native |
| `playlist/v2` | installed native, native |
| `playlist/v2/user` | installed native, native |
| `playlistextender/ft/v2/extend-playlist` | installed native, native |
| `quicksilver/v2` | installed xpui/xpui-modules.js |
| `quicksilverdev/v2` | installed xpui/xpui-modules.js |
| `recently-played/v3/recently-played` | installed native, native |
| `searchview/v3/search` | installed native, native |
| `sessiontransfer/v1/token` | installed native, native |
| `social-connect/v2/broadcast_status_update` | installed xpui/xpui-modules.js |
| `social-connect/v2/session_messages` | installed xpui/xpui-modules.js |
| `social-connect/v2/session_update` | installed xpui/xpui-modules.js |
| `social-connect/v2/sessions` | installed native, native |
| `social-connect/v2/sessions/current` | installed native, native |
| `social-connect/v2/sessions/current/queue_only_mode` | installed native, native |
| `social-connect/v2/sessions/current/volume_control` | installed native, native |
| `social-connect/v2/sessions/current_or_new` | installed native, native |
| `social-connect/v3/sessions` | installed native, native |
| `socialgraph/v2/following` | installed native, native |
| `socialgraph/v4` | installed native, native |
| `speechless/v1/retrieve/chats-share-set/latest` | installed native, native |
| `speechless/v1/retrieve/hierarchy` | installed native, native |
| `speechless/v1/submit/request` | installed native, native |
| `storage-resolve/v2/files/audio` | installed native, native |
| `track-error/v1/errors` | installed native, native |
| `tv-token-exchange/v1/token/code` | installed native, native |

</details>

<details>
<summary>Additional JavaScript request-builder paths (57)</summary>

| Entry | Desktop source |
|---|---|
| `/api/logout` | installed xpui/xpui-modules.js |
| `/audience` | installed xpui/9551.js, xpui/9551.js |
| `/capping-api/spotify.cappingapi.v1.CappingExplanation/GetAudiobooksExplanation` | installed xpui/xpui-modules.js |
| `/CheckoutBook` | installed xpui/xpui-modules.js |
| `/command/play` | installed xpui/xpui-modules.js |
| `/dsa-metadata` | installed xpui/xpui-modules.js |
| `/eligible` | installed xpui/xpui-routes-your-library-x.js, xpui/xpui-routes-your-library-x.js |
| `/extendp/` | installed xpui/xpui-modules.js |
| `/feed` | installed xpui/9551.js, xpui/9551.js |
| `/get-devices/` | installed xpui/xpui-modules.js |
| `/GetUserHasUnreadNotification` | installed xpui/xpui-modules.js |
| `/hub` | installed xpui/xpui-routes-cultural-moment-hub.js, xpui/xpui-routes-cultural-moment-hub.js |
| `/liked-songs` | installed xpui/xpui-modules.js |
| `/listening-activity/v1/audience` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/feed` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/subscribe` | installed xpui/9551.js, xpui/9551.js |
| `/listening-activity/v1/unsubscribe` | installed xpui/9551.js, xpui/9551.js |
| `/marketingPage` | installed xpui/1912.js, xpui/1912.js |
| `/mix-list-lens/api/v0/duplicate` | installed xpui/dwp-panel-section.js, xpui/dwp-panel-section.js |
| `/mock/list` | installed xpui/xpui-modules.js |
| `/playlist` | installed xpui/xpui-modules.js |
| `/playlist/{id}/permission-grant` | installed xpui/xpui-modules.js |
| `/playlist/{id}/permission-grant/token/{token}/claim` | installed xpui/xpui-modules.js |
| `/podcast-charts-hubs/categories` | installed xpui/podcast-charts-hub.js, xpui/podcast-charts-hub.js |
| `/podcast-charts-hubs/regions` | installed xpui/podcast-charts-hub.js, xpui/podcast-charts-hub.js |
| `/podcast-charts-hubs/root` | installed xpui/podcast-charts-hub.js, xpui/podcast-charts-hub.js |
| `/podcast-charts-hubs/{chartType}` | installed xpui/podcast-charts-hub.js, installed xpui/podcast-charts.js +2 |
| `/query` | installed xpui/xpui-modules.js |
| `/ResetLatestCursor` | installed xpui/xpui-modules.js |
| `/resolve-uri/{uri}` | installed xpui/xpui-modules.js |
| `/seed_to_playlist/{uri}` | installed xpui/xpui-modules.js |
| `/seo` | installed xpui/xpui-modules.js |
| `/shared` | installed login/login.js, installed xpui/xpui-modules.js +1 |
| `/site/{lookupId}` | installed xpui/xpui-routes-stage-page.js, xpui/xpui-routes-stage-page.js |
| `/sites/{lookupId}/metadata/{localeCode}` | installed xpui/xpui-routes-stage-page.js, xpui/xpui-routes-stage-page.js |
| `/sub/remove-subscriptions` | installed xpui/xpui-modules.js |
| `/subscribe` | installed xpui/9551.js, xpui/9551.js |
| `/track/{trackId}` | installed xpui/xpui-modules.js, installed xpui/xpui-routes-track-v2.js +1 |
| `/unsubscribe` | installed xpui/9551.js, xpui/9551.js |
| `/user-profile` | installed xpui/xpui-routes-profile.js, xpui/xpui-routes-profile.js |
| `/v0/experimental/{trackId}/credits` | installed xpui/xpui-root-dialogs.js, xpui/xpui-root-dialogs.js |
| `/v0/taste-profile` | installed xpui/xpui-routes-taste-profile.js, xpui/xpui-routes-taste-profile.js |
| `/v1/agent/create-or-get` | installed xpui/xpui-modules.js |
| `/v1/buddylist` | installed xpui/xpui-modules.js |
| `/v1/generate` | installed xpui/xpui-routes-blend.js, xpui/xpui-routes-blend.js |
| `/v1/generate-url` | installed xpui/xpui-modules.js |
| `/v1/notes` | installed xpui/xpui-routes-taste-profile.js, xpui/xpui-routes-taste-profile.js |
| `/v1/notes/{noteId}` | installed xpui/xpui-routes-taste-profile.js, xpui/xpui-routes-taste-profile.js |
| `/v1/user` | installed xpui/xpui-modules.js |
| `/v2/config` | installed xpui/xpui-modules.js |
| `/v2/init-friend-feed/{connectionId}` | installed xpui/xpui-modules.js |
| `/v2/read-settings` | installed xpui/xpui-modules.js |
| `/v2/store-settings` | installed xpui/xpui-modules.js |
| `/v3/profile-image/{username}` | installed xpui/xpui-routes-profile.js, xpui/xpui-routes-profile.js |
| `/v3/profile-image/{username}/{uploadToken}` | installed xpui/xpui-routes-profile.js, xpui/xpui-routes-profile.js |
| `/v3/user/username/{username}` | installed xpui/xpui-routes-profile.js, xpui/xpui-routes-profile.js |
| `/{username}/is-blocking/{target}` | installed xpui/xpui-modules.js |

</details>

<details>
<summary>Namespaced RPC path literals (15)</summary>

| Entry | Desktop source |
|---|---|
| `/capping-api/spotify.cappingapi.v1.CappingExplanation/GetAudiobooksExplanation` | installed xpui/xpui-modules.js |
| `capping-api/spotify.cappingapi.v1.CappingApi/PutConsumption` | installed native, native |
| `com/spotify.podstart.v0.PodstartForEnvoy/ShowResumePoint` | installed native, native |
| `herodotus/spotify.resumption.v1.CurrentStateService/ListCurrentStates` | installed native, native |
| `herodotus/spotify.resumption.v1.ResumePointRevisionService/BatchCreateResumePointRevisions` | installed native, native |
| `herodotus/spotify.resumption.v1.ResumePointRevisionService/CreateResumePointRevision` | installed native, native |
| `herodotus/spotify.resumption.v1.ResumePointRevisionService/ListResumePointRevisions` | installed native, native |
| `playlist/spotify.list.v1.ListReadService/GetLists` | installed native, native |
| `pns-capper/spotify.pnscapper.v1.CapService/GetCapInfo` | installed native, native |
| `sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/RemoteAction` | installed native, native |
| `sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveAudioManifest` | installed native, native |
| `sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolveContext` | installed native, native |
| `sequence-proxy/spotify.sequenceproxy.v1.SequenceProxyService/ResolvePage` | installed native, native |
| `spotify.clientrestrictions.v1.ClientRestrictionsService/GetRestrictedIntegrations` | installed native, native |
| `tags-service/spotify.tags.v1.SuggestionService/GetSuggestions` | installed native, native |

</details>

<details>
<summary>Cosmos and Mercury literals (60)</summary>

| Entry | Desktop source |
|---|---|
| `hm://affinity/v1` | installed native, native |
| `hm://artistplaycontext/v1/page/spotify/album` | installed xpui/xpui-modules.js |
| `hm://audio-attributes/v1/audio-analysis` | installed native, native |
| `hm://campfire/v1/hierarchy-message-changed` | installed native, native |
| `hm://collection/%s` | installed native, native |
| `hm://collection/ylpin` | installed xpui/xpui-routes-your-library-x.js, xpui/xpui-routes-your-library-x.js |
| `hm://common-capping/update` | installed native, native |
| `hm://conf` | installed native, native |
| `hm://connect-group/v1/group-data-event` | installed native, native |
| `hm://connect-group/v1/group-device-volume-event` | installed native, native |
| `hm://connect-state/v1/cluster` | installed native, native |
| `hm://connect-state/v1/connect/logout` | installed native, native |
| `hm://connect-state/v1/connect/proxy_transfer` | installed native, native |
| `hm://connect-state/v1/connect/proxy_wake` | installed native, native |
| `hm://connect-state/v1/connect/set_backend_metadata` | installed native, native |
| `hm://connect-state/v1/connect/volume` | installed native, native |
| `hm://connect-state/v1/group/add-device` | installed native, native |
| `hm://connect-state/v1/group/delete` | installed native, native |
| `hm://connect-state/v1/group/remove-device` | installed native, native |
| `hm://connect-state/v1/group/resync` | installed native, native |
| `hm://connect-state/v1/ping` | installed native, native |
| `hm://connect-state/v1/player/command` | installed native, native |
| `hm://context-resolve/v1/autoplay` | installed native, native |
| `hm://context-resolve/v1/autopodcast` | installed native, native |
| `hm://event-service/v1/events` | installed native, native |
| `hm://herodotus/batch` | installed native, native |
| `hm://herodotus/uri` | installed native, native |
| `hm://identity/user-profile-changed` | installed native, native |
| `hm://listening-activity` | installed xpui/9551.js, xpui/9551.js |
| `hm://playlist` | installed native, native |
| `hm://playlist-pep:` | installed native, native |
| `hm://presence2/user` | installed xpui/xpui-modules.js |
| `hm://show-access/update/json` | installed xpui/xpui-modules.js |
| `hm://show-access/update/proto` | installed native, native |
| `hm://show-access/user-capped` | installed xpui/xpui-modules.js |
| `hm://show-access/user-quota-changed` | installed xpui/xpui-modules.js |
| `hm://social-prompting/v1/session-update` | installed xpui/dwp-socialPrompting.js, xpui/dwp-socialPrompting.js |
| `hm://speech_visualization/v1/generate/x-dj` | installed native, native |
| `sp://_routes` | installed native, native |
| `sp://auth/v1/session_transfer_token` | installed xpui/xpui-modules.js |
| `sp://auth/v1/token_web_auth` | installed xpui/xpui-routes-stage-page.js, xpui/xpui-routes-stage-page.js |
| `sp://desktop/v1/browse_for_folder` | installed native, installed xpui/xpui-modules.js +1 |
| `sp://desktop/v1/clipboard` | installed native, installed xpui/xpui-modules.js +1 |
| `sp://desktop/v1/shutdown` | installed native, native |
| `sp://esperanto` | installed native, native |
| `sp://event_sender_test/v1/event` | installed native, native |
| `sp://event_sender_test/v1/sending_interval` | installed native, native |
| `sp://event_sender_test/v1/sent_events` | installed native, native |
| `sp://internal/v1/force_crash` | installed native, native |
| `sp://internal/v1/latest_messages` | installed native, installed xpui/xpui-modules.js +1 |
| `sp://logging/v1` | installed native, native |
| `sp://logging/v2` | installed native, native |
| `sp://logging/v3` | installed native, native |
| `sp://messages/v1/container/control` | installed login/login.js, installed native +4 |
| `sp://videoplayer/v1` | installed native, installed xpui/xpui-modules.js +1 |
| `sp://videoplayer/v1/advance` | installed native, native |
| `sp://videoplayer/v1/commands` | installed native, native |
| `sp://videoplayer/v1/error` | installed native, native |
| `sp://videoplayer/v1/event` | installed native, native |
| `sp://videoplayer/v1/state` | installed native, native |

</details>

<details>
<summary>Protobuf service and method index (100)</summary>

| Service | Methods | Desktop source |
|---|---|---|
| `spotify.ad_detection.impl.ad_detection_proto.AdDetection` | `Detect`, `MapTimestamp`, `SubscribeCreatorToUserTimestamp`, `SubscribeUserToCreatorTimestamp` | native descriptor |
| `spotify.ads.esperanto.proto.AdOpportunity` | `PostEvent`, `PostEventV2`, `postEvent`, `postEventV2` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Ads` | `GetAds` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Capabilities` | `addCapability`, `removeCapability` | native descriptor |
| `spotify.ads.esperanto.proto.DeviceState` | `GetDeviceVolume`, `SetDeviceVolume` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Events` | `PostEvent`, `PostUnmanagedEvent`, `SubEvent`, `postEvent`, `postUnmanagedEvent`, `subEvent` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Formats` | `SubFormat` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.InStream` | `SubBreakChanged`, `SubInStream` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.PodcastTesting` | `GetNextAdSegment`, `OverrideMetadata` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Preview` | `Preview` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Settings` | `DeleteRequestHeaders`, `GetAllSlotSettings`, `GetRequestHeaders`, `GetSlotSettings`, `SetRequestHeaders`, `UpdateAdServerEndpoint`, `UpdateAdStateEndpoint`, `UpdateDisplayTimeInterval`, `UpdateExpiryTimeInterval`, `UpdateSlotEnabled`, `UpdateStreamTimeInterval`, `deleteRequestHeaders`, `getAllSlotSettings`, `getRequestHeaders`, `getSlotSettings`, `setRequestHeaders`, `updateAdServerEndpoint`, `updateAdStateEndpoint`, `updateDisplayTimeInterval`, `updateExpiryTimeInterval`, `updateSlotEnabled`, `updateStreamTimeInterval` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Slots` | `ClearAllAds`, `ClearAvailableAds`, `CreateSlot`, `FetchSlot`, `GetSlot`, `GetSlots`, `PrepareNextContextSlot`, `PrepareNextTrackSlot`, `SubSlot`, `TriggerSlot` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.State` | `GetState`, `PutState`, `SubState` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Targeting` | `DeleteTargeting`, `GetAllTargeting`, `GetTargeting`, `PutTargeting` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.ads.esperanto.proto.Testing` | `AddPlaytime`, `InsertAds`, `SetAlwaysEligibleForAdBreak` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.analytics_esperanto.proto.Analytics` | `SubMessage` | installed xpui/4764.js, xpui/4764.js |
| `spotify.analyzer_esperanto.proto.AnalyzerService` | `SubLoudness` | installed xpui/xpui-modules.js |
| `spotify.authentication.login5.impl.proto.AccessTokenClient` | `GetToken`, `OnPermanentError`, `OnTokenCleared` | installed xpui/xpui-modules.js |
| `spotify.automix.bundle.esperanto.proto.AutomixBundle` | `GetMixingBundle` | native descriptor |
| `spotify.automix.esperanto.proto.Automix` | `AdjustTransitionRecipeForItemSpeeds`, `GetBeatmatchOptOutTransitionDuration`, `GetComputedTransitions`, `GetEqStylesForEqStyleId`, `GetFilterFxStylesForFilterFxStyleId`, `GetFxStylesForFxStyleId`, `GetJogwheelBlocks`, `GetLoopingBlocks`, `GetMaxTransitionLength`, `GetStylesForPresetId`, `GetVolumeStylesForVolumeStyleId` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.collection_esperanto.proto.CollectionService` | `Add`, `AddUnplayed`, `Ban`, `BanGlobal`, `BanInPlayContext`, `Contains`, `Decorate`, `GetArtistTrackList`, `GetArtistView`, `GetBucketSyncStatus`, `GetTrackList`, `GetUnplayedEpisodes`, `IsSnoozed`, `OfflineArtist`, `PlayArtist`, `PlayTracks`, `Remove`, `RemoveOfflineArtist`, `RemoveUnplayed`, `Snooze`, `StreamArtistTrackList`, `StreamArtistView`, `StreamContains`, `StreamDecorate`, `StreamTagsInfo`, `StreamTrackList`, `StreamTrackListMetadata`, `StreamUnplayedEpisodes`, `Unban`, `UnbanGlobal`, `UnbanInPlayContext`, `Unsnooze` | installed xpui/xpui-modules.js |
| `spotify.collection_platform_esperanto.proto.CollectionPlatformService` | `Add`, `Contains`, `Get`, `Remove`, `StreamContains`, `StreamGet` | installed xpui/xpui-modules.js |
| `spotify.common_capping.impl.common_capping_proto.CommonCapping` | `GetCapInfo`, `GetQuotaStateAt`, `IsCapped`, `ObserveCapInfo`, `ObserveCapStateChanged`, `Refresh`, `ReportConsumption` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.connect.esperanto.proto.ConnectService` | `AddDeviceToGroup`, `BecomeInactive`, `CancelTransfer`, `ClusterUpdateAfterBackendConnection`, `CreateStaticGroup`, `DeleteGroup`, `GetDebugLevel`, `InstallSpotify`, `Logout`, `NotifyAudioRouteChanged`, `PerformDiscovery`, `Pull`, `RefreshCluster`, `RemoveDeviceFromGroup`, `Rename`, `RenameGroup`, `SendCommand`, `SetDebugLevel`, `SetPreferredIncarnation`, `State`, `StopScanning`, `Transfer`, `WakeSleepingClusterDevices` | installed xpui/xpui-modules.js |
| `spotify.connect.esperanto.proto.ConnectVolumeService` | `LocalVolume`, `OnEnablingLocalPlayback`, `Volume`, `VolumeDown`, `VolumeSet`, `VolumeUp` | installed xpui/xpui-modules.js |
| `spotify.connectivity.connection_quality.esperanto.proto.ConnectionQualityService` | `listenToConnectionQualityEvents` | native descriptor |
| `spotify.connectivity.esperanto.proto.ConnectivityPolicy` | `GetRules`, `GetState`, `SetConnectionType`, `SetForceOffline`, `SetRules`, `SubState`, `getRules`, `getState`, `setConnectionType`, `setForceOffline`, `setRules`, `subState` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.connectivity.hosts.esperanto.proto.HostsProvider` | `Host`, `Hosts`, `ReportConnectionError` | installed xpui/xpui-modules.js |
| `spotify.connectivity.pubsub.esperanto.proto.PubSub` | `AddOnNewConnectionID`, `AddOnPushedMessageForIdent`, `AddOnPushedMessageForIdentFilter` | installed xpui/xpui-modules.js |
| `spotify.debug_tracer.proto.DebugTracerService` | `trace`, `trace_playback_id`, `trace_stream_handle` | native descriptor |
| `spotify.desktop.audio_observer_esperanto.proto.DesktopAudioObserver` | `GetAudioOutputDevices` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.ble_debug_esperanto.proto.BleDebug` | `Subscribe` | native descriptor |
| `spotify.desktop.commands_esperanto.proto.DesktopCommands` | `SubscribeCommand` | installed xpui/4764.js, native descriptor +1 |
| `spotify.desktop.issue_reporter_esperanto.proto.DesktopIssueReporter` | `SubmitIssue` | installed xpui/4764.js, native descriptor +1 |
| `spotify.desktop.lifecycle_esperanto.proto.DesktopLifecycle` | `NotifyViewLoaded`, `Restart`, `Shutdown` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.local_jam.proto.DesktopLocalJam` | `StartBleBroadcasting`, `StartBroadcasting`, `StartScanning`, `StopBleBroadcasting`, `StopBroadcasting`, `StopScanning`, `SubNetworkInterfaces` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.login_esperanto.proto.DesktopLogin` | `ApproveConnectLogin`, `GetSessionState`, `LoginWithAccounts`, `LoginWithConnect`, `LoginWithQRCode`, `Logout` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.logs_esperanto.proto.DesktopLogs` | `CopyLogsToTempFolder`, `OpenFolder` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.navigation_esperanto.proto.DesktopNavigation` | `SubscribeDeepLink`, `SubscribeNavigation` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.notifications.proto.NotificationsService` | `GetAvailability`, `OpenNotificationsSetting`, `ShowToast` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.platform_info_esperanto.proto.DesktopPlatformInfo` | `GetPlatformInfo` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.proxy_settings_esperanto.proto.DesktopProxySettings` | `GetProxySettings`, `SetProxySettings` | installed login/login.js, installed xpui/4764.js +3 |
| `spotify.desktop.remote_config_esperanto.proto.DesktopRemoteConfig` | `ClearOverrides`, `GetConfiguration`, `GetOverrides`, `GetProperties`, `SetOverride` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.super_connect.proto.DesktopSuperConnect` | `CheckSandboxStatus`, `CheckStudioInstalled`, `GetKeepAwakeStatus`, `GetOrchestratorToken`, `GetVersions`, `InstallStudio`, `OpenStudio`, `ResolvePermission`, `RunSandboxSetup`, `SubscribePermissionRequests` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.update_esperanto.proto.DesktopUpdate` | `ApplyUpdate`, `SubscribeStatus`, `TriggerDownload` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.update_ui_esperanto.proto.DesktopUpdateUi` | `SetButtonsVisibility`, `UpdateMenuState`, `UpdateTitlebarHeight` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.version_esperanto.proto.DesktopVersion` | `GetVersionInfo` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.watchdog_esperanto.proto.DesktopWatchdog` | `Ping`, `Subscribe` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.desktop.window_state_esperanto.proto.DesktopWindowState` | `CloseWindow`, `FocusWindow`, `GetWindowState`, `SetWindowState`, `SubscribeWindowState` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.desktop.zoom_esperanto.proto.DesktopZoom` | `GetZoomLevel`, `SetZoomLevel`, `ZoomIn`, `ZoomOut` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.display_segments_esperanto.proto.v1.DisplaySegmentsService` | `GetDisplaySegments` | installed xpui/4764.js, xpui/4764.js |
| `spotify.download.esperanto.proto.Download` | `CreateProgressiveFileStreamer`, `CreateSegmentFileStreamer`, `DestroyFileStreamer`, `DestroySegmentFileStreamer`, `FinishSegmentDataRequest`, `GetFirstCachedSegmentIndex`, `GetFirstCachedSegmentIndexAsync`, `GetMediaHandleId`, `GetMediaManifest`, `IsFileFullyCached`, `RequestData`, `RequestSegmentData`, `SubscribeVideoBandwidth` | native descriptor |
| `spotify.exclusive_mode_esperanto.proto.ExclusiveModeService` | `EnumerateDevices`, `GetDevice`, `GetEnable`, `SetDevice`, `SetEnable` | installed xpui/xpui-modules.js |
| `spotify.image.esperanto.proto.Image` | `GetCacheStatus`, `GetImage`, `SetProjectionMap` | installed xpui/4764.js, native descriptor +1 |
| `spotify.issue_reporter.proto.IssueReporter` | `SubmitIssue` | native descriptor |
| `spotify.karaoke_esperanto.proto.KaraokeService` | `GetStatus`, `PostStatus`, `PostVocalVolume`, `SubscribeToEvents` | installed xpui/xpui-modules.js |
| `spotify.local_files_esperanto.proto.LocalFiles` | `AddFolder`, `GetScannerStatus`, `GetSources`, `GetTracks`, `MutateDefaultSource`, `MutateDefaultSources`, `NotifyDefaultSources`, `RemoveFolder`, `SubscribeScannerStatus`, `SubscribeSources`, `SubscribeTracks` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.mdata_esperanto.proto.MetadataService` | `Expire`, `Fetch`, `Observe`, `Purge` | installed xpui/xpui-modules.js |
| `spotify.metadata_esperanto.proto.ClassicMetadataService` | `GetEntity`, `MultigetEntity` | native descriptor |
| `spotify.narration.proto.ClientTtsService` | `GetTtsUrl` | native descriptor |
| `spotify.offline.proto.v2.OfflinePlugin` | `Download`, `Identify`, `Remove`, `StopDownload`, `download`, `identify`, `remove`, `stopDownload` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.offline_esperanto.proto.Offline` | `AddDownload`, `BulkRedownloadCancel`, `BulkRedownloadPreflight`, `BulkRedownloadStart`, `GetContexts`, `GetContextsProgress`, `GetDevices`, `GetEstimatedStorageIncrease`, `GetItems`, `GetListsContainingItem`, `GetLocalDevice`, `GetOfflineLicenseInfo`, `GetOfflinePlaybackStatus`, `GetTotalProgress`, `HasResource`, `LicenseStatus`, `PrepareResource`, `RemoveAllDownloads`, `RemoveDownload`, `RemoveResource`, `SetOfflinePlaybackAllowed`, `SubscribeBulkRedownloadProgress`, `SubscribeContextAdded`, `SubscribeContexts`, `SubscribeContextsProgress`, `SubscribeDevicesWithContext`, `SubscribeError`, `SubscribeItems`, `SubscribeOfflinePlaybackStatus`, `SubscribeTotalProgress` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.offline_esperanto.proto.OfflineDebug` | `OnKeyStats`, `SetLinkAdditionalExtensions` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.offline_playable_cache_esperanto.proto.OfflinePlayableCache` | `GetTracks`, `HasEnoughTracks`, `RemoveTrack`, `SubscribeHasEnoughTracks`, `SubscribeTracks` | installed xpui/4764.js, native descriptor +1 |
| `spotify.pending_events.esperanto.proto.PendingEvents` | `AddPendingEvent`, `CommitPendingEvent`, `ReplacePendingEvent` | installed xpui/xpui-modules.js |
| `spotify.perf_metrics.esperanto.proto.PerfMetricsService` | `TerminateState` | native descriptor |
| `spotify.playability.esperanto.proto.Playability` | `GetLocallyPlayable`, `SubscribeLocallyPlayable` | native descriptor |
| `spotify.playback_esperanto.proto.PlaybackService` | `Duck`, `GetFiles`, `GetFormats`, `GetPlaybackInfo`, `GetRawVolume`, `GetVolume`, `LowerVolume`, `RaiseVolume`, `SetRawVolume`, `SetVolume`, `SubBufferUnderrun`, `SubPlaybackInfo`, `SubPosition`, `SubRawVolume`, `SubVolume` | installed xpui/xpui-modules.js |
| `spotify.playback_settings.esperanto.proto.PlaybackSettings` | `GetDeviceSettings`, `GetSettings`, `GetShuffleState`, `SetDeviceSettings`, `SetDeviceStreamQuality`, `SetMediaMode`, `SetSettings`, `SetShuffleState`, `StreamDeviceSettingFieldChanges`, `StreamSettingChanges` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.playbacksettings.PlaybackSettingsService` | `GetAllStoredContentValues`, `GetContentSettings`, `GetDeviceSettings`, `GetSettingsDeviceSelection`, `WriteContentValue`, `WriteDeviceValue` | native descriptor |
| `spotify.player.esperanto.proto.ContextPlayer` | `AddToQueue`, `DeleteSession`, `GetError`, `GetPlayHistory`, `GetPositionState`, `GetQueue`, `GetState`, `Pause`, `Play`, `PlayAsNextInQueue`, `PlayPrepared`, `PreparePlay`, `Resume`, `SeekTo`, `SetOptions`, `SetQueue`, `SetRepeatingContext`, `SetRepeatingTrack`, `SetShufflingContext`, `SetSleepTimer`, `Signal`, `SkipNext`, `SkipPrev`, `Stop`, `UpdateContext`, `UpdateContextPage`, `UpdateContextTrack`, `UpdateViewUri` | installed xpui/xpui-modules.js |
| `spotify.player.esperanto.proto.ContextResolveMock` | `GetLastAutoplayRequestInfo`, `GetRequestInfo`, `UpdateContext` | native descriptor |
| `spotify.player.esperanto.proto.SequenceRules` | `resetSkips`, `skipsRemaining` | native descriptor |
| `spotify.playlist.local_list_config_esperanto.proto.LocalListConfigService` | `Save` | installed xpui/xpui-modules.js |
| `spotify.playlist_esperanto.proto.PlaylistDataService` | `Find`, `Get`, `MultiGet`, `Subscribe` | installed xpui/xpui-modules.js |
| `spotify.playlist_esperanto.proto.PlaylistPlayService` | `Play` | installed xpui/xpui-modules.js |
| `spotify.playlist_esperanto.proto.PlaylistService` | `ClearToken`, `Contains`, `EmitItemSignals`, `EmitSignals`, `GetMembers`, `InvalidateRevisions`, `Modify`, `RequestLenses`, `Resync`, `SetBasePermission`, `SetMemberPermission`, `SetOfflineState`, `SetToken`, `SubscribeToMembers` | installed xpui/xpui-modules.js |
| `spotify.playlist_esperanto.proto.RootlistDataService` | `Get`, `Subscribe` | installed xpui/xpui-modules.js |
| `spotify.playlist_esperanto.proto.RootlistModificationService` | `Modify` | installed xpui/xpui-modules.js |
| `spotify.podcast_paywalls_esperanto.proto.PodcastPaywalls` | `PutShowSubscription` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.prefetch_esperanto.proto.MediaPrefetch` | `UpdateFuture`, `UpdateSegmentProfileSelectionConfig` | native descriptor |
| `spotify.prefetch_esperanto.proto.Prefetch` | `GetEventHistory`, `GetQueue` | installed xpui/4764.js, native descriptor +1 |
| `spotify.prefs.esperanto.proto.Prefs` | `Create`, `Get`, `GetAll`, `Set`, `Sub`, `SubAll` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.product_state.esperanto.proto.ProductState` | `DelOverridesValues`, `GetValues`, `PutOverridesValues`, `PutValues`, `PutValuesSync`, `SubValues` | installed login/login.js, installed xpui/xpui-modules.js +2 |
| `spotify.profile_esperanto.proto.v1.ProfileService` | `ChangeDisplayName`, `GetProfiles`, `SubscribeToProfiles` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.progress_playback_esperanto.proto.ResumptionProgressPlayback` | `OnUpdate` | native descriptor |
| `spotify.puffin_esperanto.proto.PuffinService` | `DisableProcessing`, `EnterProcessingEnvironment`, `Filter`, `LeaveProcessingEnvironment`, `LibraryVersion`, `SubscribeToErrors`, `SubscribeToProcessingCompatibility` | installed xpui/xpui-modules.js |
| `spotify.recently_played_esperanto.proto.RecentlyPlayedService` | `Stream`, `Tracks` | installed xpui/xpui-modules.js |
| `spotify.remote_config.esperanto.proto.RemoteConfig` | `lookupBool`, `lookupEnum`, `lookupInt` | native descriptor |
| `spotify.remote_config.esperanto.proto.RemoteConfigDummyResolve` | `ResolveDummyProperty` | native descriptor |
| `spotify.restrictions.permissions.permissions_proto.Permissions` | `CanAddToQueue`, `CanPlayContent`, `CanSeeQueue`, `CanSetQueue`, `CanSkipPodcastVideoAd`, `CanSwitchToMusicVideo`, `ObserveInvalidation` | installed xpui/4764.js, native descriptor +1 |
| `spotify.resumption_progress_esperanto.proto.ResumptionProgress` | `GetAllDetails`, `GetDetails`, `GetProgress`, `GetResumePoint`, `GetState`, `MarkAsFinished`, `MarkAsNotStarted`, `SubDetails`, `SubProgress`, `SubResumePoint`, `SubState` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.runtime_consumption.esperanto.proto.RuntimeConsumptionService` | `GetConsumptionRecords` | native descriptor |
| `spotify.scrobble.esperanto.proto.Scrobble` | `GetIncognitoMode`, `PutIncognitoMode`, `SubIncognitoMode` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.search_esperanto.proto.OfflineSearchService` | `Query` | native descriptor |
| `spotify.show_esperanto.proto.ShowService` | `GetDecorate`, `GetShow`, `GetShowEpisode`, `GetUnfinishedEpisodes`, `MarkShowAsPlayed`, `SubDecorate`, `SubShow`, `SubShowEpisode`, `SubUnfinishedEpisodes` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.storage.esperanto.proto.Storage` | `DeleteExpiredItems`, `DeleteItem`, `DeleteUnlockedItems`, `GetCacheSizeLimit`, `GetFileRanges`, `GetStats`, `SetCacheSizeLimit` | installed xpui/xpui-modules.js, native descriptor |
| `spotify.stream_reporting_esperanto.proto.StreamReportingService` | `CurrentTimestamp`, `StreamEnd`, `StreamInitiate`, `StreamProgress`, `StreamSeek`, `StreamStart`, `StreamStartV2` | installed xpui/xpui-modules.js |
| `spotify.your_library_esperanto.proto.YourLibraryService` | `All`, `AlsoAvailableOffline`, `Contains`, `Decorate`, `GetKeys`, `IsCurated`, `MovePin`, `Pin`, `StreamAll`, `StreamAlsoAvailableOffline`, `StreamContains`, `StreamDecorate`, `StreamIsCurated`, `UnPin` | installed xpui/xpui-modules.js |
| `spotify.your_library_tags_esperanto.proto.YourLibraryTagsService` | `StreamYourLibraryTags` | installed xpui/xpui-modules.js |

</details>

<details>
<summary>Embedded file descriptors (849)</summary>

| File | Package | Top-level definitions |
|---|---|---|
| `ad_break_context.proto` | `spotify.player.proto` | `AdBreakContext` |
| `ad_capabilities.proto` | `spotify.ads.esperanto.proto` | `CapabilityRequest`, `Capability`, `Capabilities` |
| `ad_common.proto` | `spotify.ads.esperanto.proto` | `UrlArray`, `AdMedia`, `Ad`, `AdSlotEvent`, `AdSlot`, `AdQueue`, `SurveyQuestionOption`, `SurveyQuestion`, `SurveyTransparencyContent`, `Survey`, `CanvasImage`, `CanvasVideo`, `ItemPrice`, `ItemSalePrice`, `CarouselItem`, `Carousel`, `Companion`, `SKAdNetworkData`, `Verification`, `AdFormat`, `SurveyQuestionType` |
| `ad_detection.proto` | `spotify.ad_detection.impl.ad_detection_proto` | `DetectRequest`, `DetectResponse`, `MapTimestampRequest`, `CreatorTimestamp`, `MapTimestampResponse`, `SubscribeCreatorToUserTimestampRequest`, `UserTimestamp`, `SubscribeCreatorToUserTimestampResponse`, `SubscribeUserToCreatorTimestampRequest`, `SubscribeUserToCreatorTimestampResponse`, `DetectStatus`, `Precision`, `CreatorTimestampType`, `AdDetection` |
| `ad_events.proto` | `spotify.ads.esperanto.proto` | `EventRequest`, `EventResponse`, `UnmanagedEventRequest`, `SubEventRequest`, `AdEvent`, `SubEventResponse`, `Events` |
| `ad_formats.proto` | `spotify.ads.esperanto.proto` | `SubFormatRequest`, `SubFormatResponse`, `Formats` |
| `ad_logic_ad_response.proto` | `spotify.ads.proto` | `AdPodResponse`, `AdDecisionTree`, `SlotRealtimeDecisions`, `AdlogicAd`, `AdlogicAdList`, `StringList`, `TrackingEvents`, `CoverArt`, `AudioFile`, `VideoFile`, `DisplayFile`, `UriFile`, `Decision` |
| `ad_opportunity.proto` | `spotify.ads.esperanto.proto` | `PostEventRequest`, `PostEventResponse`, `PostEventV2Request`, `PostEventV2Response`, `AdOpportunity` |
| `ad_preview.proto` | `spotify.ads.esperanto.proto` | `PreviewRequest`, `PreviewResponse`, `Preview` |
| `ad_settings.proto` | `spotify.ads.esperanto.proto` | `SlotSettings`, `GetAllSlotSettingsRequest`, `GetSlotSettingsRequest`, `UpdateAdServerEndpointRequest`, `UpdateTimeIntervalRequest`, `UpdateSlotEnabledRequest`, `GetRequestHeadersRequest`, `SetRequestHeadersRequest`, `DeleteRequestHeadersRequest`, `UpdateAdStateEndpointRequest`, `GetSlotSettingsResponse`, `UpdateSlotResponse`, `GetRequestHeadersResponse`, `Settings` |
| `ad_slots.proto` | `spotify.ads.esperanto.proto` | `GetSlotsRequest`, `GetSlotsResponse`, `GetSlotRequest`, `GetSlotResponse`, `CreateSlotRequest`, `CreateSlotResponse`, `SubSlotRequest`, `SubSlotResponse`, `PrepareSlotRequest`, `PrepareSlotResponse`, `TriggerSlotRequest`, `TriggerSlotResponse`, `ClearSlotRequest`, `Slots` |
| `ad_state.proto` | `spotify.ads.esperanto.proto` | `AdStateValue`, `StateRequest`, `StateResponse`, `PutStateRequest`, `State` |
| `ad_targeting.proto` | `spotify.ads.esperanto.proto` | `TargetingResponse`, `GetTargetingRequest`, `GetAllTargetingRequest`, `PutTargetingRequest`, `DeleteTargetingRequest`, `Targeting` |
| `ad_testing.proto` | `spotify.ads.esperanto.proto` | `AddPlaytimeRequest`, `SetAlwaysEligibleForAdBreakRequest`, `InsertAdsRequest`, `InsertAdsResponse`, `Testing` |
| `ads.proto` | `spotify.ads.esperanto.proto` | `GetAdsRequest`, `GetAdsResponse`, `Ads` |
| `also_available_offline_request.proto` | `spotify.your_library.proto` | `AlsoAvailableOfflineRequest` |
| `also_available_offline_response.proto` | `spotify.your_library.proto` | `AlsoAvailableOfflineSuggestion`, `AlsoAvailableOfflineDebugStats`, `AlsoAvailableOfflineData`, `AlsoAvailableOfflineError`, `AlsoAvailableOfflineResponse`, `AlsoAvailableOfflineSuggestionType` |
| `apiv1.proto` | `spotify.offline.proto` | `ListDevicesRequest`, `ListDevicesResponse`, `PutDeviceRequest`, `BasicDeviceRequest`, `GetDeviceResponse`, `OfflineEnableDeviceRequest`, `OfflineEnableDeviceResponse`, `ListResourcesRequest`, `ListResourcesResponse`, `WriteResourcesRequest`, `ResourcesUpdate`, `DeltaResourcesRequest`, `DeltaResourcesResponse`, `GetResourceRequest`, `GetResourceResponse`, `WriteResourcesDetailsRequest`, `ListDevicesWithResourceRequest`, `ListDevicesWithResourceResponse`, `FetchStrategy`, `PollStrategy`, `SubStrategy`, `ClearUserDataRequest`, `ClearDeltaResourcesRequest`, `DeleteResourceListRequest`, `LicenseStatusRequest`, `LicenseStatusResponse` |
| `associations_extension.proto` | `spotify.assoc.v1` | `Assoc`, `EntityAssociations` |
| `attributes_cache.proto` | `spotify.playlist_cache.proto` | `CachedPlaylistAttributes` |
| `audio.proto` | `spotify.contentagnostic.v2` | `Audio` |
| `audio_associations.proto` | `spotify.bumblebee.audio_associations.v1` | `AudioAssociations`, `AudioAssociation`, `ImageGroup`, `Image` |
| `audio_attributes_v2.proto` | `spotify.playlistmixing.extensions.audio_attributes.v2` | `AudioAttributes` |
| `audio_files_extension.proto` | `spotify.extendedmetadata.audiofiles` | `NormalizationParams`, `ExtendedAudioFile`, `AudioFilesExtensionResponse` |
| `audio_format.proto` | `spotify.stream_reporting_esperanto.proto` | `AudioFormat` |
| `audio_sink_chain_wrapper_esperanto.proto` | `spotify.audio_sink_chain_wrapper_esperanto.proto` | `AudioFormat`, `PcmAudioBuffer`, `CreateRequest`, `CreateResponse`, `DestroyRequest`, `WriteRequest`, `WriteResponse`, `ReadRequest`, `ReadResponse`, `FlushRequest`, `EofRequest`, `EofResponse`, `SampleFormat`, `ChannelConfig` |
| `audiobooks_common_capping.proto` | `spotify.player.proto` | `AudiobooksCommonCapping` |
| `autodownload_backend_service.proto` | `spotify.autodownloadservice.v1.proto` | `Identifiers`, `Settings`, `SetSettingsRequest`, `GetSettingsRequest`, `GetSettingsResponse`, `ShowRequest`, `ReplaceIdentifiersRequest`, `PendingItem` |
| `autodownload_decorated_followed_shows.proto` | `spotify.autodownload_esperanto.proto` | `DecoratedFollowedShowsRequest`, `DecoratedShow`, `DecoratedFollowedShowsResponse` |
| `autodownload_disable_all_enabled_shows.proto` | `spotify.autodownload_esperanto.proto` | `DisableAllEnabledShowsRequest`, `DisableAllEnabledShowsResponse` |
| `autodownload_enabled_shows_count.proto` | `spotify.autodownload_esperanto.proto` | `EnabledShowsCountRequest`, `EnabledShowsCountResponse` |
| `autodownload_episode_limit.proto` | `spotify.autodownload_esperanto.proto` | `EpisodeLimitRequest`, `EpisodeLimitResponse` |
| `autodownload_set_episode_limit.proto` | `spotify.autodownload_esperanto.proto` | `SetEpisodeLimitRequest`, `SetEpisodeLimitResponse` |
| `autodownload_set_show_state.proto` | `spotify.autodownload_esperanto.proto` | `SetShowStateRequest`, `SetShowStateResponse` |
| `autodownload_show_state.proto` | `spotify.autodownload_esperanto.proto` | `ShowStateRequest`, `ShowStateResponse` |
| `automix_mode.proto` | `spotify.automix.proto` | `AutomixMode`, `AutomixStyle` |
| `autoplay_context_request.proto` | `spotify.player.proto` | `AutoplayContextRequest` |
| `autoplay_node.proto` | `spotify.player.proto` | `AutoplayNode` |
| `autopodcast_familiar_show_request.proto` | `spotify.player.proto` | `AutopodcastFamiliarShowRequest`, `MediaType` |
| `auxiliary_media.proto` | `spotify.playlist.proto` | `TtsMedia`, `AudioUrlMedia`, `IsNarration`, `AuxiliaryMediaMetadata`, `Media`, `SkipPoint`, `AuxiliaryMediaItem`, `ListItemFade`, `AuxiliaryMediaItems`, `AuxiliaryMedia` |
| `bucket_data_sync_state.proto` | `spotify.collection.proto` | `BucketDataSyncState` |
| `cache_hint.proto` | `spotify.playlist.messages` | `CacheHint` |
| `campfire/campfire_client.proto` | `campfire.v1` | `RetrieveLatestNodeRequest`, `RetrieveLatestNodeResponse`, `LatestNodeResponse`, `RetrieveNodeRequest`, `RetrieveNodeResponse`, `Hierarchy`, `HierarchyMetadata`, `HierarchyStatistic`, `Group`, `GroupMember`, `GroupDetail`, `ClientNode`, `NodeIdentifier`, `NodeStatistic`, `ClientEventCapture`, `MessagePayload`, `ContributionMetadata`, `ContributionPayload`, `ActivityPayload`, `ClientNodeRequest`, `ClientNodeResponse`, `NodeRequest`, `HierarchyRequest`, `MessageRequest`, `AddContribution`, `AddContributionForGroup`, `HierarchyUserParticipationStatus`, `HierarchySource`, `GroupMemberStatus`, `HierarchyUserRole`, `ModerationStatus` |
| `campfire/campfire_common.proto` | `campfire.v1` | `HierarchyIdentifier`, `NodeClass`, `FreeText`, `TrackedShare`, `OffplatformTrackedShare`, `EventCapture`, `AgentContext`, `HierarchyClass`, `ContributionClass`, `ActivityClass`, `HierarchyType`, `HierarchyStatus`, `SortOrder` |
| `canvaz.proto` | `spotify.canvaz.cache` | `Artist`, `EntityCanvazResponse`, `Type` |
| `cappingapi.proto` | `spotify.cappingapi.v1` | `Quota`, `QuotaInfo`, `ConsumptionEvent`, `FeatureConsumption`, `GetQuotasRequest`, `GetQuotasResponse`, `PutConsumptionRequest`, `PutConsumptionResponse`, `RefreshFeaturesPayload`, `Feature` |
| `cast_messages.proto` | `spotify.connect.discovery.cast.esperanto.proto` | `Response`, `PutDeviceRequest`, `DeviceId`, `ConnectRequest`, `ConnectionErrorRequest` |
| `chapter_specifics.proto` | `spotify.chapterspecifics` | `ChapterSpecifics`, `EpisodeMetadata` |
| `client-tts.proto` | `spotify.narration.proto` | `TtsRequest`, `TtsResponse`, `ClientTtsService` |
| `client_restrictions.proto` | `spotify.clientrestrictions.v1` | `ExternalAccessoryDescription`, `RestrictedIntegration`, `RestrictedIntegrationsPayload`, `RestrictedIntegrationsRequest`, `RestrictedIntegrationsResponse`, `CachedRestrictedIntegrationsResponse` |
| `client_restrictions_extension.proto` | `spotify.clientrestrictions.v1` | `ClientRestrictionsExtensionData` |
| `client_taxonomy.proto` | `spotify.ubiquity.v1` | `Output`, `Device`, `ClientTaxonomy`, `OutputCategory`, `OutputConnectionType` |
| `client_token.proto` | `spotify.clienttoken.serialization.v0` | `TokenDomain`, `ClientToken` |
| `client_update.proto` | `spotify.desktopupdate.proto` | `UpgradeSignedPart`, `UpgradeRequiredMessage`, `UpdateQueryResponse`, `ClientUpgradeType` |
| `collection/album_collection_state.proto` | `spotify.cosmos_util.proto` | `AlbumCollectionState` |
| `collection/artist_collection_state.proto` | `spotify.cosmos_util.proto` | `ArtistCollectionState` |
| `collection/episode_collection_state.proto` | `spotify.cosmos_util.proto` | `EpisodeCollectionState` |
| `collection/show_collection_state.proto` | `spotify.cosmos_util.proto` | `ShowCollectionState` |
| `collection/track_collection_state.proto` | `spotify.cosmos_util.proto` | `TrackCollectionState` |
| `collection2v2.proto` | `spotify.collection.proto.v2` | `PageRequest`, `CollectionItem`, `PageResponse`, `DeltaRequest`, `DeltaResponse`, `WriteRequest`, `InitializedRequest`, `InitializedResponse` |
| `collection_add_remove_items_request.proto` | `spotify.collection_cosmos.proto` | `CollectionAddRemoveItem`, `CollectionAddRemoveItemsRequest`, `CollectionAddRemoveItemsResponse` |
| `collection_ban_global.proto` | `spotify.collection_esperanto.proto` | `BanGlobalRequest`, `UnbanGlobalRequest` |
| `collection_ban_in_play_context.proto` | `spotify.collection_esperanto.proto` | `BanInPlayContextRequest`, `UnbanInPlayContextRequest` |
| `collection_ban_request.proto` | `spotify.collection_cosmos.proto` | `CollectionBanRequest`, `CollectionBanResponse` |
| `collection_contains_request.proto` | `spotify.collection_cosmos.proto` | `CollectionContainsRequest`, `CollectionContainsResponse` |
| `collection_decorate_request.proto` | `spotify.collection_cosmos.proto` | `CollectionDecorateRequest`, `CollectionDecorateResponse` |
| `collection_decoration_policy.proto` | `spotify.collection_cosmos.proto` | `CollectionArtistDecorationPolicy`, `CollectionAlbumDecorationPolicy`, `CollectionTrackDecorationPolicy`, `CollectionShowDecorationPolicy`, `CollectionEpisodeDecorationPolicy` |
| `collection_get_artist_view_request.proto` | `spotify.collection_cosmos.proto` | `CollectionGetArtistViewRequest`, `CollectionGetArtistViewResponse` |
| `collection_get_bucket_sync_status_request.proto` | `spotify.collection_cosmos.proto` | `CollectionGetBucketSyncStatusRequest`, `CollectionGetBucketSyncStatusResponse` |
| `collection_index.proto` | `spotify.collection.proto` | `IndexRepairerState`, `AddTime`, `CollectionTrackEntry`, `CollectionAlbumEntry`, `CollectionShowEntry`, `CollectionBookEntry`, `CollectionArtistEntry`, `CollectionAuthorEntry`, `CollectionEpisodeEntry` |
| `collection_item.proto` | `spotify.collection_cosmos.proto` | `CollectionTrack`, `CollectionAlbum`, `CollectionArtist`, `CollectionShow`, `CollectionEpisode` |
| `collection_itemdata.proto` | `spotify.collection.itemdata` | `ItemData`, `PlayContextBansItemData`, `BanItemData`, `ArtistBanItemData`, `BanIntentItemData`, `SnoozedRecsItemData`, `RatingItemData`, `PageMatchItemData`, `HiddenFromHomeItemData`, `IsbnScanItemData` |
| `collection_offline_track_list_request.proto` | `spotify.collection_cosmos.proto` | `CollectionOfflineTrackListRequest`, `CollectionOfflineTrackListResponse` |
| `collection_platform_items.proto` | `spotify.collection_platform.proto` | `CollectionPlatformItem` |
| `collection_platform_requests.proto` | `spotify.collection_platform.proto` | `CollectionPlatformItemsRequest`, `CollectionSet` |
| `collection_platform_responses.proto` | `spotify.collection_platform.proto` | `CollectionPlatformContainsResponse`, `Status`, `CollectionPlatformEsperantoContainsResponse`, `CollectionPlatformEsperantoItemsResponse` |
| `collection_snoozed_recs.proto` | `spotify.collection_esperanto.proto` | `IsSnoozedRequest`, `IsSnoozedResponse`, `UnsnoozeRequest`, `SnoozeRequest` |
| `collection_status.proto` | `spotify.collection_esperanto.proto` | `Status` |
| `collection_unplayed.proto` | `spotify.collection_esperanto.proto` | `CollectionUnplayedRequest`, `CollectionUnplayedResponse` |
| `comment_count.proto` | `spotify.extendedmetadata.commentcount.v1` | `GetCommentCountMetadataResponse` |
| `common_capping.proto` | `spotify.common_capping.impl.common_capping_proto` | `IsCappedRequest`, `IsCappedResponse`, `ReportConsumptionRequest`, `ObserveCapStateChangedRequest`, `ObserveCapStateChangedResponse`, `ObserveCapInfoRequest`, `ObserveCapInfoResponse`, `GetCapInfoRequest`, `GetCapInfoResponse`, `RefreshRequest`, `GetQuotaStateAtRequest`, `GetQuotaStateAtResponse`, `Feature`, `QuotaState`, `CommonCapping` |
| `common_messages.proto` | `spotify.connect.esperanto.proto` | `StatusResponse`, `LoggingParams`, `Status` |
| `companion_ad.proto` | `spotify.ads.proto` | `SurveyQuestionOption`, `SurveyQuestion`, `SurveyTransparencyContent`, `Survey`, `CanvasImage`, `CanvasVideo`, `ItemPrice`, `ItemSalePrice`, `CarouselImage`, `CarouselVideo`, `CarouselItem`, `Carousel`, `Companion`, `CompanionAd`, `SurveyQuestionType` |
| `companion_content_extension.proto` | `spotify.figs.companion_content.v0` | `CompanionContent`, `Component`, `ImageComponent`, `LoopingVideoComponent` |
| `concept_metadata.proto` | `spotify.conceptmetadata` | `Concept`, `ConceptClass` |
| `concert_campaign_user_state_extension.proto` | `spotify.concertcampaignextension.v1` | `ConcertCampaignUserState`, `CampaignUserStateReserved`, `CampaignUserStateFCFS`, `CampaignUserStateWaitlistable`, `CampaignUserStateWaitlisted`, `CampaignUserStateIneligible`, `CampaignUserStatePurchased`, `CampaignUserStatePurchase`, `CampaignUserStateIneligibleReason` |
| `concert_extension.proto` | `spotify.concerts.metadata.v1` | `DateWithOffset`, `ConcertTimes`, `Concert` |
| `concert_location_extension.proto` | `spotify.concerts.metadata.v1` | `PhysicalLocation`, `Location` |
| `concert_marketing_extension.proto` | `spotify.concerts.metadata.v1` | `Image`, `ImageGroup`, `Marketing` |
| `concert_performers_extension.proto` | `spotify.concerts.metadata.v1` | `Performer`, `PerformerGroup`, `Act` |
| `connect-service.proto` | `spotify.connectstate` | `PutStateRequestWrapper`, `SetDeviceInactiveRequest`, `UpdateLocalDevicesRequest`, `UpdateLocalDevicesResponse` |
| `connect.proto` | `spotify.connectstate` | `ClusterUpdate`, `Device`, `Cluster`, `PutStateRequest`, `PrivateDeviceInfo`, `DeviceInfo`, `AudioOutputDeviceInfo`, `Capabilities`, `CapabilitySupportDetails`, `ConnectCommandOptions`, `ConnectLoggingParams`, `LogoutCommand`, `SetVolumeCommand`, `RenameCommand`, `ProxyTransferCommand`, `ProxyWakeCommand`, `SetBackendMetadataCommand`, `LocalDevice`, `WakeOnLanInfo`, `AddDeviceToGroupCommand`, `AddDeviceToGroupCommandResponse`, `RemoveDeviceFromGroupCommand`, `RemoveDeviceFromGroupCommandResponse`, `DeleteGroupCommand`, `DeleteGroupCommandResponse`, `ResyncGroupDataCommand`, `ResyncGroupDataCommandResponse`, `AudioOutputDeviceType`, `AudioOutputCategory`, `PutStateReason`, `MemberType`, `ClusterUpdateReason` |
| `connect_group.proto` | `spotify.connectgroup.v1` | `GroupData`, `GroupedDevice`, `GroupableDevice`, `GroupDeviceVolume`, `PutGroupDataRequest`, `PutGroupDataResponse`, `PutGroupDeviceVolumeRequest`, `PutGroupDeviceVolumeResponse`, `DeleteGroupInstancesRequest`, `DeleteGroupInstancesResponse` |
| `connect_group_event.proto` | `spotify.connectgroup.v1` | `GroupDataEvent`, `GroupDeviceVolumeEvent`, `GetAllGroupDataResponse`, `KnownGroupInstance`, `RefreshGroupDataRequest` |
| `connect_messages.proto` | `spotify.connect.esperanto.proto` | `LogoutRequest`, `TransferRequest`, `InstallSpotifyRequest`, `InstallSpotifyStatusUpdate`, `SendCommandRequest`, `SendCommandResponse`, `PullRequest`, `StateRequest`, `OutputPeripheral`, `StateResponse`, `NotifyAudioRouteChangedRequest`, `RenameRequest`, `SetPreferredIncarnationRequest`, `ClusterUpdateAfterBackendConnectionResponse`, `GroupData`, `GroupDevice`, `GroupDeviceIdentifiers`, `GroupDeviceVolume`, `GroupCapabilities`, `GroupDeviceCapabilities`, `CreateStaticGroupRequest`, `CreateStaticGroupResponse`, `RenameGroupRequest`, `RenameGroupResponse`, `AddDeviceToGroupRequest`, `AddDeviceToGroupResponse`, `RemoveDeviceFromGroupRequest`, `RemoveDeviceFromGroupResponse`, `DeleteGroupRequest`, `DeleteGroupResponse`, `InstallSpotifyState`, `DiscoveryMethod`, `OutputPeripheralCategory`, `OutputPeripheralConnectionType`, `LoginType`, `ProtocolChoiceReason`, `GroupingStatus` |
| `connectivity.proto` | `spotify.clienttoken.data.v0` | `ConnectivitySdkData`, `PlatformSpecificData`, `ApplicationSpecificData`, `NativeAndroidData`, `AndroidScreen`, `NativeIOSData`, `NativeDesktopMacOSData`, `NativeDesktopWindowsData`, `NativeDesktopLinuxData`, `NativeTVOSData`, `NativeWatchOSData`, `NativeCoreSpeakerData` |
| `consumption_experience_trait.proto` | `spotify.contentagnostic.v2` | `ConsumptionExperienceTrait`, `Genre`, `ContentRating`, `Format` |
| `contains_request.proto` | `spotify.playlist.cosmos.proto` | `ContainsRequest`, `ContainsResponse` |
| `content_capability_trait.proto` | `spotify.contentagnostic.v2` | `Capability`, `Allowed`, `Disallowed`, `Restricted`, `Irrelevant`, `ContentCapabilityTrait`, `ProductStateCondition` |
| `content_experience_trait.proto` | `spotify.contentagnostic.v2` | `ContentExperienceTrait` |
| `content_markers_extension.proto` | `spotify.scdcontentmarkers` | `ContentMarkers`, `ContentMarker`, `ContentMarkerType` |
| `content_type_trait.proto` | `spotify.contentagnostic` | `ContentTypeTrait`, `ContentType` |
| `context.proto` | `spotify.player.proto` | `Context` |
| `context_application.proto` | `spotify.event_sender.proto` | `Application` |
| `context_application_desktop.proto` | `spotify.event_sender.proto` | `ApplicationDesktop` |
| `context_client_context_id.proto` | `spotify.event_sender.proto` | `ClientContextId` |
| `context_client_id.proto` | `spotify.event_sender.proto` | `ClientId` |
| `context_device.proto` | `spotify.event_sender.proto` | `Device` |
| `context_device_desktop.proto` | `spotify.event_sender.proto` | `DeviceDesktop` |
| `context_index.proto` | `spotify.player.proto` | `ContextIndex` |
| `context_installation_id.proto` | `spotify.event_sender.proto` | `InstallationId` |
| `context_monotonic_clock.proto` | `spotify.event_sender.proto` | `MonotonicClock` |
| `context_node.proto` | `spotify.player.proto` | `ContextNode` |
| `context_os.proto` | `spotify.event_sender.proto` | `Os` |
| `context_page.proto` | `spotify.player.proto` | `ContextPage` |
| `context_player_options.proto` | `spotify.player.proto` | `ContextPlayerOptions`, `ContextPlayerOptionOverrides` |
| `context_processor.proto` | `spotify.player.proto` | `ContextProcessor` |
| `context_sdk.proto` | `spotify.event_sender.proto` | `Sdk` |
| `context_stack.proto` | `spotify.playback_platform.context.v1` | `ContextStack` |
| `context_time.proto` | `spotify.event_sender.proto` | `Time` |
| `context_track.proto` | `spotify.player.proto` | `ContextTrack` |
| `context_view.proto` | `spotify.player.context_view.proto` | `ContextView` |
| `context_view_cyclic_list.proto` | `spotify.player.context_view.cyclic_list.proto` | `Instance`, `Patch`, `CyclicEntryKeyList` |
| `context_view_entry.proto` | `spotify.player.context_view.proto` | `Entry` |
| `context_view_entry_key.proto` | `spotify.player.context_view.proto` | `EntryKey` |
| `counters_storage.proto` | `spotify.pending_events.proto` | `CountersStorage`, `CountersArray` |
| `cuepoints.proto` | `spotify.automix.proto` | `Cuepoint`, `Cuepoints`, `Origin` |
| `curated_playlist.proto` | `spotify.curation_state.proto` | `CuratedPlaylistItem`, `CuratedPlaylist` |
| `curation_experience_trait.proto` | `spotify.contentagnostic.v2` | `CurationExperienceTrait` |
| `debug_tracer.proto` | `spotify.debug_tracer.proto` | `DebugTracerRequest`, `DebugTracerPlaybackIdRequest`, `DebugTracerStreamHandleRequest`, `DebugTracerResponse`, `DebugTracePoint`, `DebugTracerService` |
| `decorate_request.proto` | `spotify.show_cosmos.decorate_request.proto` | `Show`, `Episode`, `Response` |
| `device_state.proto` | `spotify.ads.esperanto.proto` | `SetDeviceVolumeRequest`, `SetDeviceVolumeResponse`, `GetDeviceVolumeRequest`, `GetDeviceVolumeResponse`, `DeviceState` |
| `devices.proto` | `spotify.connectstate.devices` | `DeviceAlias`, `DeviceType` |
| `display_segments_esperanto_request.proto` | `spotify.display_segments_esperanto.proto.v1` | `SegmentsRequest`, `TrackDecorationPolicy`, `SegmentsPolicy`, `EpisodeDecorationPolicy` |
| `display_segments_esperanto_response.proto` | `spotify.display_segments_esperanto.proto.v1` | `SegmentsResponse`, `EpisodeSegments`, `DecoratedSegment` |
| `display_segments_extension.proto` | `spotify.displaysegments.v1` | `DisplaySegmentsExtension`, `DisplaySegment`, `MusicAndTalkDecoration`, `PodcastChaptersDecoration`, `SegmentType` |
| `endpoint/read_end_request.proto` | `spotify.read_reporting_esperanto.proto` | `ReadEndRequest` |
| `endpoint/read_interval.proto` | `spotify.read_reporting_esperanto.proto` | `ReadInterval` |
| `endpoint/read_start_request.proto` | `spotify.read_reporting_esperanto.proto` | `ReadStartRequest` |
| `endpoint/read_start_response.proto` | `spotify.read_reporting_esperanto.proto` | `ReadStartResponse` |
| `endpoint/read_update_request.proto` | `spotify.read_reporting_esperanto.proto` | `ReadUpdateRequest` |
| `endpoint/reason_end.proto` | `spotify.read_reporting_esperanto.proto` | `ReasonEnd` |
| `endpoint/reason_start.proto` | `spotify.read_reporting_esperanto.proto` | `ReasonStart` |
| `endpoint/status_code.proto` | `spotify.read_reporting_esperanto.proto` | `StatusCode` |
| `endpoint/status_response.proto` | `spotify.read_reporting_esperanto.proto` | `StatusResponse` |
| `entity_capping.proto` | `spotify.capping.metadata.v1` | `EntityCapping` |
| `entity_extension_data.proto` | `spotify.extendedmetadata` | `EntityExtensionDataHeader`, `EntityExtensionData`, `PlainListAssoc`, `AssocHeader`, `Assoc` |
| `entity_popularity.proto` | `spotify.contentpopularity.v1` | `EntityPopularity` |
| `entity_type_trait.proto` | `spotify.contentagnostic.v2` | `EntityTypeTrait`, `EntityType` |
| `episode_ad_placements_extension.proto` | `spotify.fingerprintfilterservice` | `EpisodeAdPlacements`, `AdPlacement` |
| `episode_transcripts_extension.proto` | `spotify.corex.transcripts.metadata` | `EpisodeTranscript`, `Transcript` |
| `es_ad_break_context.proto` | `spotify.player.esperanto.proto` | `AdBreakContext` |
| `es_add_pending_event_request.proto` | `spotify.pending_events.esperanto.proto` | `AddPendingEventRequest` |
| `es_add_pending_event_response.proto` | `spotify.pending_events.esperanto.proto` | `AddPendingEventResponse` |
| `es_add_to_queue_request.proto` | `spotify.player.esperanto.proto` | `AddToQueueRequest` |
| `es_auth_blob.proto` | `spotify.connectivity.auth.common.esperanto.proto` | `AuthBlob` |
| `es_auth_storage_result.proto` | `spotify.connectivity.auth.storage.esperanto.proto` | `AuthStorageResult` |
| `es_automix.proto` | `spotify.automix.esperanto.proto` | `TrackPair`, `GetComputedTransitionsRequest`, `ComputedPreset`, `ComputedTransition`, `ComputedTransitions`, `GetComputedTransitionsResponse`, `GetStylesForPresetIdRequest`, `PresetIdStyle`, `GetStylesForPresetIdResponse`, `GetVolumeStylesForVolumeStyleIdRequest`, `GetVolumeStylesForVolumeStyleIdResponse`, `GetEqStylesForEqStyleIdRequest`, `GetEqStylesForEqStyleIdResponse`, `GetFilterFxStylesForFilterFxStyleIdRequest`, `GetFilterFxStylesForFilterFxStyleIdResponse`, `GetFxStylesForFxStyleIdRequest`, `GetFxStylesForFxStyleIdResponse`, `GetMaxTransitionLengthResponse`, `GetBeatmatchOptOutTransitionDurationResponse`, `AdjustTransitionRecipeForItemSpeedsRequest`, `AdjustTransitionRecipeForItemSpeedsResponse`, `GetJogwheelBlocksRequest`, `GetJogwheelBlocksResponse`, `GetLoopingBlocksRequest`, `GetLoopingBlocksResponse`, `Automix` |
| `es_automix_bundle.proto` | `spotify.automix.bundle.esperanto.proto` | `GetMixingBundleRequest`, `GetMixingBundleResponse`, `AutomixBundle` |
| `es_ble_debug.proto` | `spotify.desktop.ble_debug_esperanto.proto` | `BleCapabilityBit`, `BleWriteOperation`, `BleDiscoveredDevice`, `BleReceivedWrite`, `BleAdvertisedDevice`, `BleAdvertisingState`, `BleScanningState`, `BleDebugState`, `ConnectionStatus`, `DeviceType`, `DeviceRange`, `CapabilitiesSource`, `BleDebug` |
| `es_client_token.proto` | `spotify.connectivity.auth.client_token.esperanto.proto` | `AcquireResult`, `AcquireFailure`, `ClientToken`, `EncryptedClientToken` |
| `es_command_options.proto` | `spotify.player.esperanto.proto` | `CommandOptions` |
| `es_commit_pending_event_request.proto` | `spotify.pending_events.esperanto.proto` | `CommitPendingEventRequest` |
| `es_commit_pending_event_response.proto` | `spotify.pending_events.esperanto.proto` | `CommitPendingEventResponse` |
| `es_connection_error.proto` | `spotify.connectivity.hosts.esperanto.proto` | `ConnectionError` |
| `es_connection_id.proto` | `spotify.connectivity.pubsub.esperanto.proto` | `ConnectionID` |
| `es_connection_quality_service.proto` | `spotify.connectivity.connection_quality.esperanto.proto` | `ConnectionQualityEvent`, `ConnectionQuality`, `ConnectionQualityService` |
| `es_connectivity_policy.proto` | `spotify.connectivity.esperanto.proto` | `PutRulesRequest`, `ForceOfflineRequest`, `SubStateRequest`, `GetStateRequest`, `GetRulesRequest`, `SetConnectionTypeRequest`, `PutRulesResponse`, `ForceOfflineResponse`, `GetStateResponse`, `GetRulesResponse`, `SetConnectionTypeResponse`, `ConnectionType`, `ConnectivityPolicy` |
| `es_context.proto` | `spotify.player.esperanto.proto` | `Context` |
| `es_context_page.proto` | `spotify.player.esperanto.proto` | `ContextPage` |
| `es_context_player_error.proto` | `spotify.player.esperanto.proto` | `ContextPlayerError` |
| `es_context_player_options.proto` | `spotify.player.esperanto.proto` | `ContextPlayerOptions`, `ContextPlayerOptionOverrides` |
| `es_context_player_state.proto` | `spotify.player.esperanto.proto` | `ContextIndex`, `PlaybackQuality`, `ContextPlayerState` |
| `es_context_resolve_mock.proto` | `spotify.player.esperanto.proto` | `ContextUpdate`, `RequestInfoRequest`, `RequestInfo`, `AutoplayRequestInfo`, `ContextResolveMock` |
| `es_context_track.proto` | `spotify.player.esperanto.proto` | `ContextTrack` |
| `es_delete_session.proto` | `spotify.player.esperanto.proto` | `DeleteSessionRequest`, `DeleteSessionResponse` |
| `es_desktop_audio_observer.proto` | `spotify.desktop.audio_observer_esperanto.proto` | `AudioDevice`, `AudioDevicesResponse`, `TerminalType`, `TransportType`, `DesktopAudioObserver` |
| `es_desktop_commands.proto` | `spotify.desktop.commands_esperanto.proto` | `SubscribeCommandRequest`, `SubscribeCommandResponse`, `DesktopCommand`, `DesktopCommands` |
| `es_desktop_issue_reporter.proto` | `spotify.desktop.issue_reporter_esperanto.proto` | `FileAttachment`, `SubmitIssueRequest`, `SubmitIssueResponse`, `DesktopIssueReporter` |
| `es_desktop_lifecycle.proto` | `spotify.desktop.lifecycle_esperanto.proto` | `NotifyViewLoadedRequest`, `DesktopLifecycle` |
| `es_desktop_local_jam.proto` | `spotify.desktop.local_jam.proto` | `LocalNetDevice`, `LocalBleDevice`, `NetworkInterface`, `StartScanRequest`, `ScanResponse`, `StopScanRequest`, `StopScanResponse`, `StartBroadcastRequest`, `StartBroadcastResponse`, `StartBleBroadcastRequest`, `StartBleBroadcastResponse`, `StopBroadcastRequest`, `StopBroadcastResponse`, `StopBleBroadcastRequest`, `StopBleBroadcastResponse`, `SubNetworkInterfacesRequest`, `NetworkInterfacesResponse`, `Tech`, `Type`, `IpVersion`, `DesktopLocalJam` |
| `es_desktop_login.proto` | `spotify.desktop.login_esperanto.proto` | `AuthorizationRequired`, `ConfirmationRequired`, `LoginError`, `LoginSuccess`, `LoginWithQRCodeRequest`, `LoginWithQRCodeResponse`, `LoginWithAccountsRequest`, `LoginWithAccountsResponse`, `LoginWithConnectRequest`, `LoginWithConnectResponse`, `ApproveConnectLoginRequest`, `ApproveConnectLoginResponse`, `LogoutRequest`, `LogoutResponse`, `GetSessionStateRequest`, `GetSessionStateResponse`, `DesktopLogin` |
| `es_desktop_logs.proto` | `spotify.desktop.logs_esperanto.proto` | `CopyLogsToTempFolderRequest`, `CopyLogsToTempFolderResponse`, `OpenFolderRequest`, `OpenFolderResponse`, `DesktopLogs` |
| `es_desktop_navigation.proto` | `spotify.desktop.navigation_esperanto.proto` | `SubscribeNavigationRequest`, `SubscribeDeepLinkRequest`, `NavigationEvent`, `DeepLinkEvent`, `DesktopNavigation` |
| `es_desktop_platform_info.proto` | `spotify.desktop.platform_info_esperanto.proto` | `Capabilities`, `EventSenderContextInformation`, `GetPlatformInfoRequest`, `PlatformInfoResponse`, `DesktopPlatformInfo` |
| `es_desktop_proxy_settings.proto` | `spotify.desktop.proxy_settings_esperanto.proto` | `ProxySettings`, `ProxyOption`, `DesktopProxySettings` |
| `es_desktop_remote_config.proto` | `spotify.desktop.remote_config_esperanto.proto` | `GetPropertiesResponse`, `GetConfigurationResponse`, `GetOverridesResponse`, `SetOverrideRequest`, `SetOverrideResponse`, `ClearOverridesResponse`, `DesktopRemoteConfig` |
| `es_desktop_super_connect.proto` | `spotify.desktop.super_connect.proto` | `GetOrchestratorTokenRequest`, `GetOrchestratorTokenResponse`, `PermissionEvent`, `PermissionRequest`, `PermissionResolved`, `ResolvePermissionRequest`, `ResolvePermissionResponse`, `SubscribePermissionRequestsRequest`, `GetVersionsRequest`, `GetVersionsResponse`, `CheckStudioInstalledRequest`, `CheckStudioInstalledResponse`, `OpenStudioRequest`, `OpenStudioResponse`, `CheckSandboxStatusRequest`, `CheckSandboxStatusResponse`, `RunSandboxSetupRequest`, `RunSandboxSetupEvent`, `InstallStudioRequest`, `InstallStudioEvent`, `GetKeepAwakeStatusRequest`, `GetKeepAwakeStatusResponse`, `DesktopSuperConnect` |
| `es_desktop_update.proto` | `spotify.desktop.update_esperanto.proto` | `SubscribeStatusResponse`, `TriggerDownloadRequest`, `TriggerDownloadResponse`, `ApplyUpdateRequest`, `ApplyUpdateResponse`, `UpdateState`, `DesktopUpdate` |
| `es_desktop_update_ui.proto` | `spotify.desktop.update_ui_esperanto.proto` | `UpdateTitlebarHeightRequest`, `UpdateTitlebarResponse`, `SetButtonsVisibilityRequest`, `SetButtonsVisibilityResponse`, `FieldState`, `UpdateMenuStateRequest`, `UpdateMenuStateResponse`, `MenuField`, `DesktopUpdateUi` |
| `es_desktop_version.proto` | `spotify.desktop.version_esperanto.proto` | `VersionInfoResponse`, `GetVersionRequest`, `BuildType`, `BuildConfiguration`, `Platform`, `CefRuntime`, `DesktopVersion` |
| `es_desktop_watchdog.proto` | `spotify.desktop.watchdog_esperanto.proto` | `SubscribeRequest`, `SubscribeResponse`, `PingRequest`, `DesktopWatchdog` |
| `es_desktop_window_state.proto` | `spotify.desktop.window_state_esperanto.proto` | `WindowStateResponse`, `SetWindowStateRequest`, `GetWindowStateResponse`, `WindowState`, `DesktopWindowState` |
| `es_desktop_zoom.proto` | `spotify.desktop.zoom_esperanto.proto` | `GetZoomLevelRequest`, `SetZoomLevelRequest`, `GetZoomLevelResponse`, `SetZoomLevelResponse`, `ZoomInRequest`, `ZoomInResponse`, `ZoomOutRequest`, `ZoomOutResponse`, `DesktopZoom` |
| `es_download.proto` | `spotify.download.esperanto.proto` | `FileStreamerId`, `RequestDataParams`, `RequestDataResponse`, `SegmentFileStreamerId`, `SegmentFileStreamerParams`, `SegmentRequestMetadata`, `RequestSegmentDataParams`, `FinishSegmentDataRequestParams`, `MediaManifestParams`, `MediaManifestResponse`, `ProgressiveFileStreamerParams`, `IsFileFullyCachedParams`, `GetFirstCachedSegmentIndexParams`, `GetFirstCachedSegmentIndexResponse`, `GetMediaHandleParams`, `GetMediaHandleIdResponse`, `VideoBandwidthResponse`, `Error`, `VerificationStatus`, `ContentType`, `ContentForm`, `MediaManifestRequestType`, `MediaManifestError`, `Download` |
| `es_encrypted_stored_credentials.proto` | `spotify.connectivity.auth.common.esperanto.proto` | `EncryptedStoredCredentials` |
| `es_get_error_request.proto` | `spotify.player.esperanto.proto` | `GetErrorRequest` |
| `es_get_play_history.proto` | `spotify.player.esperanto.proto` | `GetPlayHistoryRequest`, `GetPlayHistoryResponse` |
| `es_get_position_state.proto` | `spotify.player.esperanto.proto` | `GetPositionStateRequest`, `GetPositionStateResponse` |
| `es_get_queue_request.proto` | `spotify.player.esperanto.proto` | `GetQueueRequest` |
| `es_get_state_request.proto` | `spotify.player.esperanto.proto` | `GetStateRequest` |
| `es_host.proto` | `spotify.connectivity.hosts.esperanto.proto` | `Host` |
| `es_hosts.proto` | `spotify.connectivity.hosts.esperanto.proto` | `Hosts` |
| `es_ident.proto` | `spotify.connectivity.pubsub.esperanto.proto` | `Ident` |
| `es_ident_filter.proto` | `spotify.connectivity.pubsub.esperanto.proto` | `IdentFilter` |
| `es_image.proto` | `spotify.image.esperanto.proto` | `ImageData`, `Size`, `GetImageParams`, `CacheStatus`, `GetCacheStatusParams`, `SetProjectionMapParams`, `ImageSource`, `Image` |
| `es_issue_reporter.proto` | `spotify.issue_reporter.proto` | `FileAttachment`, `SubmitIssueRequest`, `SubmitIssueResponse`, `IssueReporter` |
| `es_key.proto` | `spotify.connectivity.pubsub.esperanto.proto` | `Key` |
| `es_local_files.proto` | `spotify.local_files_esperanto.proto` | `Item`, `Response`, `ResponseStatus`, `Range`, `Query`, `GetTracksRequest`, `GetTracksResponse`, `GetScannerStatusResponse`, `DefaultSource`, `Folder`, `GetSourcesResponse`, `MutateSourceResponse`, `MutateDefaultSourcesRequest`, `MutateSourcesResponse`, `LocalFiles` |
| `es_logging_params.proto` | `spotify.player.esperanto.proto` | `LoggingParams` |
| `es_media_prefetch.proto` | `spotify.prefetch_esperanto.proto` | `AudioPreviewResource`, `ProgressiveVideoResource`, `SegmentedVideoResource`, `Item`, `Future`, `UpdateFutureRequest`, `SegmentProfileSelectionConfig`, `EncryptionScheme`, `VideoCodecFamily`, `MediaPrefetch` |
| `es_notifications_service.proto` | `spotify.desktop.notifications.proto` | `ToastAction`, `NotificationRequest`, `NotificationResponse`, `AvailabilityResponse`, `NotificationAvailability`, `NotificationsService` |
| `es_offline.proto` | `spotify.offline_esperanto.proto` | `GetOfflinePlaybackStatusResponse`, `SetOfflinePlaybackAllowedRequest`, `SetOfflinePlaybackAllowedResponse`, `Error`, `ContextInfoPolicy`, `Context`, `Item`, `ContextInfo`, `DeviceKey`, `DeviceContextInfo`, `GetContextsRequest`, `GetContextsResponse`, `GetItemsRequest`, `GetItemsResponse`, `DownloadRequest`, `DownloadResponse`, `Progress`, `GetContextsProgressRequest`, `ContextProgress`, `ContextsProgressResponse`, `TotalProgressResponse`, `Capabilities`, `Device`, `GetDevicesResponse`, `DevicesWithContextRequest`, `DevicesWithContextResponse`, `OfflineLicenseInfo`, `GetEstimatedStorageIncreaseRequest`, `EstimatedStorageIncrease`, `GetListsContainingItemRequest`, `GetListsContainingItemResponse`, `LicenseStatusResponse`, `PrepareResourceRequest`, `PrepareResourceResponse`, `HasResourceRequest`, `HasResourceResponse`, `RemoveResourceRequest`, `BulkRedownloadPreflightResponse`, `BulkRedownloadStartResponse`, `BulkRedownloadCancelResponse`, `ContextAdded`, `ResourceType`, `Offline` |
| `es_offline_debug.proto` | `spotify.offline_esperanto.proto` | `KeyStatsResponse`, `SetLinkAdditionalExtensionsRequest`, `OfflineDebug` |
| `es_offline_playable_cache.proto` | `spotify.offline_playable_cache_esperanto.proto` | `Item`, `Concept`, `Response`, `ResponseStatus`, `Range`, `SmartSortParameters`, `Query`, `GetTracksRequest`, `GetTracksResponse`, `RemoveTrackRequest`, `RemoveTrackResponse`, `HasEnoughTracksRequest`, `HasEnoughTracksResponse`, `OfflinePlayableCache` |
| `es_offline_plugin.proto` | `spotify.offline.proto.v2` | `TargetFormat`, `Metadata`, `IdentifyCommand`, `IdentifyResponse`, `DownloadCommand`, `DownloadResponse`, `StopDownloadCommand`, `RemoveCommand`, `PluginMetadata`, `LinkType`, `ResourceType`, `OfflinePlugin` |
| `es_optional.proto` | `spotify.player.esperanto.proto` | `OptionalInt64`, `OptionalDouble`, `OptionalBoolean` |
| `es_pause.proto` | `spotify.player.esperanto.proto` | `PauseRequest` |
| `es_pauseresume_origin.proto` | `spotify.player.esperanto.proto` | `PauseResumeOrigin` |
| `es_pending_events_error.proto` | `spotify.pending_events.esperanto.proto` | `PendingEventsError` |
| `es_play.proto` | `spotify.player.esperanto.proto` | `PlayRequest`, `PlayPreparedRequest` |
| `es_play_as_next_in_queue_request.proto` | `spotify.player.esperanto.proto` | `PlayAsNextInQueueRequest` |
| `es_play_options.proto` | `spotify.player.esperanto.proto` | `PlayOptions` |
| `es_play_origin.proto` | `spotify.player.esperanto.proto` | `PlayOrigin` |
| `es_playability.proto` | `spotify.playability.esperanto.proto` | `GetLocallyPlayableRequest`, `LocallyPlayable`, `GetLocallyPlayableResponse`, `Playability` |
| `es_prefetch.proto` | `spotify.prefetch_esperanto.proto` | `Track`, `Event`, `GetQueueResponse`, `GetEventHistoryResponse`, `Prefetch` |
| `es_prefs.proto` | `spotify.prefs.esperanto.proto` | `GetParams`, `SubParams`, `GetAllParams`, `SubAllParams`, `Value`, `SetParams`, `CreateParams`, `PrefValues`, `Prefs` |
| `es_prepare_play.proto` | `spotify.player.esperanto.proto` | `PreparePlayRequest` |
| `es_prepare_play_options.proto` | `spotify.player.esperanto.proto` | `PreparePlayOptions` |
| `es_provided_track.proto` | `spotify.player.esperanto.proto` | `ProvidedTrack` |
| `es_pushed_message.proto` | `spotify.connectivity.pubsub.esperanto.proto` | `PushedMessage` |
| `es_queue.proto` | `spotify.player.esperanto.proto` | `Queue` |
| `es_remote_config.proto` | `spotify.remote_config.esperanto.proto` | `LookupRequest`, `BoolResponse`, `IntResponse`, `EnumResponse`, `RemoteConfig` |
| `es_remote_config_dummy.proto` | `spotify.remote_config.esperanto.proto` | `ResolveDummyRequest`, `ResolveDummyResponse`, `RemoteConfigDummyResolve` |
| `es_replace_pending_event_request.proto` | `spotify.pending_events.esperanto.proto` | `ReplacePendingEventRequest` |
| `es_replace_pending_event_response.proto` | `spotify.pending_events.esperanto.proto` | `ReplacePendingEventResponse` |
| `es_response_with_reasons.proto` | `spotify.player.esperanto.proto` | `ResponseWithReasons` |
| `es_restrictions.proto` | `spotify.player.esperanto.proto` | `ModeRestrictions`, `RestrictionReasons`, `Restrictions` |
| `es_resume.proto` | `spotify.player.esperanto.proto` | `ResumeRequest` |
| `es_seek_to.proto` | `spotify.player.esperanto.proto` | `SeekToRequest` |
| `es_send_event_request.proto` | `spotify.event_sender.esperanto.proto` | `SendEventRequest` |
| `es_send_event_response.proto` | `spotify.event_sender.esperanto.proto` | `SendEventResponse`, `SendResult`, `PersistenceResult` |
| `es_sequence_rules.proto` | `spotify.player.esperanto.proto` | `RemainingSkipsRequest`, `ResetSkipsRequest`, `RemainingSkipsResponse`, `ResetSkipResponse`, `SequenceRules` |
| `es_service_type.proto` | `spotify.connectivity.hosts.esperanto.proto` | `ServiceType`, `ServiceTypeEnum` |
| `es_session_response.proto` | `spotify.player.esperanto.proto` | `SessionResponse` |
| `es_set_options.proto` | `spotify.player.esperanto.proto` | `SetOptionsRequest` |
| `es_set_queue_request.proto` | `spotify.player.esperanto.proto` | `SetQueueRequest` |
| `es_set_repeating_context.proto` | `spotify.player.esperanto.proto` | `SetRepeatingContextRequest` |
| `es_set_repeating_track.proto` | `spotify.player.esperanto.proto` | `SetRepeatingTrackRequest` |
| `es_set_shuffling_context.proto` | `spotify.player.esperanto.proto` | `SetShufflingContextRequest` |
| `es_set_sleep_timer_request.proto` | `spotify.player.esperanto.proto` | `SetSleepTimerRequest` |
| `es_signal_request.proto` | `spotify.player.esperanto.proto` | `SignalRequest` |
| `es_skip_next.proto` | `spotify.player.esperanto.proto` | `SkipNextRequest` |
| `es_skip_prev.proto` | `spotify.player.esperanto.proto` | `SkipPrevRequest` |
| `es_skip_to_track.proto` | `spotify.player.esperanto.proto` | `SkipToTrack` |
| `es_sleep_timer.proto` | `spotify.player.esperanto.proto` | `SleepTimer` |
| `es_stop.proto` | `spotify.player.esperanto.proto` | `StopRequest` |
| `es_storage.proto` | `spotify.storage.esperanto.proto` | `CacheSizeLimit`, `GetCacheSizeLimitParams`, `SetCacheSizeLimitParams`, `DeleteExpiredItemsParams`, `DeleteUnlockedItemsParams`, `RealmStats`, `Stats`, `GetStatsParams`, `FileRanges`, `GetFileRangesParams`, `DeleteItemParams`, `Realm`, `Storage` |
| `es_stored_user_info.proto` | `spotify.connectivity.auth.storage.esperanto.proto` | `StoredUserInfo`, `NullableStoredUserInfo` |
| `es_unencrypted_stored_credentials.proto` | `spotify.connectivity.auth.common.esperanto.proto` | `UnencryptedStoredCredentials` |
| `es_update.proto` | `spotify.player.esperanto.proto` | `UpdateContextRequest`, `UpdateContextPageRequest`, `UpdateContextTrackRequest`, `UpdateViewUriRequest` |
| `event_counters.proto` | `spotify.event_sender.proto` | `EventCounters` |
| `event_entity.proto` | `spotify.event_sender.proto` | `EventEntity` |
| `exclusive_mode.proto` | `spotify.exclusive_mode_esperanto.proto` | `SetEnableRequest`, `GetEnableResponse`, `GetDeviceResponse`, `Response`, `DeviceId`, `Device`, `EnumerateDevicesResponse`, `ResponseCode` |
| `extended_metadata.proto` | `spotify.extendedmetadata` | `ExtensionQuery`, `EntityRequest`, `BatchedEntityRequestHeader`, `BatchedEntityRequest`, `EntityExtensionDataArrayHeader`, `EntityExtensionDataArray`, `BatchedExtensionResponseHeader`, `BatchedExtensionResponse`, `ExtensionType` |
| `extension_kind.proto` | `spotify.extendedmetadata` | `ExtensionKind` |
| `extracted_colors.proto` | `spotify.context_track_color` | `ColorResult`, `Color`, `Status` |
| `fingerprint_audio.proto` | `spotify.fingerprintfilterservice` | `FingerprintedAudio`, `FingerprintedRange` |
| `flat_file.proto` | `spotify.contentagnostic.v2` | `FlatFile` |
| `follow_request.proto` | `spotify.socialgraph_esperanto.proto` | `FollowRequest`, `FollowRequestV4`, `FollowResponse` |
| `followed_users_request.proto` | `spotify.socialgraph_esperanto.proto` | `FollowedUsersRequest`, `FollowedUsersResponse` |
| `gabito.proto` | `spotify.event_sender.proto` | `EventEnvelope`, `PublishEventsRequest`, `PublishEventsResponse` |
| `generated_audio.proto` | `spotify.playback_metadata.proto` | `GeneratedAudioFile` |
| `get_keys_request.proto` | `spotify.your_library.proto` | `GetKeysRequest`, `GetKeysResponse` |
| `get_recently_played.proto` | `spotify.recently_played_esperanto.proto` | `GetRecentlyPlayedRequest`, `RecentlyPlayedContext`, `GetRecentlyPlayedResponse`, `HideRecentlyPlayedRequest`, `HideRecentlyPlayedResponse` |
| `get_recently_played_tracks.proto` | `spotify.recently_played_esperanto.proto` | `GetRecentlyPlayedTracksRequest`, `RecentlyPlayedEntity`, `GetRecentlyPlayedTracksResponse` |
| `global_node.proto` | `spotify.player.proto` | `GlobalNode` |
| `google/protobuf/any.proto` | `google.protobuf` | `Any` |
| `google/protobuf/cpp_features.proto` | `pb` | `CppFeatures` |
| `google/protobuf/descriptor.proto` | `google.protobuf` | `FileDescriptorSet`, `FileDescriptorProto`, `DescriptorProto`, `ExtensionRangeOptions`, `FieldDescriptorProto`, `OneofDescriptorProto`, `EnumDescriptorProto`, `EnumValueDescriptorProto`, `ServiceDescriptorProto`, `MethodDescriptorProto`, `FileOptions`, `MessageOptions`, `FieldOptions`, `OneofOptions`, `EnumOptions`, `EnumValueOptions`, `ServiceOptions`, `MethodOptions`, `UninterpretedOption`, `FeatureSet`, `FeatureSetDefaults`, `SourceCodeInfo`, `GeneratedCodeInfo`, `Edition`, `SymbolVisibility` |
| `google/protobuf/duration.proto` | `google.protobuf` | `Duration` |
| `google/protobuf/empty.proto` | `google.protobuf` | `Empty` |
| `google/protobuf/field_mask.proto` | `google.protobuf` | `FieldMask` |
| `google/protobuf/source_context.proto` | `google.protobuf` | `SourceContext` |
| `google/protobuf/struct.proto` | `google.protobuf` | `Struct`, `Value`, `ListValue`, `NullValue` |
| `google/protobuf/timestamp.proto` | `google.protobuf` | `Timestamp` |
| `google/protobuf/type.proto` | `google.protobuf` | `Type`, `Field`, `Enum`, `EnumValue`, `Option`, `Syntax` |
| `google/protobuf/wrappers.proto` | `google.protobuf` | `DoubleValue`, `FloatValue`, `Int64Value`, `UInt64Value`, `Int32Value`, `UInt32Value`, `BoolValue`, `StringValue`, `BytesValue` |
| `highlightability.proto` | `spotify.bookmarks.entity.v1` | `Highlightability` |
| `identity.proto` | `spotify.identity.v3` | `Image`, `UserProfile`, `UserProfileUpdateRequest`, `UserProfileChangedEvent` |
| `identity_trait.proto` | `spotify.contentagnostic.v2` | `IdentityTrait`, `ContributorReference`, `EntityReference` |
| `image.proto` | `spotify.contentagnostic.v2` | `Image`, `ImageInstance`, `ColorSetContrastOptions`, `ColorSet`, `RgbaColor`, `ImageSize` |
| `image_resolve.proto` | `spotify.imageresolve.proto` | `Collection`, `ProjectionMetadata`, `ProjectionMap`, `ProjectionsResponse` |
| `in_stream.proto` | `spotify.ads.esperanto.proto` | `SubInStreamRequest`, `SubInStreamResponse`, `SubBreakChangedResponse`, `InStream` |
| `include/AdConfigMetricsEvent.proto` | `spotify.event_sender.proto` | `AdConfigMetricsEvent` |
| `include/AdContext.proto` | `spotify.event_sender.proto` | `AdContext` |
| `include/AdDecisionEvent.proto` | `spotify.event_sender.proto` | `AdDecisionEvent` |
| `include/AdDetectionMappingVerification.proto` | `spotify.event_sender.proto` | `AdDetectionMappingVerification` |
| `include/AdDetectionResult.proto` | `spotify.event_sender.proto` | `AdDetectionResult` |
| `include/AdError.proto` | `spotify.event_sender.proto` | `AdError` |
| `include/AdEvent.proto` | `spotify.event_sender.proto` | `AdEvent` |
| `include/AdOpportunityEvent.proto` | `spotify.event_sender.proto` | `AdOpportunityEvent` |
| `include/AdRequestEvent.proto` | `spotify.event_sender.proto` | `AdRequestEvent` |
| `include/AdRequestMetricsEvent.proto` | `spotify.event_sender.proto` | `AdRequestMetricsEvent` |
| `include/AdSlotEvent.proto` | `spotify.event_sender.proto` | `AdSlotEvent` |
| `include/AdTransparencyEvent.proto` | `spotify.event_sender.proto` | `AdTransparencyEvent` |
| `include/AdvanceStuck.proto` | `spotify.event_sender.proto` | `AdvanceStuck` |
| `include/ApPermanentErrorNonAuth.proto` | `spotify.event_sender.proto` | `ApPermanentErrorNonAuth` |
| `include/AudioDriverError.proto` | `spotify.event_sender.proto` | `AudioDriverError` |
| `include/AudioDriverInfo.proto` | `spotify.event_sender.proto` | `AudioDriverInfo` |
| `include/AudioFileSelection.proto` | `spotify.event_sender.proto` | `AudioFileSelection` |
| `include/AudioIndexRepairReport.proto` | `spotify.event_sender.proto` | `AudioIndexRepairReport` |
| `include/AudioOffliningSettingsReport.proto` | `spotify.event_sender.proto` | `AudioOffliningSettingsReport` |
| `include/AudioRateLimit.proto` | `spotify.event_sender.proto` | `AudioRateLimit` |
| `include/AudioResolve.proto` | `spotify.event_sender.proto` | `AudioResolve` |
| `include/AudioRouteSegmentEnd.proto` | `spotify.event_sender.proto` | `AudioRouteSegmentEnd` |
| `include/AudioSessionEvent.proto` | `spotify.event_sender.proto` | `AudioSessionEvent` |
| `include/AudioSettingsReport.proto` | `spotify.event_sender.proto` | `AudioSettingsReport` |
| `include/AudioStartFrameMissed.proto` | `spotify.event_sender.proto` | `AudioStartFrameMissed` |
| `include/AudioStreamingSettingsReport.proto` | `spotify.event_sender.proto` | `AudioStreamingSettingsReport` |
| `include/AutoTransition.proto` | `spotify.event_sender.proto` | `AutoTransition` |
| `include/BoomboxExtraGappySilence.proto` | `spotify.event_sender.proto` | `BoomboxExtraGappySilence` |
| `include/BoomboxPlaybackSession.proto` | `spotify.event_sender.proto` | `BoomboxPlaybackSession` |
| `include/BoomboxTimelineContents.proto` | `spotify.event_sender.proto` | `BoomboxTimelineContents` |
| `include/BrokenObject.proto` | `spotify.event_sender.proto` | `BrokenObject` |
| `include/CacheError.proto` | `spotify.event_sender.proto` | `CacheError` |
| `include/CachePruningReport.proto` | `spotify.event_sender.proto` | `CachePruningReport` |
| `include/CacheRealmPruningReport.proto` | `spotify.event_sender.proto` | `CacheRealmPruningReport` |
| `include/CacheRealmReport.proto` | `spotify.event_sender.proto` | `CacheRealmReport` |
| `include/CacheReport.proto` | `spotify.event_sender.proto` | `CacheReport` |
| `include/CanPlayContentPermissionsMissingMetadataEvent.proto` | `spotify.event_sender.proto` | `CanPlayContentPermissionsMissingMetadataEvent` |
| `include/ClientAllowlistDownloadNonAuth.proto` | `spotify.event_sender.proto` | `ClientAllowlistDownloadNonAuth` |
| `include/ClientAllowlistFetchNonAuth.proto` | `spotify.event_sender.proto` | `ClientAllowlistFetchNonAuth` |
| `include/ClientCoreConnectCommand.proto` | `spotify.event_sender.proto` | `ClientCoreConnectCommand` |
| `include/ClientCoreConnectPlayCommand.proto` | `spotify.event_sender.proto` | `ClientCoreConnectPlayCommand` |
| `include/ClientCoreConnectPullOperation.proto` | `spotify.event_sender.proto` | `ClientCoreConnectPullOperation` |
| `include/ClientLocale.proto` | `spotify.event_sender.proto` | `ClientLocale` |
| `include/ClientOAuthMigrationAttempt.proto` | `spotify.event_sender.proto` | `ClientOAuthMigrationAttempt` |
| `include/ClientOAuthMigrationFailure.proto` | `spotify.event_sender.proto` | `ClientOAuthMigrationFailure` |
| `include/ClientOAuthMigrationSuccess.proto` | `spotify.event_sender.proto` | `ClientOAuthMigrationSuccess` |
| `include/ClientRuntimeDiag.proto` | `spotify.event_sender.proto` | `ClientRuntimeDiag` |
| `include/CollectionEndpointUsage.proto` | `spotify.event_sender.proto` | `CollectionEndpointUsage` |
| `include/CommonCappingFallback.proto` | `spotify.event_sender.proto` | `CommonCappingFallback` |
| `include/CommonCappingInitRetry.proto` | `spotify.event_sender.proto` | `CommonCappingInitRetry` |
| `include/CommonCappingStorageError.proto` | `spotify.event_sender.proto` | `CommonCappingStorageError` |
| `include/ConductorErrorEvent.proto` | `spotify.event_sender.proto` | `ConductorErrorEvent` |
| `include/ConductorEvent.proto` | `spotify.event_sender.proto` | `ConductorEvent` |
| `include/ConfigurationApplied.proto` | `spotify.event_sender.proto` | `ConfigurationApplied` |
| `include/ConfigurationFetched.proto` | `spotify.event_sender.proto` | `ConfigurationFetched` |
| `include/ConfigurationFetchedNonAuth.proto` | `spotify.event_sender.proto` | `ConfigurationFetchedNonAuth` |
| `include/ConnectAuthCodeGrantFailure.proto` | `spotify.event_sender.proto` | `ConnectAuthCodeGrantFailure` |
| `include/ConnectAuthCodeGrantSuccess.proto` | `spotify.event_sender.proto` | `ConnectAuthCodeGrantSuccess` |
| `include/ConnectAuthInfoFailureNonAuth.proto` | `spotify.event_sender.proto` | `ConnectAuthInfoFailureNonAuth` |
| `include/ConnectAuthInfoSuccessNonAuth.proto` | `spotify.event_sender.proto` | `ConnectAuthInfoSuccessNonAuth` |
| `include/ConnectConnectivityError.proto` | `spotify.event_sender.proto` | `ConnectConnectivityError` |
| `include/ConnectCredentialsRequest.proto` | `spotify.event_sender.proto` | `ConnectCredentialsRequest` |
| `include/ConnectDeviceDiscovered.proto` | `spotify.event_sender.proto` | `ConnectDeviceDiscovered` |
| `include/ConnectDiscoveryScanTrigger.proto` | `spotify.event_sender.proto` | `ConnectDiscoveryScanTrigger` |
| `include/ConnectInstallationResult.proto` | `spotify.event_sender.proto` | `ConnectInstallationResult` |
| `include/ConnectStateRestoreResult.proto` | `spotify.event_sender.proto` | `ConnectStateRestoreResult` |
| `include/ConnectTransferIncarnation.proto` | `spotify.event_sender.proto` | `ConnectTransferIncarnation` |
| `include/ConnectTransferResult.proto` | `spotify.event_sender.proto` | `ConnectTransferResult` |
| `include/ConnectUnmappedAudioOutputType.proto` | `spotify.event_sender.proto` | `ConnectUnmappedAudioOutputType` |
| `include/ConnectionError.proto` | `spotify.event_sender.proto` | `ConnectionError` |
| `include/ConnectionInfo.proto` | `spotify.event_sender.proto` | `ConnectionInfo` |
| `include/ConnectionQualityEventV2.proto` | `spotify.event_sender.proto` | `ConnectionQualityEventV2` |
| `include/ContextPlayerCommandEvent.proto` | `spotify.event_sender.proto` | `ContextPlayerCommandEvent` |
| `include/CoreAutoplayLoadingResults.proto` | `spotify.event_sender.proto` | `CoreAutoplayLoadingResults` |
| `include/CorePlaybackCommandCorrelation.proto` | `spotify.event_sender.proto` | `CorePlaybackCommandCorrelation` |
| `include/CoreShuffleStateEvent.proto` | `spotify.event_sender.proto` | `CoreShuffleStateEvent` |
| `include/CosmosVideoError.proto` | `spotify.event_sender.proto` | `CosmosVideoError` |
| `include/DatabaseStatsDebugEvent.proto` | `spotify.event_sender.proto` | `DatabaseStatsDebugEvent` |
| `include/DefaultConfigurationApplied.proto` | `spotify.event_sender.proto` | `DefaultConfigurationApplied` |
| `include/DesktopAppAuthorizationRequest.proto` | `spotify.event_sender.proto` | `DesktopAppAuthorizationRequest` |
| `include/DesktopAuthenticationSuccess.proto` | `spotify.event_sender.proto` | `DesktopAuthenticationSuccess` |
| `include/DesktopBleAdvertisingNonAuth.proto` | `spotify.event_sender.proto` | `DesktopBleAdvertisingNonAuth` |
| `include/DesktopBleErrorNonAuth.proto` | `spotify.event_sender.proto` | `DesktopBleErrorNonAuth` |
| `include/DesktopBleSupportedNonAuth.proto` | `spotify.event_sender.proto` | `DesktopBleSupportedNonAuth` |
| `include/DesktopCastError.proto` | `spotify.event_sender.proto` | `DesktopCastError` |
| `include/DesktopCoreInitializedNonAuth.proto` | `spotify.event_sender.proto` | `DesktopCoreInitializedNonAuth` |
| `include/DesktopGPUAccelerationInfo.proto` | `spotify.event_sender.proto` | `DesktopGPUAccelerationInfo` |
| `include/DesktopHighMemoryUsage.proto` | `spotify.event_sender.proto` | `DesktopHighMemoryUsage` |
| `include/DesktopHighMemoryUsageNonAuth.proto` | `spotify.event_sender.proto` | `DesktopHighMemoryUsageNonAuth` |
| `include/DesktopURIActivationNonAuth.proto` | `spotify.event_sender.proto` | `DesktopURIActivationNonAuth` |
| `include/DesktopUpdateDownloadComplete.proto` | `spotify.event_sender.proto` | `DesktopUpdateDownloadComplete` |
| `include/DesktopUpdateDownloadError.proto` | `spotify.event_sender.proto` | `DesktopUpdateDownloadError` |
| `include/DesktopUpdateMessageAction.proto` | `spotify.event_sender.proto` | `DesktopUpdateMessageAction` |
| `include/DesktopUpdateMessageProcessed.proto` | `spotify.event_sender.proto` | `DesktopUpdateMessageProcessed` |
| `include/DesktopUpdateResponse.proto` | `spotify.event_sender.proto` | `DesktopUpdateResponse` |
| `include/Download.proto` | `spotify.event_sender.proto` | `Download` |
| `include/DrmRequestFailure.proto` | `spotify.event_sender.proto` | `DrmRequestFailure` |
| `include/EndAd.proto` | `spotify.event_sender.proto` | `EndAd` |
| `include/EventSenderCanaryNonAuth.proto` | `spotify.event_sender.proto` | `EventSenderCanaryNonAuth` |
| `include/EventSenderStats.proto` | `spotify.event_sender.proto` | `EventSenderStats` |
| `include/EventSenderStats2NonAuth.proto` | `spotify.event_sender.proto` | `EventSenderStats2NonAuth` |
| `include/ExtHostedPlaybackAdPositionAdjustmentException.proto` | `spotify.event_sender.proto` | `ExtHostedPlaybackAdPositionAdjustmentException` |
| `include/ExternalDeviceInfo.proto` | `spotify.event_sender.proto` | `ExternalDeviceInfo` |
| `include/ForcedLogoutTriggeredNonAuth.proto` | `spotify.event_sender.proto` | `ForcedLogoutTriggeredNonAuth` |
| `include/HeadFileDownload.proto` | `spotify.event_sender.proto` | `HeadFileDownload` |
| `include/ImageDownload.proto` | `spotify.event_sender.proto` | `ImageDownload` |
| `include/ListPlayerErrorEvent.proto` | `spotify.event_sender.proto` | `ListPlayerErrorEvent` |
| `include/ListPlayerEvent.proto` | `spotify.event_sender.proto` | `ListPlayerEvent` |
| `include/LocalFileSyncError.proto` | `spotify.event_sender.proto` | `LocalFileSyncError` |
| `include/LocalFilesError.proto` | `spotify.event_sender.proto` | `LocalFilesError` |
| `include/LocalFilesImport.proto` | `spotify.event_sender.proto` | `LocalFilesImport` |
| `include/LocalFilesReport.proto` | `spotify.event_sender.proto` | `LocalFilesReport` |
| `include/LocalFilesSourceEvent.proto` | `spotify.event_sender.proto` | `LocalFilesSourceEvent` |
| `include/LocalFilesSourceReport.proto` | `spotify.event_sender.proto` | `LocalFilesSourceReport` |
| `include/Login5PermanentErrorNonAuth.proto` | `spotify.event_sender.proto` | `Login5PermanentErrorNonAuth` |
| `include/Login5TaskNonAuth.proto` | `spotify.event_sender.proto` | `Login5TaskNonAuth` |
| `include/MediaPrefetch.proto` | `spotify.event_sender.proto` | `MediaPrefetch` |
| `include/MidtrackDowngrade.proto` | `spotify.event_sender.proto` | `MidtrackDowngrade` |
| `include/Offline2ClientError.proto` | `spotify.event_sender.proto` | `Offline2ClientError` |
| `include/Offline2ClientEvent.proto` | `spotify.event_sender.proto` | `Offline2ClientEvent` |
| `include/OfflineError.proto` | `spotify.event_sender.proto` | `OfflineError` |
| `include/OfflineEvent.proto` | `spotify.event_sender.proto` | `OfflineEvent` |
| `include/OfflinePlayableCacheReport.proto` | `spotify.event_sender.proto` | `OfflinePlayableCacheReport` |
| `include/OfflinePruneReport.proto` | `spotify.event_sender.proto` | `OfflinePruneReport` |
| `include/OfflineReport.proto` | `spotify.event_sender.proto` | `OfflineReport` |
| `include/PartnerApiConnectLoginNonAuth.proto` | `spotify.event_sender.proto` | `PartnerApiConnectLoginNonAuth` |
| `include/PartnerApiEventNonAuth.proto` | `spotify.event_sender.proto` | `PartnerApiEventNonAuth` |
| `include/PendingEventsInternalErrorNonAuth.proto` | `spotify.event_sender.proto` | `PendingEventsInternalErrorNonAuth` |
| `include/PendingEventsStatsNonAuth.proto` | `spotify.event_sender.proto` | `PendingEventsStatsNonAuth` |
| `include/PickAndShuffleCapStateRestore.proto` | `spotify.event_sender.proto` | `PickAndShuffleCapStateRestore` |
| `include/PickAndShuffleCapStateSave.proto` | `spotify.event_sender.proto` | `PickAndShuffleCapStateSave` |
| `include/PlaybackError.proto` | `spotify.event_sender.proto` | `PlaybackError` |
| `include/PlaybackRetry.proto` | `spotify.event_sender.proto` | `PlaybackRetry` |
| `include/PlaybackSettingsCoreOperation.proto` | `spotify.event_sender.proto` | `PlaybackSettingsCoreOperation` |
| `include/PlaylistSyncEvent.proto` | `spotify.event_sender.proto` | `PlaylistSyncEvent` |
| `include/PodcastAdSegmentDifference.proto` | `spotify.event_sender.proto` | `PodcastAdSegmentDifference` |
| `include/PodcastAdSegmentReceived.proto` | `spotify.event_sender.proto` | `PodcastAdSegmentReceived` |
| `include/Prefetch.proto` | `spotify.event_sender.proto` | `Prefetch` |
| `include/PrefetchError.proto` | `spotify.event_sender.proto` | `PrefetchError` |
| `include/ProgressiveDownload.proto` | `spotify.event_sender.proto` | `ProgressiveDownload` |
| `include/PubSubCountPerIdent.proto` | `spotify.event_sender.proto` | `PubSubCountPerIdent` |
| `include/PuffinError.proto` | `spotify.event_sender.proto` | `PuffinError` |
| `include/PuffinStreamV2.proto` | `spotify.event_sender.proto` | `PuffinStreamV2` |
| `include/RawCoreReadPrototype.proto` | `spotify.event_sender.proto` | `RawCoreReadPrototype` |
| `include/RawCoreStream.proto` | `spotify.event_sender.proto` | `RawCoreStream` |
| `include/RawCoreStreamSegment.proto` | `spotify.event_sender.proto` | `RawCoreStreamSegment` |
| `include/RejectedClientEventNonAuth.proto` | `spotify.event_sender.proto` | `RejectedClientEventNonAuth` |
| `include/RestrictionsError.proto` | `spotify.event_sender.proto` | `RestrictionsError` |
| `include/ResumptionContentResumed.proto` | `spotify.event_sender.proto` | `ResumptionContentResumed` |
| `include/ResumptionProgressUpdate.proto` | `spotify.event_sender.proto` | `ResumptionProgressUpdate` |
| `include/ResumptionProgressUpdateError.proto` | `spotify.event_sender.proto` | `ResumptionProgressUpdateError` |
| `include/ResumptionRevisionRejected.proto` | `spotify.event_sender.proto` | `ResumptionRevisionRejected` |
| `include/ResumptionRevisionSyncFailure.proto` | `spotify.event_sender.proto` | `ResumptionRevisionSyncFailure` |
| `include/ResumptionRevisionSynced.proto` | `spotify.event_sender.proto` | `ResumptionRevisionSynced` |
| `include/ResumptionRevisionUpdate.proto` | `spotify.event_sender.proto` | `ResumptionRevisionUpdate` |
| `include/ResumptionRevisionsRestored.proto` | `spotify.event_sender.proto` | `ResumptionRevisionsRestored` |
| `include/ResumptionRevisionsStored.proto` | `spotify.event_sender.proto` | `ResumptionRevisionsStored` |
| `include/SAIMetadataReceived.proto` | `spotify.event_sender.proto` | `SAIMetadataReceived` |
| `include/SegmentTransferStats.proto` | `spotify.event_sender.proto` | `SegmentTransferStats` |
| `include/SegmentedDownload.proto` | `spotify.event_sender.proto` | `SegmentedDownload` |
| `include/SessionCreationFailureNonAuth.proto` | `spotify.event_sender.proto` | `SessionCreationFailureNonAuth` |
| `include/SessionCreationSuccessNonAuth.proto` | `spotify.event_sender.proto` | `SessionCreationSuccessNonAuth` |
| `include/SessionTerminationNonAuth.proto` | `spotify.event_sender.proto` | `SessionTerminationNonAuth` |
| `include/ShuffleAlgorithmEvent.proto` | `spotify.event_sender.proto` | `ShuffleAlgorithmEvent` |
| `include/ShuffleContextEvent.proto` | `spotify.event_sender.proto` | `ShuffleContextEvent` |
| `include/ShuffleSequenceEvent.proto` | `spotify.event_sender.proto` | `ShuffleSequenceEvent` |
| `include/SlowLoadingListEvent.proto` | `spotify.event_sender.proto` | `SlowLoadingListEvent` |
| `include/StreamLocation.proto` | `spotify.event_sender.proto` | `StreamLocation` |
| `include/StreamReportingApiDebug.proto` | `spotify.event_sender.proto` | `StreamReportingApiDebug` |
| `include/StreamReportingDebugTracerTest.proto` | `spotify.event_sender.proto` | `StreamReportingDebugTracerTest` |
| `include/StreamReportingError.proto` | `spotify.event_sender.proto` | `StreamReportingError` |
| `include/StreamReportingProbe.proto` | `spotify.event_sender.proto` | `StreamReportingProbe` |
| `include/StreamReportingRCSCopy.proto` | `spotify.event_sender.proto` | `StreamReportingRCSCopy` |
| `include/StriderSetup.proto` | `spotify.event_sender.proto` | `StriderSetup` |
| `include/Stutter.proto` | `spotify.event_sender.proto` | `Stutter` |
| `include/TimeMeasurement.proto` | `spotify.event_sender.proto` | `TimeMeasurement` |
| `include/TimeMeasurementErrorNonAuth.proto` | `spotify.event_sender.proto` | `TimeMeasurementErrorNonAuth` |
| `include/TimelineReporterError.proto` | `spotify.event_sender.proto` | `TimelineReporterError` |
| `include/TokenRefreshFailureNonAuth.proto` | `spotify.event_sender.proto` | `TokenRefreshFailureNonAuth` |
| `include/TokenRefreshSuccess.proto` | `spotify.event_sender.proto` | `TokenRefreshSuccess` |
| `include/TrackErrorBypass.proto` | `spotify.event_sender.proto` | `TrackErrorBypass` |
| `include/TrackNotPlayed.proto` | `spotify.event_sender.proto` | `TrackNotPlayed` |
| `include/TrackStuck.proto` | `spotify.event_sender.proto` | `TrackStuck` |
| `include/TracksClusterReport.proto` | `spotify.event_sender.proto` | `TracksClusterReport` |
| `include/TrafficStats.proto` | `spotify.event_sender.proto` | `TrafficStats` |
| `include/TrafficStatsNonAuth.proto` | `spotify.event_sender.proto` | `TrafficStatsNonAuth` |
| `include/VideoManifestDownload.proto` | `spotify.event_sender.proto` | `VideoManifestDownload` |
| `include/WebsocketConnectionNonAuth.proto` | `spotify.event_sender.proto` | `WebsocketConnectionNonAuth` |
| `include/WindowSizeNonAuth.proto` | `spotify.event_sender.proto` | `WindowSizeNonAuth` |
| `installation_data.proto` | `spotify.event_sender.proto` | `InstallationEntity`, `InstallationData` |
| `instrumentation_params.proto` | `spotify.player.proto.transfer` | `InstrumentationParams` |
| `is_curated_request.proto` | `spotify.your_library.proto` | `IsCuratedRequest`, `IsCuratedResponse`, `IsCuratedItem` |
| `item_reporting_context.proto` | `spotify.timeline_reporter.proto` | `ItemReportingContext` |
| `karaoke_get_status_request.proto` | `spotify.karaoke_esperanto.proto` | `KaraokeGetStatusRequest`, `KaraokeGetStatusResponse` |
| `karaoke_post_status_request.proto` | `spotify.karaoke_esperanto.proto` | `KaraokePostStatusRequest`, `KaraokePostStatusResponse` |
| `karaoke_post_vocal_volume_request.proto` | `spotify.karaoke_esperanto.proto` | `KaraokePostVocalVolumeRequest`, `KaraokePostVocalVolumeResponse` |
| `karaoke_status.proto` | `spotify.karaoke_esperanto.proto` | `KaraokeStatus` |
| `karaoke_sub_events_request.proto` | `spotify.karaoke_esperanto.proto` | `KaraokeSubEventsRequest`, `KaraokeSubEventsResponse` |
| `lens-model.proto` | `spotify.lens.proto` | `Lens`, `LensStatus`, `LensState` |
| `lfs_secret_provider.proto` | `spotify.lfssecretprovider.proto` | `GetSecretResponse` |
| `list_player_state.proto` | `spotify.list_player.proto` | `ListPlayerState` |
| `list_tuner_audio_analysis_mdata_extension.proto` | `spotify.playlist.tuner.extension` | `ListTunerAudioAnalysis` |
| `local_list_config.proto` | `spotify.playlist.local_list_config.proto` | `LocalListConfig` |
| `local_list_config_save_request.proto` | `spotify.playlist.local_list_config_esperanto.proto` | `LocalListConfig`, `LocalListConfigSaveRequest`, `LocalListConfigSaveResponse` |
| `local_sync_state.proto` | `spotify.local_sync_state.proto` | `State` |
| `logging_params.proto` | `spotify.player.proto` | `LoggingParams` |
| `loudness_response.proto` | `spotify.analyzer_esperanto.proto` | `LoudnessResponse` |
| `mdata.proto` | `spotify.mdata.proto` | `LocalExtensionQuery`, `LocalBatchedEntityRequest`, `LocalBatchedExtensionResponse` |
| `mdata_cosmos.proto` | `spotify.mdata_cosmos.proto` | `InvalidateCacheRequest`, `InvalidateCacheResponse` |
| `mdata_debug.proto` | `spotify.mdata_debug_esperanto.proto` | `ListAllInCacheRequest`, `ListAllInCacheResponse`, `PurgeAllInCacheRequest`, `PurgeAllInCacheResponse`, `PeekCacheEntryRequest`, `PeekCacheEntryResponse` |
| `mdata_entities.proto` | `spotify.mdata.proto` | `AudiobookSpecifics`, `Credits` |
| `mdata_observe_request.proto` | `spotify.mdata_esperanto.proto` | `ObserveRequest` |
| `mdata_observe_response.proto` | `spotify.mdata_esperanto.proto` | `ObserveResponse` |
| `mdata_storage.proto` | `spotify.mdata.proto.storage` | `CacheEntry`, `Vary`, `CacheInfo`, `OfflineLock`, `AudioFiles`, `TrackDescriptor` |
| `media.proto` | `spotify.common.media` | `AudioQuality` |
| `media_manifest.proto` | `spotify.media_manifest.proto` | `AudioFile`, `File`, `Files`, `EncryptionType` |
| `media_type.proto` | `spotify.stream_reporting_esperanto.proto` | `MediaType` |
| `members_request.proto` | `spotify.playlist.cosmos.proto` | `OptionalLimit`, `PlaylistMembersRequest` |
| `members_response.proto` | `spotify.playlist.cosmos.proto` | `Member`, `PlaylistMembersResponse`, `FollowState` |
| `metadata.proto` | `spotify.metadata` | `Artist`, `Album`, `Track`, `TrackAudioFormat`, `ArtistWithRole`, `Show`, `Episode`, `Licensor`, `Audio`, `TopTracks`, `ActivityPeriod`, `AlbumGroup`, `Date`, `Image`, `ImageGroup`, `Biography`, `Disc`, `Copyright`, `Restriction`, `Availability`, `SalePeriod`, `ExternalId`, `AudioFile`, `Video`, `VideoFile`, `ContentRating` |
| `metadata/album_metadata.proto` | `spotify.cosmos_util.proto` | `AlbumArtistMetadata`, `AlbumMetadata` |
| `metadata/artist_metadata.proto` | `spotify.cosmos_util.proto` | `ArtistMetadata` |
| `metadata/episode_metadata.proto` | `spotify.cosmos_util.proto` | `EpisodeShowMetadata`, `EpisodeMetadata` |
| `metadata/extension.proto` | `spotify.cosmos_util.proto` | `Extension` |
| `metadata/image_group.proto` | `spotify.cosmos_util.proto` | `ImageGroup` |
| `metadata/show_metadata.proto` | `spotify.cosmos_util.proto` | `ShowListAttribute`, `ShowMetadata` |
| `metadata/track_metadata.proto` | `spotify.cosmos_util.proto` | `TrackAlbumArtistMetadata`, `TrackAlbumMetadata`, `TrackArtistMetadata`, `TrackDescriptor`, `TrackMetadata` |
| `metadata_cosmos.proto` | `spotify.metadata_cosmos.proto` | `MetadataItem`, `MultiResponse`, `MultiRequest` |
| `metadata_esperanto.proto` | `spotify.metadata_esperanto.proto` | `GetEntityRequest`, `GetEntityResponse`, `ClassicMetadataService` |
| `mft_play_history.proto` | `spotify.playlist.proto.mft_lens_poc` | `PlayHistoryItem`, `CurrentContext`, `PlayHistory` |
| `mft_resumption.proto` | `spotify.list_player.proto` | `MftResumptionEntry`, `MftResumptionState` |
| `mft_state.proto` | `spotify.list_player.proto` | `SequenceSpecEntry`, `SequenceSpec`, `PlayedItemsInfo`, `PlayedItemsCurrentItem`, `PlayedItems`, `MftState`, `SequenceSpecItemType`, `PlayedItemsSource` |
| `mix_beats.proto` | `spotify.playlistmixing.extensions.mixbeats` | `Beat`, `Beats` |
| `mix_state.proto` | `spotify.playlistmixing.extensions.mixstate` | `MixState` |
| `mix_vocal_activity.proto` | `spotify.playlistmixing.extensions.mixvocalactivity` | `VocalActivity` |
| `mixability.proto` | `spotify.playlistmixing.extensions.mixability` | `Mixability` |
| `mixing_bundle.proto` | `spotify.automix.bundle.proto` | `Point`, `AutomationCurve`, `AutomationCurves`, `SalesPeriod`, `Identifier`, `PresetDefinition`, `LegacyPresetDefinition`, `LegacyVolumeStyleMapping`, `LegacyEqStyleMapping`, `LegacyFilterFxStyleMapping`, `Preset`, `VolumeAutomation`, `VolumeStyle`, `EqAutomation`, `EqStyle`, `FilterAutomation`, `FilterStyle`, `FxAutomation`, `FxStyle`, `LegacyVolumeStyle`, `LegacyEqStyle`, `LegacyFilterFxStyle`, `MixingBundle` |
| `modification_request.proto` | `spotify.playlist.cosmos.proto` | `ModificationRequest`, `CreationInfo`, `ModificationResponse`, `ListAttributeKind`, `PublishedState`, `CreateItemKind` |
| `music_video_node.proto` | `spotify.player.proto` | `MusicVideoNode` |
| `net-fortune.proto` | `spotify.netfortune.proto` | `NetFortuneResponse`, `NetFortuneV2Response` |
| `next_best_episode.proto` | `spotify.nextepisode` | `NextEpisode`, `NextEpisodes`, `NextEpisodeRecs` |
| `offline.proto` | `spotify.offline.proto` | `Capacity`, `Capabilities`, `Device`, `Restrictions`, `Resource`, `DeviceKey`, `ResourceOperation`, `PlaylistAdditionalData`, `ResourceState`, `ChangeReason` |
| `on_demand_in_free_reason.proto` | `spotify.on_demand_set.proto` | `OnDemandInFreeReason` |
| `on_demand_set_cosmos_request.proto` | `spotify.on_demand_set_cosmos.proto` | `Set`, `Temporary` |
| `on_demand_set_response.proto` | `spotify.on_demand_set_esperanto.proto` | `ResponseStatus` |
| `on_platform_reputation_trait.proto` | `spotify.contentagnostic.v2` | `OnPlatformReputationTrait`, `Chart`, `ChartPosition`, `Rating`, `GranularRatings` |
| `orchestration_stack.proto` | `spotify.stream_reporting_esperanto.proto` | `OrchestrationStack` |
| `overrides.proto` | `spotify.desktop.desktop_remote_config.proto` | `Overrides` |
| `parametric_eq_esperanto.proto` | `spotify.parametric_eq_esperanto.proto` | `FilterSpec`, `ParametricEqConfig`, `SetConfigResponse`, `BiquadFilterType`, `SetConfigError` |
| `parental_controls_pubsub.proto` | `spotify.parental_controls.proto` | `KeyValuePair`, `UserAttributesUpdate` |
| `pause_resume_origin.proto` | `spotify.player.proto` | `PauseResumeOrigin` |
| `pe_sequence_number_entity.proto` | `spotify.pending_events.proto` | `PESequenceNumberEntity` |
| `pending_event_entity.proto` | `spotify.pending_events.proto` | `PendingEventEntity` |
| `pending_tracking.proto` | `spotify.ads.proto` | `SavedTrackingRequest`, `SavedPendingTracking` |
| `perf_metrics/perf_metrics_service.proto` | `spotify.perf_metrics.esperanto.proto` | `PerfMetricsRequest`, `PerfMetricsResponse`, `PerfMetricsService` |
| `permissions_esperanto.proto` | `spotify.restrictions.permissions.permissions_proto` | `QueueQueryResponse`, `MusicVideoAccessPermissionQueryResponse`, `PodcastVideoAdSkipPermissionQueryResponse`, `CanPlayContentRequest`, `CanPlayContentResponse`, `CanPlayContentResult`, `ObserveInvalidationResponse`, `QueuePermissionResults`, `MusicVideoAccessPermissionResult`, `PodcastVideoAdSkipPermissionResult`, `Permissions` |
| `permissions_state.proto` | `spotify.restrictions.proto` | `PermissionsState`, `MusicLicense` |
| `pin_request.proto` | `spotify.your_library.proto` | `PinRequest`, `MovePinRequest`, `PinResponse`, `PinItem` |
| `play_context_bans_storage.proto` | `spotify.collection.proto.storage` | `PlayContextBansTimestamp` |
| `play_history.proto` | `spotify.list_player.proto` | `PlayHistory` |
| `play_history_entry.proto` | `spotify.player.proto.transfer` | `PlayHistoryEntry`, `PlayHistoryEntries`, `CommandType`, `Provider` |
| `play_origin.proto` | `spotify.player.proto` | `PlayOrigin` |
| `play_queue_node.proto` | `spotify.player.proto` | `PlayQueueNode` |
| `play_reason.proto` | `spotify.stream_reporting_esperanto.proto` | `PlayReason` |
| `play_request_state.proto` | `spotify.list_player.proto` | `PlayRequestState` |
| `playback-settings-pubsub.proto` | `spotify.playbacksettings.pubsub` | `DeviceSettingsFieldsChangedEvent`, `ContentSettingsChangedEvent`, `UserSettingsChangedEvent` |
| `playback-settings-storage.proto` | `spotify.playbacksettingsinternal` | `ScopeKey`, `CacheEntry`, `CacheFile` |
| `playback-settings.proto` | `spotify.playbacksettings` | `GetDeviceSettingsRequest`, `GetDeviceSettingsResponse`, `WriteDeviceValueRequest`, `WriteDeviceValueResponse`, `Device`, `DeviceDescriptor`, `DeviceSettings`, `GetContentSettingsRequest`, `GetContentSettingsResponse`, `WriteContentValueRequest`, `WriteContentValueResponse`, `GetAllStoredContentValuesRequest`, `GetAllStoredContentValuesResponse`, `ContentSettings`, `WriteUserSettingValueRequest`, `WriteUserSettingValueResponse`, `UserSettings`, `GetSettingsDeviceSelectionRequest`, `GetSettingsDeviceSelectionResponse`, `DeviceCapabilities`, `FieldValue`, `Field`, `MediaMode`, `ContextEnhancement`, `FieldId`, `PlaybackSettingsService` |
| `playback.proto` | `spotify.player.proto.transfer` | `Playback` |
| `playback_esperanto.proto` | `spotify.playback_esperanto.proto` | `ConnectLoggingParams`, `GetVolumeResponse`, `GetRawVolumeResponse`, `SubVolumeResponse`, `SubRawVolumeResponse`, `SetVolumeRequest`, `SetRawVolumeRequest`, `NudgeVolumeRequest`, `PlaybackInfoResponse`, `SubPositionRequest`, `SubPositionResponse`, `GetFilesRequest`, `GetFilesResponse`, `DuckRequest`, `DuckResponse`, `Status`, `GetFilesStatus`, `VolumeChangeSource` |
| `playback_experience_trait.proto` | `spotify.contentagnostic.v2` | `PlaybackExperienceTrait` |
| `playback_observer.proto` | `spotify.restrictions.proto` | `PlaybackObserverObservedState`, `PlaybackObserver` |
| `playback_segments.proto` | `spotify.podcast_segments.playback` | `PlaybackSegments` |
| `playback_settings.proto` | `spotify.list_player.impl.proto` | `PlaybackSettings` |
| `playback_settings_esperanto_service.proto` | `spotify.playback_settings.esperanto.proto` | `ContentSettings`, `UserSettings`, `FieldValue`, `ShuffleState`, `SetShuffleStateRequest`, `SetDisableSmartShuffleRequest`, `GetSettingsRequest`, `GetSettingsResponse`, `FieldUpdate`, `ContentSettingUpdate`, `SetSettingsRequest`, `StreamSettingsRequest`, `StreamSettingsUpdate`, `StreamSettingChangesRequest`, `ContentSettingChange`, `UserSettingChange`, `StreamSettingChange`, `GetDeviceSettingsRequest`, `GetDeviceSettingsResponse`, `SetDeviceStreamQualityRequest`, `StatusResponse`, `DeviceSettings`, `SetMediaModeRequest`, `SetDeviceSettingsRequest`, `StreamDeviceSettingFieldChangesRequest`, `DeviceSettingFieldChange`, `Status`, `FieldId`, `ContextEnhancement`, `MediaMode`, `PlaybackSettings` |
| `playback_stack.proto` | `spotify.stream_reporting_esperanto.proto` | `PlaybackStack` |
| `playback_stack_v2.proto` | `spotify.stream_reporting_esperanto.proto` | `PlaybackStackV2` |
| `playback_state.proto` | `spotify.stream_reporting_esperanto.proto` | `PlaybackState` |
| `playback_trait.proto` | `spotify.contentagnostic.v2` | `PlaybackOffsets`, `VideoHosted`, `VideoPlayback`, `AdDetection`, `AudioPassThrough`, `AudioFile`, `NormalizationParams`, `AudioHosted`, `AudioHls`, `AudioPlayback`, `PlaybackTrait`, `VideoMediaFormat`, `AudioMediaFormat` |
| `played_state/episode_played_state.proto` | `spotify.cosmos_util.proto` | `EpisodePlayState` |
| `played_state/playability_restriction.proto` | `spotify.cosmos_util.proto` | `PlayabilityRestriction` |
| `played_state/show_played_state.proto` | `spotify.cosmos_util.proto` | `ShowPlayState` |
| `played_state/track_played_state.proto` | `spotify.cosmos_util.proto` | `TrackPlayState` |
| `player.proto` | `spotify.connectstate` | `PlayerState`, `ProvidedTrack`, `ContextIndex`, `ModeRestrictions`, `RestrictionReasons`, `Restrictions`, `PlayOrigin`, `ContextPlayerOptions`, `Suppressions`, `PlaybackQuality`, `AdBreakContext`, `SleepTimer`, `FileOrigin`, `BitrateLevel`, `BitrateStrategy`, `HiFiStatus` |
| `player_filter_counts.proto` | `spotify.playlist.proto` | `FilterInfo` |
| `player_license.proto` | `spotify.player.proto` | `PlayerLicense` |
| `player_sequence/player/state/context_tree.proto` | `spotify.player.sequence.proto` | `ContextTree` |
| `playlist_api.proto` | `spotify.playlist4.proto` | `Item`, `MetaItem`, `ListItems`, `PaginatedUnfollowedListItems`, `UnfollowedListItem`, `FormatListAttribute`, `PictureSize`, `RecommendationInfo`, `ListAttributes`, `ItemAttributes`, `Add`, `Rem`, `Mov`, `ItemAttributesPartialState`, `ListAttributesPartialState`, `UpdateItemAttributes`, `UpdateListAttributes`, `UpdateItemUris`, `UriReplacement`, `Op`, `OpList`, `ChangeInfo`, `CreationInfo`, `PartnerInfo`, `SourceInfo`, `Delta`, `Diff`, `ListChanges`, `ListSignals`, `SelectedListContent`, `AppliedLenses`, `CreateListReply`, `PlaylistV1UriRequest`, `PlaylistV1UriReply`, `ListUpdateRequest`, `RegisterPlaylistImageRequest`, `RegisterPlaylistImageResponse`, `ResolvedPersonalizedPlaylist`, `PlaylistUriResolverResponse`, `ItemIdAlgorithm`, `ListAttributeKind`, `ItemAttributeKind`, `GeoblockBlockingType` |
| `playlist_cache.proto` | `spotify.playlist_cache.proto` | `ContextSyncErrorCode`, `CacheHintInfo`, `PlaylistCacheInfo`, `PendingPlaylistChanges`, `AdditionalData`, `Playlist`, `GeneralInfo` |
| `playlist_clear_token_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistClearTokenRequest`, `PlaylistClearTokenResponse` |
| `playlist_contains_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistContainsRequest`, `PlaylistContainsResponse` |
| `playlist_find_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistFindRequest`, `ItemUriToRowIds`, `PlaylistFindResponse` |
| `playlist_folder_state.proto` | `spotify.playlist.cosmos.proto` | `FolderMetadata` |
| `playlist_get_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistGetRequest`, `PlaylistMultiGetSingleRequest`, `PlaylistMultiGetRequest`, `PlaylistGetResponse`, `PlaylistMultiGetSingleResponse`, `PlaylistMultiGetResponse` |
| `playlist_invalidate_revisions_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistInvalidateRevisionsResponse` |
| `playlist_item_reporting_context.proto` | `spotify.playlist.proto` | `ItemReportingContext` |
| `playlist_lens_request.proto` | `spotify.playlist_esperanto.proto` | `LensDefinition`, `PlaylistLensRequest`, `PlaylistLensResponse` |
| `playlist_members_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistMembersResponse` |
| `playlist_modification_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistModificationRequest`, `PlaylistModificationResponse` |
| `playlist_offline_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistOfflineRequest`, `PlaylistOfflineResponse`, `PlaylistOfflineAction` |
| `playlist_permission.proto` | `spotify.playlist_permission.proto` | `AuditMetadata`, `Audit`, `Permission`, `GrantableLevels`, `AdministratePermissionCapabilities`, `AdministratePermissionKindCapabilities`, `AttributeCapabilities`, `AttributeKindCapabilities`, `ListAttributeCapabilities`, `Capabilities`, `CapabilitiesMultiRequest`, `CapabilitiesRequestOptions`, `CapabilitiesRequest`, `CapabilitiesMultiResponse`, `CapabilitiesResponse`, `SetPermissionLevelRequest`, `SetPermissionResponse`, `GetMemberPermissionsResponse`, `SetMemberPermissionEntry`, `SetMemberPermissionsRequest`, `SetMemberPermissionsResponse`, `SetMemberPermissionResult`, `Permissions`, `PermissionState`, `PermissionStatePub`, `PermissionGrantOptions`, `PermissionGrant`, `PermissionGrantDetails`, `PermissionGrantDescription`, `ClaimPermissionGrantResponse`, `ResponseStatus`, `PermissionIdentifier`, `PermissionEntry`, `CreateInitialPermissions`, `CreateInitialPermissionsResponse`, `DefaultOwnerCapabilitiesResponse`, `ListPermissionGrantsResponse`, `PermissionHistoryEntryUser`, `PermissionHistoryEntry`, `GetPermissionHistoryResponse`, `PermissionLevel`, `PermissionLevelState`, `AttributeKind`, `PermissionIdentifierKind` |
| `playlist_play.proto` | `spotify.playlist.proto` | `PlaylistPlayRequest`, `SyntheticItemInfo`, `TransferredQueueItem`, `TransferredQueueAnchor`, `TransferredQueueInfo`, `ResponseStatus`, `PlaylistPlayResponse`, `ReasonStart` |
| `playlist_play_options.proto` | `spotify.playlist.proto` | `PlayOptions`, `AudioStream` |
| `playlist_play_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistPlayRequest`, `PlaylistPlayResponse` |
| `playlist_play_state.proto` | `spotify.playlist.proto` | `NavigationHistory`, `PlayState` |
| `playlist_playlist_state.proto` | `spotify.playlist.cosmos.proto` | `FormatListAttribute`, `Allows`, `PlaylistMetadata`, `PlaylistOfflineState` |
| `playlist_query.proto` | `spotify.playlist_esperanto.proto` | `PlaylistRange`, `PlaylistQuery`, `PlaylistSourceRestriction` |
| `playlist_request.proto` | `spotify.playlist.cosmos.playlist_request.proto` | `AvailableConcepts`, `AvailableLanguage`, `AvailableSignal`, `ItemOfflineState`, `ItemCollectionState`, `ItemMetadata`, `ItemCurationState`, `Item`, `Lens`, `LensState`, `Playlist`, `RecommendationItem`, `Collaborator`, `Collaborators`, `NumberOfItemsForLinkType`, `Response`, `SignalState` |
| `playlist_resolved_list.proto` | `spotify.playlist.proto` | `ShuffleListMetadata`, `ListHeader`, `ShuffleItemMetadata`, `FormatAttributes`, `MediaSlice`, `ListItem`, `ResolvedList`, `EndOfListAction` |
| `playlist_resync_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistResyncResponse` |
| `playlist_set_base_permission_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistSetBasePermissionRequest`, `PlaylistSetBasePermissionResponse` |
| `playlist_set_member_permission_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistSetMemberPermissionResponse` |
| `playlist_set_permission_request.proto` | `spotify.playlist.cosmos.proto` | `SetBasePermissionRequest`, `SetBasePermissionResponse` |
| `playlist_set_token_request.proto` | `spotify.playlist_esperanto.proto` | `PlaylistSetTokenRequest`, `PlaylistSetTokenResponse` |
| `playlist_signal_request.proto` | `spotify.playlist_esperanto.proto` | `Signal`, `PlaylistMultiSignalRequest`, `PlaylistMultiItemSignalRequest`, `PlaylistItemSignalResponse`, `PlaylistSignalResponse` |
| `playlist_sync_reason.proto` | `spotify.playlist4.proto` | `PlaylistSyncReason`, `PlaylistSyncCaller` |
| `playlist_track_state.proto` | `spotify.playlist.cosmos.proto` | `TrackCollectionState`, `TrackOfflineState` |
| `playlist_user_state.proto` | `spotify.playlist.cosmos.proto` | `User` |
| `plugin.proto` | `spotify.offline.proto` | `PluginRegistry`, `PluginInit`, `TargetFormat`, `Metadata`, `IdentifyCommand`, `IdentifyResponse`, `DownloadCommand`, `DownloadResponse`, `StopDownloadCommand`, `StopDownloadResponse`, `RemoveCommand`, `RemoveResponse`, `PluginCommand`, `PluginResponse` |
| `pnscapper.proto` | `spotify.pnscapper.v1` | `GetCapInfoResponse`, `Empty` |
| `podcast_ad_segments.proto` | `spotify.ads.formats` | `PodcastAds`, `Segment`, `Slot` |
| `podcast_ad_testing.proto` | `spotify.ads.esperanto.proto` | `PodcastAdSegment`, `GetNextAdSegmentRequest`, `GetNextAdSegmentResponse`, `OverrideMetadataRequest`, `PodcastTesting` |
| `podcast_cta_cards.proto` | `spotify.context_mdata.podcastctacards` | `Card` |
| `podcast_paywalls.proto` | `spotify.podcast_paywalls_esperanto.proto` | `PodcastPaywallsShowSubscriptionRequest`, `PodcastPaywallsShowSubscriptionResponse`, `PodcastPaywalls` |
| `podcast_poll.proto` | `spotify.polls` | `PodcastPoll`, `Poll`, `PollOption`, `PollType`, `PollStatus` |
| `podcast_qna.proto` | `spotify.qanda` | `PodcastQna`, `Prompt`, `QAndAStatus` |
| `podcast_segments.proto` | `spotify.podcast_segments` | `PodcastSegments`, `PlaybackSegment`, `EmbeddedSegment`, `SegmentType` |
| `podcast_sponsored_content.proto` | `spotify.sponsoredcontentlistener.v1` | `SponsoredContentListenerPayload` |
| `podcast_subscription.proto` | `spotify.podcast_paywalls` | `PodcastSubscription` |
| `podcastextensions.proto` | `spotify.podcast.extensions` | `PodcastTopics`, `PodcastTopic`, `PodcastHtmlDescription` |
| `policy/album_decoration_policy.proto` | `spotify.cosmos_util.proto` | `AlbumDecorationPolicy`, `AlbumCollectionDecorationPolicy`, `AlbumSyncDecorationPolicy` |
| `policy/artist_decoration_policy.proto` | `spotify.cosmos_util.proto` | `ArtistDecorationPolicy`, `ArtistCollectionDecorationPolicy`, `ArtistSyncDecorationPolicy` |
| `policy/episode_decoration_policy.proto` | `spotify.cosmos_util.proto` | `EpisodeDecorationPolicy`, `EpisodeCollectionDecorationPolicy`, `EpisodeSyncDecorationPolicy`, `EpisodePlayedStateDecorationPolicy` |
| `policy/folder_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `FolderDecorationPolicy` |
| `policy/playlist_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `AvailableConcepts`, `AvailableLanguages`, `PlaylistAllowsDecorationPolicy`, `PlaylistDecorationPolicy` |
| `policy/playlist_episode_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `PlaylistEpisodeDecorationPolicy` |
| `policy/playlist_request_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `ItemExtensionPolicy`, `ItemOfflineStateDecorationPolicy`, `ItemMetadataPolicy`, `ItemCurationStatePolicy`, `PlaylistItemDecorationPolicy`, `PlaylistRequestDecorationPolicy` |
| `policy/rootlist_request_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `RootlistRequestDecorationPolicy` |
| `policy/show_decoration_policy.proto` | `spotify.cosmos_util.proto` | `ShowDecorationPolicy`, `ShowPlayedStateDecorationPolicy`, `ShowCollectionDecorationPolicy`, `ShowOfflineStateDecorationPolicy` |
| `policy/track_decoration_policy.proto` | `spotify.cosmos_util.proto` | `TrackDecorationPolicy`, `TrackPlayedStateDecorationPolicy`, `TrackCollectionDecorationPolicy`, `TrackSyncDecorationPolicy` |
| `policy/user_decoration_policy.proto` | `spotify.playlist.cosmos.proto` | `UserDecorationPolicy`, `CollaboratorPolicy`, `CollaboratingUsersDecorationPolicy` |
| `popcount2_external.proto` | `spotify.popcount2.proto` | `PopcountRequest`, `PopcountResult`, `PopcountUserUpdate`, `PopcountFollowerResult`, `PopcountSetFollowerCounterValueRequest` |
| `preloaded.proto` | `spotify.playback_platform.context.v1` | `Preloaded` |
| `prepare_play_options.proto` | `spotify.player.proto` | `PreparePlayOptions`, `PrefetchLevel`, `AudioStream` |
| `prerelease_extension.proto` | `spotify.prerelease.extension` | `Entity`, `Prerelease`, `Image`, `Agent` |
| `preview.proto` | `spotify.automix.proto` | `Preview` |
| `product_state.proto` | `spotify.product_state.esperanto.proto` | `GetValuesRequest`, `SubValuesRequest`, `PutOverridesValuesRequest`, `DelOverridesValuesRequest`, `PutValuesRequest`, `PutValuesResponse`, `PutValuesSyncRequest`, `PutValuesSyncResponse`, `GetValuesResponse`, `PutOverridesValuesResponse`, `DelOverridesValuesResponse`, `ProductState` |
| `profile_service.proto` | `spotify.profile_esperanto.proto.v1` | `GetProfilesRequest`, `GetProfilesResponse`, `ChangeDisplayNameRequest`, `ChangeDisplayNameResponse`, `ProfileService` |
| `progress_playback_esperanto.proto` | `spotify.progress_playback_esperanto.proto` | `OnUpdateRequest`, `OnUpdateResponse`, `Reason`, `ResponseResult`, `ResumptionProgressPlayback` |
| `property_definition.proto` | `spotify.remote_config.ucs.proto` | `PropertyDefinition` |
| `protobuf_delta.proto` | `spotify.protobuf_deltas.proto` | `Delta` |
| `publishing_metadata_trait.proto` | `spotify.contentagnostic.v2` | `PublishingMetadataTrait`, `TimestampOfVaryingGranularity`, `Year`, `YearMonth`, `YearMonthDay` |
| `pubsub.proto` | `spotify.playlist4.proto` | `SubscribeRequest`, `UnsubscribeRequest`, `PlaylistModificationInfo`, `RootlistModificationInfo`, `FollowerUpdate`, `PlaylistCreationInfo` |
| `puffin_error.proto` | `spotify.puffin_esperanto.proto` | `PuffinError` |
| `puffin_error_code.proto` | `spotify.puffin_esperanto.proto` | `PuffinErrorCode` |
| `puffin_filter_request.proto` | `spotify.puffin_esperanto.proto` | `PuffinFilterRequest`, `PuffinDisableProcessingRequest`, `Reason` |
| `puffin_library_version_response.proto` | `spotify.puffin_esperanto.proto` | `PuffinLibraryVersionResponse` |
| `puffin_processing_compatibility.proto` | `spotify.puffin_esperanto.proto` | `PuffinProcessingCompatibility` |
| `queue.proto` | `spotify.player.proto.transfer` | `Queue` |
| `rate_limited_events.proto` | `spotify.event_sender.proto` | `RateLimitedEventsEntity` |
| `rcs.proto` | `spotify.remote_config.proto` | `GranularConfiguration`, `PolicyGroupId`, `ClientPropertySet`, `PropertyDefinition`, `Platform` |
| `reactions.proto` | `spotify.reactions.v1` | `SingleReactionMetadata`, `ReactionsMetadata`, `ReactionType` |
| `rec_pool_loader.proto` | `spotify.list_player.impl.proto` | `RecPoolLoaderState` |
| `recently_played.proto` | `spotify.recently_played.proto` | `Item` |
| `recently_played_backend.proto` | `spotify.recently_played_backend.proto` | `Context`, `RecentlyPlayed` |
| `recently_played_decoration_policy.proto` | `spotify.recently_played_esperanto.proto` | `RecentlyPlayedArtistDecorationPolicy`, `RecentlyPlayedAlbumDecorationPolicy`, `RecentlyPlayedTrackDecorationPolicy`, `RecentlyPlayedShowDecorationPolicy`, `RecentlyPlayedEpisodeDecorationPolicy`, `RecentlyPlayedProfileDecorationPolicy`, `RecentlyPlayedPlaylistDecorationPolicy`, `RecentlyPlayedCollectionTracksDecorationPolicy` |
| `recently_played_item.proto` | `spotify.recently_played_esperanto.proto` | `RecentlyPlayedArtist`, `RecentlyPlayedAlbum`, `RecentlyPlayedTrack`, `RecentlyPlayedShow`, `RecentlyPlayedEpisode`, `RecentlyPlayedProfile`, `RecentlyPlayedPlaylist`, `RecentlyPlayedCollectionTracks` |
| `recently_played_playlist.proto` | `spotify.your_library.proto` | `RecentlyPlayedPlaylist` |
| `record_id.proto` | `spotify.event_sender.proto` | `RecordId` |
| `remote.proto` | `spotify.player.shuffle.remote` | `ServiceRequest`, `ServiceResponse` |
| `repeat_tracks.proto` | `spotify.restrictions.impl.proto` | `RepeatTracks` |
| `request_failure.proto` | `spotify.image.proto` | `RequestFailure` |
| `resolve.proto` | `spotify.remote_config.ucs.proto` | `ResolveRequest`, `ResolveResponse`, `Configuration`, `Fetch`, `Context`, `BackendContext`, `DynamicContext` |
| `resource_type.proto` | `spotify.offline.proto` | `ResourceType` |
| `response_status.proto` | `spotify.playlist_esperanto.proto` | `ResponseStatus` |
| `restrictions.proto` | `spotify.player.proto` | `ModeRestrictions`, `RestrictionReasons`, `Restrictions` |
| `resumption_platform.proto` | `spotify.resumption.v1` | `ListResumePoint`, `RelativePosition`, `ResumePoint`, `ResumePointRevision`, `ListResumePointRevisionsRequest`, `ListResumePointRevisionsResponse`, `CreateResumePointRevisionRequest`, `CreateResumePointRevisionResponse`, `BatchCreateResumePointRevisionsRequest`, `BatchCreateResumePointRevisionsResponse`, `ResumePointRevisionList`, `CurrentState`, `ListCurrentStatesRequest`, `ListCurrentStatesResponse`, `Affinity` |
| `resumption_platform_core_persistent.proto` | `spotify.resumption.core.persistent` | `Metadata`, `ListResumePoint`, `ResumePointRevision`, `ResumePointRevisions` |
| `resumption_progress_esperanto.proto` | `spotify.resumption_progress_esperanto.proto` | `ItemState`, `CurrentProgress`, `ItemDetails`, `GetStateRequest`, `GetStateResponse`, `GetDetailsRequest`, `GetDetailsResponse`, `TimeRange`, `PercentageRange`, `CurrentProgressFilter`, `LastPlayedFilter`, `Filter`, `GetAllDetailsRequest`, `GetAllDetailsResponse`, `ItemMarkResult`, `MarkAsFinishedRequest`, `MarkAsFinishedResponse`, `TimeProgress`, `ItemProgress`, `Progress`, `ProgressItem`, `GetProgressRequest`, `GetProgressResponse`, `MarkAsNotStartedRequest`, `MarkAsNotStartedResponse`, `GetResumePointRequest`, `GetResumePointResponse`, `ItemResumePoint`, `ItemResumePointEntry`, `ResumePoint`, `ResponseResult`, `ItemResult`, `State`, `StateFilter`, `ContentType`, `Sort`, `ResumePointView`, `ResumptionProgress` |
| `rootlist_get_request.proto` | `spotify.playlist_esperanto.proto` | `RootlistGetRequest`, `RootlistGetResponse` |
| `rootlist_modification_request.proto` | `spotify.playlist_esperanto.proto` | `RootlistModificationRequest`, `RootlistModificationResponse` |
| `rootlist_query.proto` | `spotify.playlist_esperanto.proto` | `RootlistRange`, `SourceRestriction`, `RootlistQuery`, `RootlistItemType` |
| `rootlist_request.proto` | `spotify.playlist.cosmos.rootlist_request.proto` | `Playlist`, `Item`, `Folder`, `Response` |
| `route_switcher_node.proto` | `spotify.player.proto` | `RouteSwitcherNode` |
| `runtime_consumption_persistence.proto` | `spotify.runtime_consumption.proto` | `PersistedConsumptionEntry` |
| `runtime_consumption_service.proto` | `spotify.runtime_consumption.esperanto.proto` | `GetConsumptionRecordsRequest`, `ConsumptionRecord`, `GetConsumptionRecordsResponse`, `RuntimeConsumptionService` |
| `save_source.proto` | `spotify.list_collection.save_source.proto` | `SaveSource`, `SourceTool` |
| `scrobble.proto` | `spotify.scrobble.esperanto.proto` | `GetIncognitoModeRequest`, `GetIncognitoModeResponse`, `PutIncognitoModeRequest`, `SubIncognitoModeRequest`, `Scrobble` |
| `search.proto` | `spotify.search.proto` | `OfflineAlbum`, `OfflineAudioEpisode`, `OfflinePlaylist`, `OfflineTrack` |
| `search_service.proto` | `spotify.search_esperanto.proto` | `OfflineSearchRequest`, `OfflineSearchResponse`, `OfflineSearchService` |
| `searchview.proto` | `spotify.searchview` | `NativeViewResponse`, `MainViewResponse`, `Section`, `SectionItem`, `VideoCarouselItem`, `GenericItem`, `DiscoveryFeedItem`, `InstantMixItem`, `RelatedSearchItem`, `DrillDownViewResponse`, `PodcastViewResponse`, `AutocompleteViewResponse`, `Recommendations`, `Entity`, `HighlightTypeNewRelease`, `HighlightTypeRecentSearch`, `HighlightTypeBestMatch`, `DateWithOffset`, `Kallax`, `Concert`, `Venue`, `Artist`, `ConcertCampaign`, `ConcertCampaignUserState`, `CampaignUserStateBeforeSale`, `CampaignUserStateOnSale`, `Track`, `RelatedEntity`, `Album`, `Playlist`, `Genre`, `Author`, `Audiobook`, `Tag`, `AudioShow`, `AudioEpisode`, `Snippet`, `Profile`, `OnDemand`, `BannerContent`, `AutocompleteQuery`, `Filters`, `Filter`, `ContentFilter`, `UploadDateFilter`, `EntityFilter`, `RenderHint`, `SectionType`, `RecommendationsType`, `EntityType` |
| `searchview_shared.proto` | `spotify.searchview` | `ChatEntryPoint`, `ConversationStarterItem`, `PodcastChapter` |
| `seek_to_position.proto` | `spotify.player.proto` | `SeekToPosition` |
| `segment_switch_node.proto` | `spotify.player.proto` | `SegmentSwitchNode` |
| `sequence_number_entity.proto` | `spotify.event_sender.proto` | `SequenceNumberEntity` |
| `sequence_player_node.proto` | `spotify.player.proto` | `SequencePlayerNode` |
| `session.proto` | `spotify.player.proto.transfer` | `Session` |
| `set_member_permission_request.proto` | `spotify.playlist.cosmos.proto` | `SetMemberPermissionRequest` |
| `settingsapi.proto` | `spotify.settingsapi.v1` | `SettingEnumValue`, `SettingValue`, `UpdateSettingRequest`, `UpdateSettingResponse` |
| `shared_enums.proto` | `spotify.audiophile` | `Bucket`, `Format` |
| `show_access.proto` | `spotify.podcast_paywalls` | `ShowAccess`, `Signifier`, `BasicExplanation`, `LegacyExplanation`, `NoExplanation`, `UpsellLinkExplanation`, `EngagementExplanation`, `CheckoutOnWebOverlayExplanation`, `FreeCheckoutExplanation`, `ConsumptionCappedExplanation`, `MultiPassExplanation`, `SOAPartner`, `AccountLinkPrompt`, `Disclaimer`, `UnlockingMethod` |
| `show_access_update.proto` | `spotify.showaccess.v1` | `ShowAccessUpdate` |
| `show_episode_state.proto` | `spotify.show_cosmos.proto` | `EpisodeCollectionState`, `EpisodeOfflineState` |
| `show_offline_state.proto` | `spotify.show_cosmos.proto` | `ShowOfflineState` |
| `show_request.proto` | `spotify.show_cosmos.proto` | `Item`, `Header`, `Response`, `AuxiliarySections`, `ContinueListeningSection`, `TrailerSection`, `LatestUnplayedEpisodeSection`, `NextBestEpisodeSection`, `SavedEpisodesSection` |
| `show_service.proto` | `spotify.show_esperanto.proto` | `Status`, `HeaderPolicy`, `ListPolicy`, `SectionsPolicy`, `QueryFilter`, `GetShowRequest`, `GetShowResponse`, `GetDecorateRequest`, `GetDecorateResponse`, `GetUnfinishedEpisodesRequest`, `GetUnfinishedEpisodesResponse`, `MarkShowAsPlayedRequest`, `MarkShowAsPlayedResponse`, `SortOrder`, `ShowService` |
| `show_show_state.proto` | `spotify.show_cosmos.proto` | `ShowCollectionState` |
| `shuffle_played_tracks_order.proto` | `spotify.player.proto` | `ShuffleTrack`, `ShufflePlayedTracksOrder` |
| `signal-model.proto` | `spotify.playlist.signal.proto` | `Signal`, `SignalData` |
| `skip_nexts.proto` | `spotify.restrictions.impl.proto` | `SkipNexts` |
| `skip_to_track.proto` | `spotify.player.proto` | `SkipToTrack` |
| `sleep_timer.proto` | `spotify.player.proto.transfer` | `SleepTimer` |
| `sms_play_history.proto` | `spotify.enhancedplaylistlens` | `HistoryItem`, `CurrentItem`, `PlayHistory`, `SignalPayload` |
| `social_connect_v2.proto` | `socialconnect` | `Session`, `SessionMember`, `SessionUpdate`, `SessionUpdateReason` |
| `socialgraph-v4.proto` | `spotify.socialgraph.v4` | `UsernameUser`, `GetRelationsUsernameResponse` |
| `socialgraph_response_status.proto` | `spotify.socialgraph_esperanto.proto` | `ResponseStatus` |
| `socialgraphv2.proto` | `spotify.socialgraph.proto` | `SocialGraphEntity`, `SocialGraphRequest`, `SocialGraphReply`, `ChangeNotification`, `EventType` |
| `speed_preview.proto` | `spotify.speedpreview.proto` | `SpeedPreview` |
| `spotify/client_token/v0/client_token_http.proto` | `spotify.clienttoken.http.v0` | `ClientTokenRequest`, `ClientDataRequest`, `ChallengeAnswersRequest`, `ClientTokenResponse`, `TokenDomain`, `GrantedTokenResponse`, `ChallengesResponse`, `ClientSecretParameters`, `EvaluateJSParameters`, `HashCashParameters`, `Challenge`, `ClientSecretHMACAnswer`, `EvaluateJSAnswer`, `HashCashAnswer`, `ChallengeAnswer`, `ClientTokenBadRequest`, `ClientTokenRequestType`, `ClientTokenResponseType`, `ChallengeType` |
| `spotify/list/v1/list_read_service.proto` | `spotify.list.v1` | `GetListRequest`, `GetListsRequest`, `GetListResponse`, `AdditionalResponse`, `GetListsResponse`, `DiffListRequest`, `DiffListsRequest`, `DiffListResponse`, `DiffListsResponse` |
| `spotify/list/v1/model/attribute_extension.proto` | `spotify.list.v1.model` | `MappingKind` |
| `spotify/list/v1/model/attribute_kinds.proto` | `spotify.list.v1.model` | `AttributeKind` |
| `spotify/list/v1/model/attributes.proto` | `spotify.list.v1.model` | `Attributes` |
| `spotify/list/v1/model/attributes/format_list_attributes.proto` | `spotify.list.v1.model.attributes` | `FormatListAttributes` |
| `spotify/list/v1/model/attributes/geoblock.proto` | `spotify.list.v1.model.attributes` | `Geoblock`, `GeoblockBlockingType` |
| `spotify/list/v1/model/attributes/recommendation_info.proto` | `spotify.list.v1.model.attributes` | `RecommendationInfo` |
| `spotify/list/v1/model/caching.proto` | `spotify.list.v1.model` | `CacheControl`, `CacheInfo`, `CacheHint`, `CachePolicy` |
| `spotify/list/v1/model/lenses.proto` | `spotify.list.v1.model` | `Lens`, `LensStatus`, `LensState`, `LensOptions`, `AppliedLenses` |
| `spotify/list/v1/model/list_diff.proto` | `spotify.list.v1.model` | `ListDiff` |
| `spotify/list/v1/model/list_entity.proto` | `spotify.list.v1.model` | `MetaItem`, `Item`, `ListEntity`, `LensEffect`, `LensRemovedItems` |
| `spotify/list/v1/model/list_identifier.proto` | `spotify.list.v1.model` | `ListIdentifier` |
| `spotify/list/v1/model/list_sync_reason.proto` | `spotify.list.v1.model` | `ListSyncReason`, `ListSyncOrigin` |
| `spotify/list/v1/model/ops.proto` | `spotify.list.v1.model` | `Target`, `Add`, `Rem`, `Mov`, `Set`, `UpdateAttributes`, `UpdateItemId`, `Op` |
| `spotify/list/v1/model/pagination.proto` | `spotify.list.v1.model` | `Pagination`, `PaginationResponse` |
| `spotify/list/v1/model/parameters.proto` | `spotify.list.v1.model` | `Parameters` |
| `spotify/list/v1/model/request_error.proto` | `spotify.list.v1.model` | `RequestError` |
| `spotify/list/v1/model/request_info.proto` | `spotify.list.v1.model` | `RequestInfo`, `RequestOptions`, `MultiListRequestOptions` |
| `spotify/list/v1/model/requested_fields.proto` | `spotify.list.v1.model` | `AttributesRequestedFields`, `ItemRequestedFields`, `MetaItemsRequestedFields`, `LensRemovedItemsRequestedFields`, `LensEffectRequestedFields`, `ListEntityRequestedFields`, `ListResponseRequestedFields`, `RequestedFields` |
| `spotify/list/v1/model/response_info.proto` | `spotify.list.v1.model` | `ResponseInfo` |
| `spotify/list/v1/model/signals.proto` | `spotify.list.v1.model` | `Signal`, `SignalData` |
| `spotify/list/v1/model/sorting.proto` | `spotify.list.v1.model` | `Sorting`, `SortingOrder`, `SortingType` |
| `spotify/playback_platform/context/v1/behavior/action.proto` | `spotify.playback_platform.context.v1.behavior` | `Action` |
| `spotify/playback_platform/context/v1/behavior/disable.proto` | `spotify.playback_platform.context.v1.behavior` | `Disable` |
| `spotify/playback_platform/context/v1/behavior/events.proto` | `spotify.playback_platform.context.v1.behavior` | `Events` |
| `spotify/playback_platform/context/v1/behavior/handler.proto` | `spotify.playback_platform.context.v1.behavior` | `Trigger`, `Actions`, `Enabled`, `EventType` |
| `spotify/playback_platform/context/v1/behavior/ledger.proto` | `spotify.playback_platform.context.v1.behavior` | `Ledger` |
| `spotify/playback_platform/context/v1/behavior/navigation.proto` | `spotify.playback_platform.context.v1.behavior` | `Navigation` |
| `spotify/playback_platform/context/v1/behavior/signals.proto` | `spotify.playback_platform.context.v1.behavior` | `Signals` |
| `spotify/playback_platform/context/v1/behavior/timeline.proto` | `spotify.playback_platform.context.v1.behavior` | `Timeline` |
| `spotify/playback_platform/context/v1/behavior/timers.proto` | `spotify.playback_platform.context.v1.behavior` | `Timers` |
| `spotify/playback_platform/context/v1/context_behavior.proto` | `spotify.playback_platform.context.v1` | `ContextBehavior` |
| `spotify/playback_platform/context/v1/context_environment.proto` | `spotify.playback_platform.context.v1` | `ContextEnvironment` |
| `spotify/playback_platform/context/v1/context_metadata.proto` | `spotify.playback_platform.context.v1` | `ContextMetadata` |
| `spotify/playback_platform/context/v1/sequence_options.proto` | `spotify.playback_platform.context.v1` | `SequenceOptions` |
| `spotify/playback_platform/timeline/v1/behavior/reporting.proto` | `spotify.playback_platform.timeline.v1.behavior` | `Reporting` |
| `spotify/playback_platform/timeline/v1/behavior/seek_into.proto` | `spotify.playback_platform.timeline.v1.behavior` | `SeekInto` |
| `spotify/playback_platform/timeline/v1/behavior/seek_skip.proto` | `spotify.playback_platform.timeline.v1.behavior` | `SeekSkip` |
| `spotify/playback_platform/timeline/v1/behavior/seek_snap.proto` | `spotify.playback_platform.timeline.v1.behavior` | `SeekSnap` |
| `spotify/playback_platform/timeline/v1/metadata/artist.proto` | `spotify.playback_platform.timeline.v1.metadata` | `Artist`, `ArtistRole` |
| `spotify/playback_platform/timeline/v1/timeline_behavior.proto` | `spotify.playback_platform.timeline.v1` | `TimelineBehavior` |
| `spotify/playback_platform/timeline/v1/timeline_match.proto` | `spotify.playback_platform.timeline.v1` | `TimelineMatch` |
| `spotify/playback_platform/timeline/v1/timeline_metadata.proto` | `spotify.playback_platform.timeline.v1` | `TimelineMetadata` |
| `spotify/playback_platform/timeline/v1/timeline_options.proto` | `spotify.playback_platform.timeline.v1` | `TimelineOptions` |
| `spotify/playback_platform/timeline/v1/timeline_query.proto` | `spotify.playback_platform.timeline.v1` | `TimelineQuery` |
| `spotify/playback_platform/transition/v1/transition_maps.proto` | `spotify.playback_platform.transition.v1` | `TargetId`, `TargetTag`, `TransitionMaps`, `TransitionMap`, `TimeMap`, `TransitionDirection` |
| `spotify/venueextendedmetadata/v1/venueextendedmetadata.proto` | `spotify.venueextendedmetadata.v1` | `VenueArtistsPreview`, `VenueLocation` |
| `state_restore/ads_rules_inject_tracks.proto` | `spotify.player.proto.state_restore` | `AdsRulesInjectTracks` |
| `state_restore/automix_rules.proto` | `spotify.player.proto.state_restore` | `AutomixRules` |
| `state_restore/automix_talk_rules.proto` | `spotify.player.proto.state_restore` | `AutomixTalkRules` |
| `state_restore/behavior_metadata_rules.proto` | `spotify.player.proto.state_restore` | `BehaviorMetadataRules` |
| `state_restore/circuit_breaker_rules.proto` | `spotify.player.proto.state_restore` | `CircuitBreakerRules` |
| `state_restore/context_loader.proto` | `spotify.player.proto.state_restore` | `ContextLoader` |
| `state_restore/context_player_rules.proto` | `spotify.player.proto.state_restore` | `PlayEvents`, `SkipEvents`, `Context`, `MftConfiguration`, `MftRules`, `ContextPlayerRules` |
| `state_restore/context_player_state.proto` | `spotify.player.proto.state_restore` | `ContextPlayerState` |
| `state_restore/explicit_content_rules.proto` | `spotify.player.proto.state_restore` | `ExplicitContentRules` |
| `state_restore/explicit_filter_bypass.proto` | `spotify.player.proto.state_restore` | `ExplicitFilterBypass` |
| `state_restore/kitteh_box_rules.proto` | `spotify.player.proto.state_restore` | `KittehBoxRules` |
| `state_restore/mft_context_history.proto` | `spotify.player.proto.state_restore` | `MftContextHistoryEntry`, `MftContextHistory` |
| `state_restore/mft_context_switch_rules.proto` | `spotify.player.proto.state_restore` | `MftContextSwitchRules` |
| `state_restore/mft_rules_core.proto` | `spotify.player.proto.state_restore` | `MftRulesCore` |
| `state_restore/mft_state.proto` | `spotify.player.proto.state_restore` | `EventList`, `LastEvent`, `History`, `MftState` |
| `state_restore/music_injection_rules.proto` | `spotify.player.proto.state_restore` | `InjectionSegment`, `InjectionModel`, `MusicInjectionRules` |
| `state_restore/play_history.proto` | `spotify.player.proto.state_restore` | `PlayHistory`, `PlayHistoryV1` |
| `state_restore/playback_quality.proto` | `spotify.player.proto.state_restore` | `PlaybackQuality` |
| `state_restore/playback_state.proto` | `spotify.player.proto.state_restore` | `PlaybackState` |
| `state_restore/player_model.proto` | `spotify.player.proto.state_restore` | `PlayerModel` |
| `state_restore/player_session.proto` | `spotify.player.proto.state_restore` | `PlayerSession` |
| `state_restore/player_session_fake.proto` | `spotify.player.proto.state_restore` | `PlayerSessionFake` |
| `state_restore/player_session_queue.proto` | `spotify.player.proto.state_restore` | `QueuedSession`, `PlayerSessionQueue` |
| `state_restore/pns_capper.proto` | `spotify.player.proto.state_restore` | `PnsCapper` |
| `state_restore/provided_track.proto` | `spotify.player.proto.state_restore` | `ProvidedTrack` |
| `state_restore/random_source.proto` | `spotify.player.proto.state_restore` | `RandomSource` |
| `state_restore/repeat_settings_node.proto` | `spotify.player.proto.state_restore` | `RepeatSettingsNode` |
| `state_restore/resume_points_rules.proto` | `spotify.player.proto.state_restore` | `ResumePoint`, `ResumePointsRules` |
| `state_restore/track_error_rules.proto` | `spotify.player.proto.state_restore` | `TrackErrorRules` |
| `state_restore/video_persistence_state.proto` | `spotify.player.proto.state_restore` | `VideoPersistenceState` |
| `status.proto` | `spotify.collection_cosmos.proto` | `Status` |
| `status_code.proto` | `spotify.stream_reporting_esperanto.proto` | `StatusCode` |
| `status_response.proto` | `spotify.stream_reporting_esperanto.proto` | `StatusResponse` |
| `storage-resolve.proto` | `spotify.download.proto` | `StorageResolveResponse` |
| `storage.proto` | `spotify.cappingapi.storage` | `Quota`, `QuotaInfo`, `ConsumptionEvent`, `Feature` |
| `storylines.proto` | `spotify.storylines.v1` | `Artist`, `Card`, `Storyline` |
| `stream_end_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamEndRequest` |
| `stream_handle.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamHandle` |
| `stream_initiate_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamInitiateRequest` |
| `stream_progress_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamProgressRequest` |
| `stream_seek_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamSeekRequest` |
| `stream_start_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamStartRequest` |
| `stream_start_response.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamStartResponse` |
| `stream_start_v2_request.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamStartV2Request` |
| `streaming_rule.proto` | `spotify.stream_reporting_esperanto.proto` | `StreamingRule` |
| `suppressions.proto` | `spotify.player.proto` | `Suppressions` |
| `sync/album_sync_state.proto` | `spotify.cosmos_util.proto` | `AlbumSyncState` |
| `sync/artist_sync_state.proto` | `spotify.cosmos_util.proto` | `ArtistSyncState` |
| `sync/episode_sync_state.proto` | `spotify.cosmos_util.proto` | `EpisodeSyncState` |
| `sync/track_sync_state.proto` | `spotify.cosmos_util.proto` | `TrackSyncState` |
| `sync_request.proto` | `spotify.playlist.cosmos.proto` | `SyncRequest` |
| `tags_data.proto` | `spotify.your_library_tags.proto` | `TagsData`, `Assignment` |
| `time_measurement_builder_snapshot.proto` | `spotify.timekeeper_proto` | `TimeMeasurementBuilderSnapshot`, `TimeMeasurementBuilderSnapshotCapturedPointStart`, `TimeMeasurementBuilderSnapshotCapturedPointEnd`, `TimeMeasurementBuilderSnapshotCapturedPoint` |
| `timestamp_response.proto` | `spotify.stream_reporting_esperanto.proto` | `TimestampResponse` |
| `track-content-filter.proto` | `spotify.bumblebee.trackcontentfilter.v1` | `TrackContentFilterData`, `Filter` |
| `track_descriptor.proto` | `spotify.descriptorextension` | `ExtensionDescriptor`, `ExtensionDescriptorData`, `ExtensionDescriptorType` |
| `track_instance.proto` | `spotify.player.proto` | `TrackInstance` |
| `track_instantiator.proto` | `spotify.player.proto` | `TrackInstantiator` |
| `transcript.proto` | `spotify.contentagnostic.v2` | `Transcript` |
| `transfer_progress.proto` | `spotify.your_library.proto` | `TransferProgressRequest`, `TransferProgressUpdate`, `TransferProgressResponse`, `TransferStatus` |
| `transfer_state.proto` | `spotify.player.proto.transfer` | `TransferState` |
| `transition.proto` | `spotify.automix.proto` | `Point`, `AutomationCurve`, `AutomationCurves`, `VolumeStyle`, `VolumeStyleCurves`, `EqStyle`, `EqStyleCurves`, `FilterFxStyle`, `FilterFxStyleCurves`, `FxStyle`, `FxStyleCurves`, `JogwheelStyle`, `LoopingStyle`, `StyleBlock`, `BlockSequence`, `JogwheelStyleBlocks`, `LoopingStyleBlocks`, `EqCurveOverrides`, `FilterCurveOverrides`, `FxCurveOverrides`, `Preset`, `Overlap`, `Transition`, `StyleBlockType`, `PresetType`, `BeatmatchPreference` |
| `transition_map.proto` | `spotify.player.proto` | `TransitionMap`, `TimeMap`, `TransitionDirection` |
| `transition_metadata_extension.proto` | `spotify.timbrefademetadata.v1` | `TransitionMetadataExtensionResponse` |
| `tts-resolve.proto` | `spotify.narration.proto` | `ResolveRequest`, `ResolveResponse`, `TtsPrompt` |
| `ucs.proto` | `spotify.remote_config.ucs.proto` | `UcsRequest`, `UcsResponseWrapper` |
| `unfinished_episodes_request.proto` | `spotify.show_cosmos.unfinished_episodes_request.proto` | `Episode`, `Response` |
| `unified_config.proto` | `spotify.ads.proto` | `UnifiedConfigRequest`, `UnifiedConfigResponse`, `SlotConfig`, `StreamConfig`, `LeaderboardConfig`, `BillboardConfig`, `MobileScreensaverConfig`, `CachedConfigEnvelope` |
| `user_queue.proto` | `spotify.queue.proto` | `SyntheticItemInfo`, `Item`, `UserQueue`, `Source` |
| `useraccount.proto` | `spotify.remote_config.ucs.proto` | `AccountAttribute` |
| `verification_ad.proto` | `spotify.ads.proto` | `SKAdNetworkData`, `Verification`, `VerificationAd` |
| `video.proto` | `spotify.contentagnostic.v2` | `Video`, `VideoAsset`, `HLSVideo` |
| `video_associations.proto` | `spotify.bumblebee.video_associations.v1` | `VideoAssociations`, `VideoAssociation`, `ImageGroup`, `Image` |
| `video_surface.proto` | `spotify.stream_reporting_esperanto.proto` | `VideoSurface` |
| `video_switch_node.proto` | `spotify.player.proto` | `VideoSwitchNode` |
| `visual_identity_trait.proto` | `spotify.contentagnostic.v2` | `VisualIdentityTrait` |
| `volume_messages.proto` | `spotify.connect.esperanto.proto` | `VolumeUpRequest`, `VolumeDownRequest`, `VolumeSetRequest`, `VolumeResponse`, `LocalVolumeResponse` |
| `your_library_config.proto` | `spotify.your_library.proto` | `YourLibraryLabelAndImage`, `YourLibraryPredefinedPlaylistConfig`, `YourLibraryPredefinedPlaylistConfigs`, `YourLibraryFilters`, `YourLibrarySortOrder` |
| `your_library_contains_request.proto` | `spotify.your_library.proto` | `YourLibraryContainsRequest` |
| `your_library_contains_response.proto` | `spotify.your_library.proto` | `YourLibraryContainsResponseHeader`, `YourLibraryContainsResponseEntity`, `YourLibraryContainsResponse` |
| `your_library_decorate_request.proto` | `spotify.your_library.proto` | `YourLibraryDecorateRequest` |
| `your_library_decorate_response.proto` | `spotify.your_library.proto` | `YourLibraryDecorateResponseHeader`, `YourLibraryDecorateResponse` |
| `your_library_decorated_entity.proto` | `spotify.your_library.proto` | `YourLibraryEntityInfo`, `Offline`, `YourLibraryAlbumExtraInfo`, `YourLibraryArtistExtraInfo`, `NumberOfItemsForLinkType`, `YourLibraryPlaylistFolderInfo`, `YourLibraryPlaylistExtraInfo`, `YourLibraryShowExtraInfo`, `YourLibraryFolderExtraInfo`, `YourLibraryNewEpisodesExtraInfo`, `YourLibraryLocalFilesExtraInfo`, `YourLibraryBookExtraInfo`, `YourLibraryPreReleaseExtraInfo`, `YourLibraryEventExtraInfo`, `YourLibraryAuthorExtraInfo`, `YourLibraryVenueExtraInfo`, `YourLibraryConcertCampaignExtraInfo`, `YourLibraryDecoratedEntity` |
| `your_library_entity.proto` | `spotify.your_library.proto` | `YourLibraryEntity` |
| `your_library_index.proto` | `spotify.your_library.proto` | `YourLibraryRootlistPlaylist`, `YourLibraryPredefinedPlaylist`, `YourLibraryRootlistFolder`, `YourLibraryRootlistPseudoPlaylist`, `YourLibraryRootlistEntity`, `YourLibraryPreReleaseEntity`, `YourLibraryEventEntity`, `YourLibraryVenueEntity`, `YourLibraryConcertCampaignEntity` |
| `your_library_request.proto` | `spotify.your_library.proto` | `YourLibraryTagFilter`, `CuratedItems`, `YourLibraryRequestHeader`, `YourLibraryRequest` |
| `your_library_response.proto` | `spotify.your_library.proto` | `YourLibraryTagInfo`, `YourLibraryResponseHeader`, `YourLibraryResponse` |
| `your_library_tags_backend.proto` | `spotify.your_library_tags.proto` | `GetSuggestionsRequest`, `GetSuggestionsResponse`, `Tag`, `Item` |
| `your_library_tags_request.proto` | `spotify.your_library_tags_esperanto.proto` | `DecoratedTag`, `DecoratedClass`, `YourLibraryTagsResponse`, `YourLibraryTagsRequest` |

</details>

<details>
<summary>Additional protobuf filename strings (145)</summary>

```text
-include/BoomboxPlaybackNonCriticalError.proto
-include/DesktopConnectPullResumeNonAuth.proto
-policy/playlist_album_decoration_policy.proto
-policy/playlist_track_decoration_policy.proto
-state_restore/context_player_restorable.proto
-state_restore/mft_fallback_page_history.proto
-state_restore/shuffle_order_event_rules.proto
.include/DesktopUIShellInteractionNonAuth.proto
.include/EventSenderInternalError2NonAuth.proto
.include/MetadataExtensionClientStatistic.proto
.include/ResumptionTraitResolutionFailure.proto
.policy/rootlist_folder_decoration_policy.proto
.policy/supported_link_types_in_playlists.proto
.state_restore/remove_banned_tracks_rules.proto
0include/MigrationOAuthClientCreatedNonAuth.proto
0include/StreamReportingStrictModeViolation.proto
0player_sequence/player/state/context_stack.proto
0policy/rootlist_playlist_decoration_policy.proto
0spotify/playback_platform/common/v1/client.proto
0spotify/playback_platform/common/v1/device.proto
0spotify/playback_platform/common/v1/locale.proto
1include/DesktopAuthenticationFailureNonAuth.proto
1include/OAuthRefreshGrantClientErrorNonAuth.proto
1spotify/audiobookcashier/v1/audiobook_price.proto
1spotify/playback_platform/context/v1/ledger.proto
1spotify/playback_platform/media/v1/requests.proto
2spotify/list/v1/model/attributes/ai_curation.proto
2spotify/playback_platform/common/v1/duration.proto
2spotify/playback_platform/context/v1/context.proto
2spotify/playback_platform/media/v1/manifests.proto
2state_restore/mft_rules_inject_filler_tracks.proto
3include/DesktopWidevineComponentUpdateNonAuth.proto
3spotify/list/v1/model/attributes/capabilities.proto
3spotify/list/v1/model/attributes/picture_size.proto
3spotify/list/v1/model/attributes/signals_list.proto
3spotify/playback_platform/context/v1/requests.proto
4spotify/playback_platform/context/v1/sequences.proto
4spotify/playback_platform/timeline/v1/timeline.proto
4spotify/playback_platform/timeline/v1/timespan.proto
5include/ConnectionQualityInstrumentationBuckets.proto
5include/DesktopWatchdogSessionExpirationNonAuth.proto
5include/OAuthTokenFileStorageClientErrorNonAuth.proto
5spotify/playback_platform/common/v1/restriction.proto
5spotify/playback_platform/timeline/v1/timepoint.proto
6player_sequence/player/state/context_stack_entry.proto
6spotify/playback_platform/common/v1/audio_format.proto
6spotify/playback_platform/common/v1/video_format.proto
7spotify/playback_platform/common/v1/cache_control.proto
7spotify/playback_platform/common/v1/request_error.proto
7spotify/playback_platform/context/v1/ledger_entry.proto
7spotify/playback_platform/context/v1/ledger_query.proto
8player_sequence/player/state/session_context_state.proto
8spotify/list/v1/model/additional_responses_options.proto
8spotify/list/v1/model/attributes/item_id_algorithm.proto
8spotify/metadata/contentratings/v1/content_ratings.proto
8spotify/playback_platform/context/v1/context_state.proto
9spotify/playback_platform/context/v1/behavior/queue.proto
9spotify/playback_platform/context/v1/behavior/skips.proto
9spotify/playback_platform/context/v1/behavior/start.proto
9spotify/playback_platform/context/v1/metadata/basic.proto
9spotify/playback_platform/context/v1/sequence_child.proto
9spotify/playback_platform/context/v1/sequence_patch.proto
9spotify/playback_platform/context/v1/sequence_query.proto
9spotify/playback_platform/context/v1/sequence_start.proto
Aspotify/playback_platform/timeline/v1/behavior/restrictions.proto
Bspotify/playback_platform/external/legacy/v1/legacy_metadata.proto
Espotify/playback_platform/context/v1/behavior/shuffle_algorithm.proto
Espotify/playback_platform/context/v1/behavior/shuffle_placement.proto
application/vnd.collection-v2.spotify.proto
collection_get_unplayed_episodes_request.proto
end-groudescriptor.proto
esperanto_options.proto
google/protobuf/internal_options.proto
home_extras_cache.proto
include/BoomboxPlaybackNonCriticalError.proto
include/ConnectionQualityInstrumentationBuckets.proto
include/DesktopAuthenticationFailureNonAuth.proto
include/DesktopConnectPullResumeNonAuth.proto
include/DesktopUIShellInteractionNonAuth.proto
include/DesktopWatchdogSessionExpirationNonAuth.proto
include/DesktopWidevineComponentUpdateNonAuth.proto
include/EventSenderInternalError2NonAuth.proto
include/MetadataExtensionClientStatistic.proto
include/MigrationOAuthClientCreatedNonAuth.proto
include/OAuthRefreshGrantClientErrorNonAuth.proto
include/OAuthTokenFileStorageClientErrorNonAuth.proto
include/ResumptionTraitResolutionFailure.proto
include/StreamReportingStrictModeViolation.proto
net/proto2/proto/descriptor.proto
player_sequence/player/state/context_stack.proto
player_sequence/player/state/context_stack_entry.proto
player_sequence/player/state/session_context_state.proto
policy/playlist_album_decoration_policy.proto
policy/playlist_track_decoration_policy.proto
policy/rootlist_folder_decoration_policy.proto
policy/rootlist_playlist_decoration_policy.proto
policy/supported_link_types_in_playlists.proto
spotify/audiobookcashier/v1/audiobook_price.proto
spotify/list/v1/model/additional_responses_options.proto
spotify/list/v1/model/attributes/ai_curation.proto
spotify/list/v1/model/attributes/capabilities.proto
spotify/list/v1/model/attributes/item_id_algorithm.proto
spotify/list/v1/model/attributes/picture_size.proto
spotify/list/v1/model/attributes/signals_list.proto
spotify/metadata/contentratings/v1/content_ratings.proto
spotify/playback_platform/common/v1/audio_format.proto
spotify/playback_platform/common/v1/cache_control.proto
spotify/playback_platform/common/v1/client.proto
spotify/playback_platform/common/v1/device.proto
spotify/playback_platform/common/v1/duration.proto
spotify/playback_platform/common/v1/locale.proto
spotify/playback_platform/common/v1/request_error.proto
spotify/playback_platform/common/v1/restriction.proto
spotify/playback_platform/common/v1/video_format.proto
spotify/playback_platform/context/v1/behavior/queue.proto
spotify/playback_platform/context/v1/behavior/shuffle_algorithm.proto
spotify/playback_platform/context/v1/behavior/shuffle_placement.proto
spotify/playback_platform/context/v1/behavior/skips.proto
spotify/playback_platform/context/v1/behavior/start.proto
spotify/playback_platform/context/v1/context.proto
spotify/playback_platform/context/v1/context_state.proto
spotify/playback_platform/context/v1/ledger.proto
spotify/playback_platform/context/v1/ledger_entry.proto
spotify/playback_platform/context/v1/ledger_query.proto
spotify/playback_platform/context/v1/metadata/basic.proto
spotify/playback_platform/context/v1/requests.proto
spotify/playback_platform/context/v1/sequence_child.proto
spotify/playback_platform/context/v1/sequence_patch.proto
spotify/playback_platform/context/v1/sequence_query.proto
spotify/playback_platform/context/v1/sequence_start.proto
spotify/playback_platform/context/v1/sequences.proto
spotify/playback_platform/external/legacy/v1/legacy_metadata.proto
spotify/playback_platform/media/v1/manifests.proto
spotify/playback_platform/media/v1/requests.proto
spotify/playback_platform/timeline/v1/behavior/restrictions.proto
spotify/playback_platform/timeline/v1/timeline.proto
spotify/playback_platform/timeline/v1/timepoint.proto
spotify/playback_platform/timeline/v1/timespan.proto
state_restore/context_player_restorable.proto
state_restore/mft_fallback_page_history.proto
state_restore/mft_rules_inject_filler_tracks.proto
state_restore/remove_banned_tracks_rules.proto
state_restore/shuffle_order_event_rules.proto
third_party/protobuf/cpp_features.proto
third_party/protobuf/internal_options.proto
```

</details>

<details>
<summary>Extended Metadata extension kinds (264)</summary>

| Enum value | Extension kind |
|---:|---|
| 0 | `UNKNOWN_EXTENSION` |
| 1 | `CANVAZ` |
| 2 | `STORYLINES` |
| 3 | `PODCAST_TOPICS` |
| 4 | `PODCAST_SEGMENTS` |
| 5 | `AUDIO_FILES` |
| 6 | `TRACK_DESCRIPTOR` |
| 7 | `PODCAST_COUNTER` |
| 8 | `ARTIST_V4` |
| 9 | `ALBUM_V4` |
| 10 | `TRACK_V4` |
| 11 | `SHOW_V4` |
| 12 | `EPISODE_V4` |
| 13 | `PODCAST_HTML_DESCRIPTION` |
| 14 | `PODCAST_QUOTES` |
| 15 | `USER_PROFILE` |
| 16 | `CANVAS_V1` |
| 17 | `SHOW_V4_BASE` |
| 18 | `SHOW_V4_EPISODES_ASSOC` |
| 19 | `TRACK_DESCRIPTOR_SIGNATURES` |
| 20 | `PODCAST_AD_SEGMENTS` |
| 21 | `EPISODE_TRANSCRIPTS` |
| 22 | `PODCAST_SUBSCRIPTIONS` |
| 23 | `EXTRACTED_COLOR` |
| 24 | `PODCAST_VIRALITY` |
| 25 | `IMAGE_SPARKLES_HACK` |
| 26 | `PODCAST_POPULARITY_HACK` |
| 27 | `AUTOMIX_MODE` |
| 28 | `CUEPOINTS` |
| 29 | `PODCAST_POLL` |
| 30 | `EPISODE_ACCESS` |
| 31 | `SHOW_ACCESS` |
| 32 | `PODCAST_QNA` |
| 33 | `CLIPS` |
| 34 | `SHOW_V5` |
| 35 | `EPISODE_V5` |
| 36 | `PODCAST_CTA_CARDS` |
| 37 | `PODCAST_RATING` |
| 38 | `DISPLAY_SEGMENTS` |
| 39 | `GREENROOM` |
| 40 | `USER_CREATED` |
| 41 | `SHOW_DESCRIPTION` |
| 42 | `SHOW_HTML_DESCRIPTION` |
| 43 | `SHOW_PLAYABILITY` |
| 44 | `EPISODE_DESCRIPTION` |
| 45 | `EPISODE_HTML_DESCRIPTION` |
| 46 | `EPISODE_PLAYABILITY` |
| 47 | `SHOW_EPISODES_ASSOC` |
| 48 | `CLIENT_CONFIG` |
| 49 | `PLAYLISTABILITY` |
| 50 | `AUDIOBOOK_V5` |
| 51 | `CHAPTER_V5` |
| 52 | `AUDIOBOOK_SPECIFICS` |
| 53 | `EPISODE_RANKING` |
| 54 | `HTML_DESCRIPTION` |
| 55 | `CREATOR_CHANNEL` |
| 56 | `AUDIOBOOK_PROVIDERS` |
| 57 | `PLAY_TRAIT` |
| 58 | `CONTENT_WARNING` |
| 59 | `IMAGE_CUE` |
| 60 | `STREAM_COUNT` |
| 61 | `AUDIO_ATTRIBUTES` |
| 62 | `NAVIGABLE_TRAIT` |
| 63 | `NEXT_BEST_EPISODE` |
| 64 | `AUDIOBOOK_PRICE` |
| 65 | `EXPRESSIVE_PLAYLISTS` |
| 66 | `DYNAMIC_SHOW_EPISODE` |
| 67 | `LIVE` |
| 68 | `SKIP_PLAYED` |
| 69 | `AD_BREAK_FREE_PODCASTS` |
| 70 | `ASSOCIATIONS` |
| 71 | `PLAYLIST_EVALUATION` |
| 72 | `CACHE_INVALIDATIONS` |
| 73 | `LIVESTREAM_ENTITY` |
| 74 | `SINGLE_TAP_REACTIONS` |
| 75 | `USER_COMMENTS` |
| 76 | `CLIENT_RESTRICTIONS` |
| 77 | `PODCAST_GUEST` |
| 78 | `PLAYABILITY` |
| 79 | `COVER_IMAGE` |
| 80 | `SHARE_TRAIT` |
| 81 | `INSTANCE_SHARING` |
| 82 | `ARTIST_TOUR` |
| 83 | `AUDIOBOOK_GENRE` |
| 84 | `CONCEPT` |
| 85 | `ORIGINAL_VIDEO` |
| 86 | `SMART_SHUFFLE` |
| 87 | `LIVE_EVENTS` |
| 88 | `AUDIOBOOK_RELATIONS` |
| 89 | `HOME_POC_BASECARD` |
| 90 | `AUDIOBOOK_SUPPLEMENTS` |
| 91 | `PAID_PODCAST_BANNER` |
| 92 | `FEWER_ADS` |
| 93 | `WATCH_FEED_SHOW_EXPLORER` |
| 94 | `TRACK_EXTRA_DESCRIPTORS` |
| 95 | `TRACK_EXTRA_AUDIO_ATTRIBUTES` |
| 96 | `TRACK_EXTENDED_CREDITS` |
| 97 | `SIMPLE_TRAIT` |
| 98 | `AUDIO_ASSOCIATIONS` |
| 99 | `VIDEO_ASSOCIATIONS` |
| 100 | `PLAYLIST_TUNER` |
| 101 | `ARTIST_VIDEOS_ENTRYPOINT` |
| 102 | `ALBUM_PRERELEASE` |
| 103 | `CONTENT_ALTERNATIVES` |
| 105 | `SNAPSHOT_SHARING` |
| 106 | `DISPLAY_SEGMENTS_COUNT` |
| 107 | `PODCAST_FEATURED_EPISODE` |
| 108 | `PODCAST_SPONSORED_CONTENT` |
| 109 | `PODCAST_EPISODE_TOPICS_LLM` |
| 110 | `PODCAST_EPISODE_TOPICS_KG` |
| 111 | `EPISODE_RANKING_POPULARITY` |
| 112 | `MERCH` |
| 113 | `COMPANION_CONTENT` |
| 114 | `WATCH_FEED_ENTITY_EXPLORER` |
| 115 | `ANCHOR_CARD_TRAIT` |
| 116 | `AUDIO_PREVIEW_PLAYBACK_TRAIT` |
| 117 | `VIDEO_PREVIEW_STILL_TRAIT` |
| 118 | `PREVIEW_CARD_TRAIT` |
| 119 | `SHORTCUTS_CARD_TRAIT` |
| 120 | `VIDEO_PREVIEW_PLAYBACK_TRAIT` |
| 121 | `COURSE_SPECIFICS` |
| 122 | `CONCERT` |
| 123 | `CONCERT_LOCATION` |
| 124 | `CONCERT_MARKETING` |
| 125 | `CONCERT_PERFORMERS` |
| 126 | `TRACK_PAIR_TRANSITION` |
| 127 | `CONTENT_TYPE_TRAIT` |
| 128 | `NAME_TRAIT` |
| 129 | `ARTWORK_TRAIT` |
| 130 | `RELEASE_DATE_TRAIT` |
| 131 | `CREDITS_TRAIT` |
| 132 | `RELEASE_URI_TRAIT` |
| 133 | `ENTITY_CAPPING` |
| 134 | `LESSON_SPECIFICS` |
| 135 | `CONCERT_OFFERS` |
| 136 | `TRANSITION_MAPS` |
| 137 | `ARTIST_HAS_CONCERTS` |
| 138 | `PRERELEASE` |
| 139 | `PLAYLIST_ATTRIBUTES_V2` |
| 140 | `LIST_ATTRIBUTES_V2` |
| 141 | `LIST_METADATA` |
| 142 | `LIST_TUNER_AUDIO_ANALYSIS` |
| 143 | `LIST_TUNER_CUEPOINTS` |
| 144 | `CONTENT_RATING_TRAIT` |
| 145 | `COPYRIGHT_TRAIT` |
| 146 | `SUPPORTED_BADGES` |
| 147 | `BADGES` |
| 148 | `PREVIEW_TRAIT` |
| 149 | `ROOTLISTABILITY_TRAIT` |
| 150 | `LOCAL_CONCERTS` |
| 151 | `RECOMMENDED_PLAYLISTS` |
| 152 | `POPULAR_RELEASES` |
| 153 | `RELATED_RELEASES` |
| 154 | `SHARE_RESTRICTIONS` |
| 155 | `CONCERT_OFFER` |
| 156 | `CONCERT_OFFER_PROVIDER` |
| 157 | `ENTITY_BOOKMARKS` |
| 158 | `PRIVACY_TRAIT` |
| 159 | `DUPLICATE_ITEMS_TRAIT` |
| 160 | `REORDERING_TRAIT` |
| 161 | `PODCAST_RESUMPTION_SEGMENTS` |
| 162 | `ARTIST_EXPRESSION_VIDEO` |
| 163 | `PRERELEASE_VIDEO` |
| 164 | `GATED_ENTITY_RELATIONS` |
| 165 | `RELATED_CREATORS_SECTION` |
| 166 | `CREATORS_APPEARS_ON_SECTION` |
| 167 | `PROMO_V1_TRAIT` |
| 168 | `SPEECHLESS_SHARE_CARD` |
| 169 | `TOP_PLAYABLES_SECTION` |
| 170 | `AUTO_LENS` |
| 171 | `PROMO_V3_TRAIT` |
| 172 | `TRACK_CONTENT_FILTER` |
| 173 | `HIGHLIGHTABILITY` |
| 174 | `LINK_CARD_WITH_IMAGE_TRAIT` |
| 175 | `TRACK_CLOUD_SECTION` |
| 176 | `EPISODE_TOPICS` |
| 177 | `VIDEO_THUMBNAIL` |
| 178 | `IDENTITY_TRAIT` |
| 179 | `VISUAL_IDENTITY_TRAIT` |
| 180 | `CONTENT_TYPE_V2_TRAIT` |
| 181 | `PREVIEW_PLAYBACK_TRAIT` |
| 182 | `CONSUMPTION_EXPERIENCE_TRAIT` |
| 183 | `PUBLISHING_METADATA_TRAIT` |
| 184 | `DETAILED_EVALUATION_TRAIT` |
| 185 | `ON_PLATFORM_REPUTATION_TRAIT` |
| 186 | `CREDITS_V2_TRAIT` |
| 187 | `HIGHLIGHT_PLAYABILITY_TRAIT` |
| 188 | `SHOW_EPISODE_LIST` |
| 189 | `AVAILABLE_RELEASES` |
| 190 | `PLAYLIST_DESCRIPTORS` |
| 191 | `LINK_CARD_WITH_ANIMATIONS_TRAIT` |
| 192 | `RECAP` |
| 193 | `AUDIOBOOK_COMPANION_CONTENT` |
| 194 | `THREE_OH_THREE_PLAY_TRAIT` |
| 195 | `ARTIST_WRAPPED_2024_VIDEO` |
| 196 | `CONTAINED_CONTENT_TYPES` |
| 197 | `CONTENT_CLASSIFICATION` |
| 198 | `CHAPTER_SPECIFICS` |
| 199 | `CREATOR_FAN_FUNDING` |
| 200 | `CREATOR_PLAYLISTS_SECTION` |
| 201 | `CREATOR_PINNED_ITEM` |
| 202 | `PODCAST_POLL_V2` |
| 203 | `CREATOR_APPEARS_ON_SECTION` |
| 204 | `ARTIST_CONCERTS` |
| 205 | `LIST_METADATA_V2` |
| 206 | `ARTIST_HEADLINE` |
| 207 | `EPISODE_AD_PLACEMENTS` |
| 208 | `VIDEO_SOCIAL_PROOF_POC_TRAIT` |
| 209 | `ARTIST_MUSIC_VIDEOS` |
| 210 | `ARTIST_LISTENING_PARTY` |
| 211 | `CONTENT_BY_OR_ABOUT` |
| 212 | `PLAYBACK_TRAIT` |
| 213 | `ARTIST_MUSIC_VIDEOS_UNMAPPED` |
| 214 | `ENTITY_CONNECTIONS` |
| 215 | `USER_PROFILE_V2` |
| 216 | `VIDEO_RELATIONS` |
| 217 | `BEATS` |
| 218 | `VOCAL_ACTIVITY` |
| 219 | `MIXABILITY` |
| 220 | `ENTITY_TYPE_TRAIT` |
| 221 | `CHAT_SHARE_PREVIEW` |
| 222 | `AUDIO_ATTRIBUTES_V2` |
| 223 | `WATCH_FEED_CATEGORIES_TRAIT` |
| 224 | `VENUE_ARTISTS_PREVIEW` |
| 225 | `MIX_STATE` |
| 226 | `WATCH_FEED_SEED_ITEM_TRAIT` |
| 227 | `SONG_DNA_ELIGIBILITY` |
| 228 | `LEARNING_MATERIAL` |
| 229 | `BANNER_WITH_ANIMATIONS` |
| 230 | `VENUE_LOCATION` |
| 231 | `TRANSCRIPT_SEARCH` |
| 232 | `SONGDNA_CREDITS` |
| 233 | `AUDIOBOOK_TO_PHYSICAL_BOOK_MAPPING` |
| 234 | `ARTIST_WRAPPED_2025_VIDEO` |
| 235 | `MIXABILITY_TRAIT` |
| 236 | `SONGDNA_ARTIST_FACTS` |
| 237 | `THREEBAND_WAVEFORMS` |
| 238 | `AUDIOBOOK_PARTNER_SALES` |
| 239 | `CONTENT_CAPABILITY_TRAIT` |
| 240 | `CONCERT_CAMPAIGN` |
| 241 | `CONCERT_CAMPAIGN_USER_STATE` |
| 242 | `CONCERT_CAMPAIGN_STRINGS` |
| 243 | `CONCERT_CAMPAIGN_ROUTINGS` |
| 244 | `TRANSITION_DATA` |
| 245 | `CHAT_RICH_MEDIA` |
| 246 | `CURATION_EXPERIENCE_TRAIT` |
| 247 | `COMMON_TRANSITION_POINT` |
| 248 | `PLAY_LINK_CARD_TRAIT` |
| 249 | `CONTENT_EXPERIENCE_TRAIT` |
| 250 | `TRACK_ALTERNATIVE_VERSIONS` |
| 251 | `LICENSING_IDENTIFIERS_TRAIT` |
| 252 | `SPONSORSHIP` |
| 253 | `AUDIOBOOK_DIRECT_SALES` |
| 254 | `ALBUM_MUSIC_VIDEOS` |
| 255 | `CREATOR_ARTIST_FACTS` |
| 256 | `PLAYBACK_EXPERIENCE_TRAIT` |
| 257 | `SPEEDABILITY` |
| 258 | `CONTENT_MARKERS` |
| 259 | `TRACK_AUDIO_FOUNDATIONS` |
| 260 | `AD_PLACEMENTS_BETA` |
| 261 | `TRACK_SECTIONS` |
| 262 | `SHARE_CARD_CONFIG` |
| 263 | `PLAYBACK_TRAIT_V2` |
| 264 | `TARGETED_OFFER_BANNER` |

</details>
