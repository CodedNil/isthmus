use super::{
    MusicResult, Session, SpotifyWorker,
    client::PlaylistResult,
    image_url, parse_id,
    proto::{
        list_read::Attributes,
        playlist_edit::{Add, Delta, Item, ListChanges, Operation, OperationKind, Remove},
    },
};
use crate::{
    app::send_update,
    music::{CondensedPlaylist, PlaylistId, TrackId},
};
use futures_util::future::join_all;
use std::{
    collections::{HashMap, HashSet},
    str,
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;
use tracing::{error, warn};

const RATING_PLAYLISTS: [&str; 10] = ["0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"];

fn rating_index(name: &str, ratings: bool) -> Option<u8> {
    ratings.then(|| RATING_PLAYLISTS.iter().position(|rating| *rating == name)).flatten().map(|index| index as u8)
}

#[derive(Clone)]
pub(super) struct CachedPlaylist {
    pub(super) revision: Vec<u8>,
    pub(super) tracks: Arc<HashSet<TrackId>>,
    pub(super) name: String,
}

impl CachedPlaylist {
    fn matches_revision(&self, revision: &[u8]) -> bool {
        !revision.is_empty() && self.revision == revision
    }
}

#[derive(Default)]
pub(super) struct PlaylistSnapshot {
    playlists: Vec<CondensedPlaylist>,
    cache: HashMap<PlaylistId, CachedPlaylist>,
    root_revision: Vec<u8>,
}

pub(super) enum PlaylistRead {
    Root(PlaylistSnapshot),
    Selected(Vec<PlaylistResult>),
}

impl SpotifyWorker {
    pub(super) fn needs_root_refresh(&self, revision: &[u8]) -> bool {
        revision.is_empty() || self.root_revision != revision
    }

    pub(super) fn playlist_refresh(
        &self,
        ids: Vec<PlaylistId>,
    ) -> impl Future<Output = MusicResult<PlaylistRead>> + use<> {
        let session = Arc::clone(&self.session);
        let targets = self.playlist_targets.clone();
        let ratings = self.ratings_enabled;
        let cache = Arc::clone(&self.cached_playlists);
        async move {
            timeout(Duration::from_secs(15), async {
                if ids.is_empty() {
                    Self::load_playlists(&session, &targets, ratings, &cache).await.map(PlaylistRead::Root)
                } else {
                    session.playlists(&ids).await.map(PlaylistRead::Selected)
                }
            })
            .await?
        }
    }

    fn invalidate_playlist(&mut self, id: PlaylistId) {
        if let Some(playlist) = Arc::make_mut(&mut self.cached_playlists).get_mut(&id) {
            playlist.revision.clear();
        }
    }

    pub(super) fn apply_playlist_read(
        &mut self,
        requested: &[(PlaylistId, Vec<u8>)],
        read: MusicResult<PlaylistRead>,
    ) -> bool {
        match read {
            Ok(PlaylistRead::Selected(results)) => self.apply_playlist_results(requested, Ok(results)),
            Ok(PlaylistRead::Root(PlaylistSnapshot { playlists, cache, root_revision })) => {
                tracing::debug!(playlists = playlists.len(), "Loaded Spotify playlists");
                self.cached_playlists = Arc::new(cache);
                self.root_revision = root_revision;
                send_update(&self.updater, move |app| {
                    app.music.playlists = playlists;
                    app.refresh_enrichment();
                });
                false
            }
            Err(error) if !requested.is_empty() => self.apply_playlist_results(requested, Err(error)),
            Err(error) => {
                warn!(%error, "Spotify library refresh failed");
                false
            }
        }
    }

    pub(super) fn needs_playlist_refresh(&self, id: PlaylistId, revision: &[u8]) -> bool {
        self.cached_playlists.get(&id).is_some_and(|cached| !cached.matches_revision(revision))
    }

    fn apply_playlist_results(
        &mut self,
        requested: &[(PlaylistId, Vec<u8>)],
        results: MusicResult<Vec<PlaylistResult>>,
    ) -> bool {
        let results = match results {
            Ok(results) => results,
            Err(error) => {
                warn!(%error, "Could not refresh Spotify playlists");
                for &(id, _) in requested {
                    self.invalidate_playlist(id);
                }
                return true;
            }
        };
        let mut refresh_root = false;
        for ((id, announced_revision), result) in requested.iter().zip(results) {
            let id = *id;
            match result {
                Ok((updated, Some(attributes)))
                    if announced_revision.is_empty() || updated.revision == *announced_revision =>
                {
                    refresh_root |= self.cached_playlists.get(&id).is_some_and(|cached| cached.name != updated.name);
                    let tracks = Arc::clone(&updated.tracks);
                    let artwork = playlist_image(&attributes);
                    Arc::make_mut(&mut self.cached_playlists).insert(id, updated);
                    send_update(&self.updater, move |app| {
                        if let Some(playlist) = app.music.playlists.iter_mut().find(|playlist| playlist.id == id) {
                            playlist.tracks = tracks;
                            if playlist.image_url != artwork {
                                playlist.image_url = artwork;
                                app.refresh_enrichment();
                            }
                        }
                    });
                }
                result => {
                    if let Err(error) = result {
                        warn!(%error, playlist_id = %id, "Could not read Spotify playlist");
                    }
                    self.invalidate_playlist(id);
                    refresh_root = true;
                }
            }
        }
        refresh_root
    }

    pub(super) fn update_library(
        &self,
        track_id: TrackId,
        playlists: &[(PlaylistId, bool)],
        liked: Option<bool>,
    ) -> impl Future<Output = (bool, Vec<PlaylistId>)> + use<> {
        let uri = format!("spotify:track:{track_id}");
        let mut refresh_needed = false;
        let mut edits: HashMap<_, Vec<_>> = HashMap::new();
        for &(playlist_id, add) in playlists {
            if !self.cached_playlists.contains_key(&playlist_id) {
                warn!(%playlist_id, "Spotify playlist is not loaded");
                refresh_needed = true;
                continue;
            }
            edits.entry(playlist_id).or_default().push(playlist_operation(&uri, add));
        }
        let session = Arc::clone(&self.session);
        let connection_id = self.connection_id.clone();
        let cache = Arc::clone(&self.cached_playlists);
        async move {
            let writes = edits.into_iter().map(|(id, ops)| {
                let (session, connection_id, cache) = (&session, &connection_id, &cache);
                async move {
                    let revision = cache[&id].revision.clone();
                    let request = ListChanges {
                        base_revision: Some(revision.clone()),
                        deltas: vec![Delta { base_version: Some(revision), ops }],
                    };
                    if let Err(error) = session.change_playlist(connection_id, id, &request).await {
                        error!(%error, playlist_id = %id, "Failed to update Spotify playlist");
                    }
                    // A transport error can arrive after Spotify has applied the write.
                    id
                }
            });
            let (edited, ()) = tokio::join!(join_all(writes), async {
                if let Some(should_like) = liked
                    && let Err(error) = session.write_liked_song(&connection_id, uri, should_like).await
                {
                    error!(%error, %track_id, "Failed to update Spotify library");
                }
            },);
            (refresh_needed, edited)
        }
    }

    async fn load_playlists(
        session: &Session,
        targets: &[String],
        ratings: bool,
        cache: &HashMap<PlaylistId, CachedPlaylist>,
    ) -> MusicResult<PlaylistSnapshot> {
        if targets.is_empty() && !ratings {
            return Ok(PlaylistSnapshot::default());
        }
        let (root, mut root_revision) = session.root_playlist().await?;
        let mut candidates: Vec<_> = root
            .items
            .iter()
            .filter_map(|item| {
                let item_uri = &item.attributes.as_ref()?.uri;
                let metadata = root.meta_items.get(item_uri)?;
                let uri = item_uri.strip_prefix("spotify:")?;
                let kind_and_id =
                    if let Some(named) = uri.strip_prefix("user:") { named.split_once(':')?.1 } else { uri };
                let id = parse_id(kind_and_id.strip_prefix("playlist:")?)?;
                let attributes = metadata.attributes.as_ref()?;
                let name = attributes.name.as_str();
                let rating_index = rating_index(name, ratings);
                if !targets.iter().any(|target| target == name) && rating_index.is_none() {
                    return None;
                }
                let revision = metadata.info.as_ref().map_or(&[][..], |info| info.revision.as_slice());
                let current = cache.get(&id).filter(|cached| cached.matches_revision(revision));
                Some((id, attributes, rating_index, current))
            })
            .collect();
        candidates.sort_unstable_by_key(|(_, attributes, ..)| attributes.name.as_str());
        let ids: Vec<_> =
            candidates.iter().filter_map(|&(id, _, _, current)| current.is_none().then_some(id)).collect();
        let mut fetched = session.playlists(&ids).await?.into_iter();
        let mut updated_cache = HashMap::new();
        let mut playlists = Vec::new();
        for (id, attributes, rating_index, current) in candidates {
            let mut snapshot = if let Some(cached) = current {
                cached.clone()
            } else {
                match fetched.next().expect("one snapshot result per requested playlist") {
                    Ok((snapshot, _)) => snapshot,
                    Err(error) => {
                        warn!(%error, playlist_id = %id, "Could not read Spotify playlist");
                        // Allow another notification to retry the incomplete refresh.
                        root_revision.clear();
                        let Some(previous) = cache.get(&id) else { continue };
                        previous.clone()
                    }
                }
            };
            playlists.push(CondensedPlaylist {
                id,
                image_url: playlist_image(attributes),
                tracks: Arc::clone(&snapshot.tracks),
                rating_index,
            });
            snapshot.name.clone_from(&attributes.name);
            updated_cache.insert(id, snapshot);
        }
        Ok(PlaylistSnapshot { playlists, cache: updated_cache, root_revision })
    }
}

fn playlist_operation(uri: &str, add: bool) -> Operation {
    let items = vec![Item { uri: uri.into() }];
    if add {
        Operation { kind: OperationKind::Add as i32, add: Some(Add { items, add_last: Some(true) }), remove: None }
    } else {
        Operation {
            kind: OperationKind::Remove as i32,
            add: None,
            remove: Some(Remove { items, items_as_key: Some(true) }),
        }
    }
}

fn playlist_image(attributes: &Attributes) -> Option<String> {
    attributes
        .picture_sizes
        .iter()
        .flat_map(|sizes| &sizes.picture_size)
        .rev()
        .map(|picture| picture.url.as_str())
        .find(|url| !url.is_empty())
        .map(image_url)
        .or_else(|| {
            let picture = &attributes.picture_file_id;
            let id = str::from_utf8(picture)
                .ok()
                .and_then(|picture| picture.strip_prefix("spotify:image:"))
                .map(str::to_owned)
                .or_else(|| (picture.len() == 20).then(|| hex::encode(picture)))?;
            Some(format!("https://i.scdn.co/image/{id}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use calloop::channel;

    #[tokio::test]
    async fn playlist_notices_deduplicate_cached_revisions() {
        let id = parse_id("37i9dQZF1DXcBWIGoYBM5M").unwrap();
        let (updater, _updates) = channel::channel();
        let mut worker = SpotifyWorker {
            root_revision: vec![1],
            ..SpotifyWorker::new(Arc::new(Session::new(reqwest::Client::new()).unwrap()), updater)
        };
        assert!(!worker.needs_root_refresh(&[1]));
        assert!(worker.needs_root_refresh(&[2]));
        assert!(worker.needs_root_refresh(&[]));
        assert!(!worker.needs_playlist_refresh(id, &[2]));
        Arc::make_mut(&mut worker.cached_playlists).insert(id, CachedPlaylist {
            revision: vec![1],
            tracks: Arc::default(),
            name: String::new(),
        });
        assert!(!worker.needs_playlist_refresh(id, &[1]));
        assert!(worker.needs_playlist_refresh(id, &[2]));
        assert!(worker.needs_playlist_refresh(id, &[]));
    }

    #[tokio::test]
    async fn playlist_snapshots_keep_successes_when_one_needs_recovery() {
        let first = parse_id("37i9dQZF1DXcBWIGoYBM5M").unwrap();
        let second = parse_id("37i9dQZF1DXcBWIGoYBM5N").unwrap();
        let track = parse_id("6vWfZZyIlODYPMEbsrOK3G").unwrap();
        let (updater, _updates) = channel::channel();
        let mut worker = SpotifyWorker::new(Arc::new(Session::new(reqwest::Client::new()).unwrap()), updater);
        let previous =
            CachedPlaylist { revision: vec![1], tracks: Arc::new(HashSet::from([track])), name: String::new() };
        Arc::make_mut(&mut worker.cached_playlists).extend([(first, previous.clone()), (second, previous.clone())]);
        let updated = CachedPlaylist { revision: vec![2], tracks: Arc::default(), ..previous.clone() };
        let snapshots = vec![Ok((updated, Some(Attributes::default()))), Ok((previous, Some(Attributes::default())))];
        assert!(worker.apply_playlist_results(&[(first, vec![2]), (second, vec![2])], Ok(snapshots)));
        assert_eq!(worker.cached_playlists[&first].revision, [2]);
        assert!(worker.cached_playlists[&first].tracks.is_empty());
        assert_eq!(worker.cached_playlists[&second].revision, [] as [u8; 0]);
        assert!(worker.cached_playlists[&second].tracks.contains(&track));
    }
}
