use super::{
    MusicResult, Session, SpotifyWorker, image_url, parse_id,
    protocol::{
        list_read::Attributes,
        playlist4_external::{
            Add, Delta, Item, ListChanges, Op, PlaylistModificationInfo, PlaylistWriteResult, Rem, op,
        },
    },
    track_id_from_uri,
};
use crate::{
    app::send_update,
    music::{CondensedPlaylist, PlaylistId, TrackId},
};
use protobuf::MessageField;
use std::{
    collections::{HashMap, HashSet},
    str,
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;
use tracing::{error, warn};

const RATING_PLAYLISTS: [&str; 10] = ["0.5", "1.0", "1.5", "2.0", "2.5", "3.0", "3.5", "4.0", "4.5", "5.0"];

#[derive(Clone)]
pub(super) struct CachedPlaylist {
    pub(super) revision: Vec<u8>,
    pub(super) items: Vec<String>,
}

impl CachedPlaylist {
    fn matches_revision(&self, revision: &[u8]) -> bool {
        !revision.is_empty() && self.revision == revision
    }

    fn membership(&self) -> HashSet<TrackId> {
        self.items.iter().filter_map(|uri| track_id_from_uri(uri)).collect()
    }

    fn apply(&self, change: &PlaylistModificationInfo) -> Option<Self> {
        if change.truncated_ops() {
            return None;
        }
        self.apply_operations(change.parent_revision(), change.new_revision(), &change.ops)
    }

    fn apply_operations(&self, parent: &[u8], revision: &[u8], operations: &[Op]) -> Option<Self> {
        if !self.matches_revision(parent) || revision.is_empty() || operations.is_empty() {
            return None;
        }
        let mut items = self.items.clone();
        for operation in operations {
            match operation.kind() {
                op::Kind::ADD => {
                    let add = operation.add.as_ref()?;
                    // Item-relative anchors require identities that the snapshot does not request.
                    let unknown = add.special_fields.unknown_fields();
                    if unknown.get(6).is_some() || unknown.get(7).is_some() {
                        return None;
                    }
                    let index = match (add.from_index, add.add_first(), add.add_last()) {
                        (Some(index), false, false) => usize::try_from(index).ok()?,
                        (None, true, false) => 0,
                        (None, false, true) => items.len(),
                        _ => return None,
                    };
                    if index > items.len() || add.items.is_empty() {
                        return None;
                    }
                    items.splice(index..index, add.items.iter().map(|item| item.uri().to_owned()));
                }
                op::Kind::REM => {
                    let remove = operation.rem.as_ref()?;
                    if let Some(index) = remove.from_index {
                        if remove.items_as_key() {
                            return None;
                        }
                        let start = usize::try_from(index).ok()?;
                        let length = remove.length.map_or(Ok(remove.items.len()), usize::try_from).ok()?;
                        let end = start.checked_add(length)?;
                        let removed = items.get(start..end)?;
                        if length == 0
                            || (!remove.items.is_empty()
                                && !removed.iter().map(String::as_str).eq(remove.items.iter().map(Item::uri)))
                        {
                            return None;
                        }
                        items.drain(start..end);
                    } else if remove.items_as_key() && !remove.items.is_empty() && remove.length.is_none() {
                        for item in &remove.items {
                            let mut matches = items.iter().enumerate().filter(|(_, uri)| *uri == item.uri());
                            let (index, _) = matches.next()?;
                            if matches.next().is_some() {
                                return None;
                            }
                            items.remove(index);
                        }
                    } else {
                        return None;
                    }
                }
                op::Kind::KIND_UNKNOWN => return None,
            }
        }
        Some(Self { revision: revision.to_vec(), items })
    }
}

#[derive(Default)]
pub(super) struct PlaylistSnapshot {
    playlists: Vec<CondensedPlaylist>,
    cache: HashMap<PlaylistId, CachedPlaylist>,
    root_revision: Vec<u8>,
}

pub(super) enum PlaylistChange {
    Ignore,
    Root,
    Selected(PlaylistId, Vec<u8>),
}

pub(super) enum PlaylistRead {
    Root(PlaylistSnapshot),
    Selected(PlaylistId, CachedPlaylist, Attributes),
}

fn track_only_edit(change: &PlaylistModificationInfo) -> bool {
    !change.truncated_ops()
        && !change.parent_revision().is_empty()
        && !change.new_revision().is_empty()
        && !change.ops.is_empty()
        && change.ops.iter().all(|operation| {
            operation.special_fields.unknown_fields().iter().next().is_none()
                && match operation.kind() {
                    op::Kind::ADD => {
                        operation.add.as_ref().is_some_and(|add| !add.items.is_empty()) && operation.rem.is_none()
                    }
                    op::Kind::REM => {
                        operation.rem.as_ref().is_some_and(|rem| !rem.items.is_empty() || rem.length() > 0)
                            && operation.add.is_none()
                    }
                    op::Kind::KIND_UNKNOWN => false,
                }
        })
}

impl SpotifyWorker {
    pub(super) fn playlist_refresh(
        &self,
        selected: Option<(PlaylistId, Vec<u8>)>,
    ) -> impl Future<Output = MusicResult<PlaylistRead>> + use<> {
        let session = self.session.clone();
        let targets = self.playlist_targets.clone();
        let ratings = self.ratings_enabled;
        let cache = Arc::clone(&self.cached_playlists);
        async move {
            timeout(Duration::from_secs(15), async {
                let Some((id, revision)) = selected else {
                    return Self::load_playlists(&session, &targets, ratings, &cache).await.map(PlaylistRead::Root);
                };
                match session.playlists(&[id], true).await?.pop() {
                    Some((snapshot, Some(attributes))) if snapshot.revision == revision => {
                        Ok(PlaylistRead::Selected(id, snapshot, attributes))
                    }
                    _ => Err("Spotify playlist snapshot did not match the Dealer revision".into()),
                }
            })
            .await?
        }
    }

    pub(super) fn invalidate_playlist(&mut self, id: PlaylistId) {
        if let Some(playlist) = Arc::make_mut(&mut self.cached_playlists).get_mut(&id) {
            playlist.revision.clear();
        }
    }

    pub(super) fn apply_playlist_read(&mut self, read: PlaylistRead) {
        match read {
            PlaylistRead::Selected(id, snapshot, attributes) => {
                self.store_playlist(id, snapshot, Some(&attributes));
            }
            PlaylistRead::Root(PlaylistSnapshot { playlists, cache, root_revision }) => {
                tracing::debug!(playlists = playlists.len(), "Loaded Spotify playlists");
                self.cached_playlists = Arc::new(cache);
                self.root_revision = root_revision;
                send_update(&self.updater, move |app| {
                    app.music.playlists = playlists;
                    app.refresh_enrichment();
                });
            }
        }
    }

    pub(super) fn playlist_change(&mut self, change: &PlaylistModificationInfo, can_apply: bool) -> PlaylistChange {
        if !can_apply {
            return PlaylistChange::Root;
        }
        let Ok(uri) = str::from_utf8(change.uri()) else { return PlaylistChange::Root };
        let Some(id) = uri.strip_prefix("spotify:playlist:").and_then(parse_id) else {
            return if !self.root_revision.is_empty()
                && self.root_revision == change.new_revision()
                && uri == format!("spotify:user:{}:rootlist", self.session.username())
            {
                PlaylistChange::Ignore
            } else {
                PlaylistChange::Root
            };
        };
        if let Some(cached) = self.cached_playlists.get(&id) {
            if cached.matches_revision(change.new_revision()) {
                return PlaylistChange::Ignore;
            }
            if let Some(updated) = cached.apply(change) {
                self.store_playlist(id, updated, None);
                return PlaylistChange::Ignore;
            }
        }
        if !track_only_edit(change) {
            return PlaylistChange::Root;
        }
        if !self.cached_playlists.contains_key(&id) {
            return PlaylistChange::Ignore;
        }
        PlaylistChange::Selected(id, change.new_revision().to_vec())
    }

    fn store_playlist(&mut self, id: PlaylistId, updated: CachedPlaylist, attributes: Option<&Attributes>) {
        let tracks = updated.membership();
        let artwork = attributes.map(playlist_image);
        Arc::make_mut(&mut self.cached_playlists).insert(id, updated);
        send_update(&self.updater, move |app| {
            if let Some(playlist) = app.music.playlists.iter_mut().find(|playlist| playlist.id == id) {
                playlist.tracks = tracks;
                if let Some(image_url) = artwork {
                    playlist.image_url = image_url;
                    app.refresh_enrichment();
                }
            }
        });
    }

    pub(super) async fn update_library(
        &mut self,
        track_id: TrackId,
        playlists: &[(PlaylistId, bool)],
        liked: Option<bool>,
    ) -> bool {
        let uri = format!("spotify:track:{track_id}");
        let mut refresh_needed = false;
        let mut resync = Vec::new();
        for &(playlist_id, add) in playlists {
            let Some(cached) = self.cached_playlists.get(&playlist_id) else {
                warn!(%playlist_id, "Spotify playlist is not loaded");
                refresh_needed = true;
                continue;
            };
            let revision = cached.revision.clone();
            let operation = playlist_operation(&uri, add);
            let request = ListChanges {
                base_revision: Some(revision.clone()),
                deltas: vec![Delta {
                    base_version: Some(revision.clone()),
                    ops: vec![operation.clone()],
                    ..Default::default()
                }],
                want_resulting_revisions: Some(true),
                want_sync_result: Some(true),
                ..Default::default()
            };
            match self.session.change_playlist(playlist_id, &request).await {
                Ok(result) => {
                    let receipt = result.as_ref().and_then(|result| {
                        Some((result.changes_require_resync?, committed_playlist_revision(result)?))
                    });
                    if let Some((false, new_revision)) = receipt
                        && let Some(updated) = cached.apply_operations(&revision, new_revision, &[operation])
                    {
                        self.store_playlist(playlist_id, updated, None);
                        continue;
                    }
                    Arc::make_mut(&mut self.cached_playlists).remove(&playlist_id);
                    if let Some((_, revision)) = receipt {
                        resync.push((playlist_id, revision.to_vec()));
                    } else {
                        refresh_needed = true;
                    }
                }
                Err(error) => {
                    error!(%error, %playlist_id, "Failed to update Spotify playlist");
                    refresh_needed = true;
                }
            }
        }

        if !resync.is_empty() {
            let ids: Vec<_> = resync.iter().map(|(id, _)| *id).collect();
            match self.session.playlists(&ids, true).await {
                Ok(snapshots) => {
                    for ((id, revision), (snapshot, attributes)) in resync.into_iter().zip(snapshots) {
                        if let Some(attributes) = attributes.filter(|_| snapshot.revision == revision) {
                            self.store_playlist(id, snapshot, Some(&attributes));
                        } else {
                            refresh_needed = true;
                        }
                    }
                }
                Err(error) => {
                    warn!(%error, "Could not refresh edited Spotify playlists");
                    refresh_needed = true;
                }
            }
        }
        if let Some(should_like) = liked
            && let Err(error) = self.session.write_liked_song(uri, should_like).await
        {
            error!(%error, %track_id, "Failed to update Spotify library");
        }
        refresh_needed
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
        let (root, root_revision) = session.root_playlist().await?;
        let mut playlists: Vec<_> = root
            .items
            .iter()
            .filter_map(|item| {
                let item_uri = &item.attributes.get_or_default().uri;
                let metadata = root.meta_items.get(item_uri)?;
                let uri = item_uri.strip_prefix("spotify:")?;
                let kind_and_id =
                    if let Some(named) = uri.strip_prefix("user:") { named.split_once(':')?.1 } else { uri };
                let id = parse_id(kind_and_id.strip_prefix("playlist:")?)?;
                let attributes = metadata.attributes.get_or_default();
                let name = attributes.name.as_str();
                let rating_index = RATING_PLAYLISTS
                    .iter()
                    .position(|rating| *rating == name)
                    .filter(|_| ratings)
                    .map(|index| index as u8);
                if !targets.iter().any(|target| target == name) && rating_index.is_none() {
                    return None;
                }
                Some((
                    CondensedPlaylist {
                        id,
                        image_url: playlist_image(attributes),
                        tracks: HashSet::new(),
                        rating_index,
                    },
                    metadata.info.get_or_default().revision.clone(),
                    name,
                ))
            })
            .collect();
        let mut pending = Vec::new();
        let mut updated_cache = HashMap::new();
        for (index, (playlist, revision, _)) in playlists.iter_mut().enumerate() {
            if let Some(cached) = cache.get(&playlist.id).filter(|cached| cached.matches_revision(revision)) {
                playlist.tracks = cached.membership();
                updated_cache.insert(playlist.id, cached.clone());
            } else {
                pending.push((index, playlist.id));
            }
        }
        let ids: Vec<_> = pending.iter().map(|&(_, id)| id).collect();
        for ((index, id), (cached, _)) in pending.into_iter().zip(session.playlists(&ids, false).await?) {
            playlists[index].0.tracks = cached.membership();
            updated_cache.insert(id, cached);
        }
        playlists.sort_unstable_by_key(|(_, _, name)| *name);
        let playlists = playlists.into_iter().map(|(playlist, ..)| playlist).collect();
        Ok(PlaylistSnapshot { playlists, cache: updated_cache, root_revision })
    }
}

fn playlist_operation(uri: &str, add: bool) -> Op {
    let items = vec![Item { uri: Some(uri.into()), ..Default::default() }];
    let mut operation = Op::new();
    if add {
        operation.set_kind(op::Kind::ADD);
        operation.add = MessageField::some(Add { items, add_last: Some(true), ..Default::default() });
    } else {
        operation.set_kind(op::Kind::REM);
        operation.rem = MessageField::some(Rem { items, items_as_key: Some(true), ..Default::default() });
    }
    operation
}

fn committed_playlist_revision(result: &PlaylistWriteResult) -> Option<&[u8]> {
    let revision = result.revision();
    let [resulting] = result.resulting_revisions.as_slice() else { return None };
    let sync = result.sync_result.as_ref()?;
    // An empty sync result from the committed revision to itself means no merge was needed.
    (!revision.is_empty()
        && revision == resulting
        && revision == sync.from_revision()
        && revision == sync.to_revision()
        && !result.multiple_heads()
        && sync.ops.is_empty())
    .then_some(revision)
}

fn playlist_image(attributes: &Attributes) -> Option<String> {
    attributes
        .picture_sizes
        .get_or_default()
        .picture_size
        .iter()
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

    const REQUESTED: &str = "spotify:track:6vWfZZyIlODYPMEbsrOK3G";

    #[tokio::test]
    async fn membership_pushes_only_refresh_selected_playlists() {
        let id = parse_id("37i9dQZF1DXcBWIGoYBM5M").unwrap();
        let (updater, _updates) = channel::channel();
        let mut worker = SpotifyWorker {
            ratings_enabled: true,
            root_revision: vec![1],
            ..SpotifyWorker::new(Session::new().unwrap(), updater)
        };
        let mut change = PlaylistModificationInfo {
            uri: Some(format!("spotify:playlist:{id}").into_bytes()),
            parent_revision: Some(vec![2]),
            new_revision: Some(vec![3]),
            ops: vec![playlist_operation(REQUESTED, true)],
            ..Default::default()
        };
        assert!(matches!(worker.playlist_change(&change, true), PlaylistChange::Ignore));
        assert!(matches!(worker.playlist_change(&change, false), PlaylistChange::Root));
        Arc::make_mut(&mut worker.cached_playlists).insert(id, CachedPlaylist { revision: vec![1], items: vec![] });
        assert!(matches!(worker.playlist_change(&change, true), PlaylistChange::Selected(target, revision)
            if target == id && revision == [3]));
        change.truncated_ops = Some(true);
        assert!(matches!(worker.playlist_change(&change, true), PlaylistChange::Root));
    }

    #[test]
    fn playlist_write_receipt_requires_a_clean_sync_result() {
        use super::super::protocol::playlist4_external::Diff;

        let mut result = PlaylistWriteResult {
            revision: Some(vec![2]),
            resulting_revisions: vec![vec![2]],
            sync_result: MessageField::some(Diff {
                from_revision: Some(vec![2]),
                to_revision: Some(vec![2]),
                ..Default::default()
            }),
            multiple_heads: Some(false),
            ..Default::default()
        };
        assert_eq!(committed_playlist_revision(&result), Some([2].as_slice()));
        result.sync_result.mut_or_insert_default().from_revision = Some(vec![1]);
        assert!(committed_playlist_revision(&result).is_none());
    }

    #[test]
    fn playlist_push_preserves_duplicates_and_rejects_incomplete_edits() {
        let cached = CachedPlaylist { revision: vec![1], items: vec![REQUESTED.into()] };
        let mut change = PlaylistModificationInfo {
            parent_revision: Some(vec![1]),
            new_revision: Some(vec![2]),
            ops: vec![playlist_operation(REQUESTED, true)],
            ..Default::default()
        };
        let added = cached.apply(&change).unwrap();
        assert_eq!(added.items, [REQUESTED, REQUESTED]);
        assert_eq!(added.revision, [2]);
        assert!(added.apply(&change).is_none());

        change.parent_revision = Some(vec![2]);
        change.new_revision = Some(vec![3]);
        let mut remove = Op::new();
        remove.set_kind(op::Kind::REM);
        let range = remove.rem.mut_or_insert_default();
        range.set_from_index(0);
        range.set_length(1);
        change.ops = vec![remove];
        let removed = added.apply(&change).unwrap();
        assert_eq!(removed.membership(), cached.membership());
        assert_eq!(removed.items.len(), 1);

        change.truncated_ops = Some(true);
        assert!(added.apply(&change).is_none());
        change.truncated_ops = None;
        change.ops = vec![playlist_operation(REQUESTED, false)];
        assert!(added.apply(&change).is_none());
    }
}
