//! Filesystem-driven metadata snapshots and deferred icon loading.

use super::spawn_thread;
use crate::{
    app::{AppUpdater, send_update},
    catalog::AppCatalog,
    platform::{DesktopApp, decode_icon},
};
use freedesktop_desktop_entry::{Iter, default_paths, get_languages_from_env};
use isthmus::Image;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs, mem,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};
use tokio::task::spawn_blocking;
use tracing::warn;

const DEBOUNCE: Duration = Duration::from_millis(120);
// Reconcile missed notifications, symlinked files, and unavailable watches.
const RECONCILE: Duration = Duration::from_secs(30);

pub fn start_app_catalog(updater: &AppUpdater) {
    let updater = updater.clone();
    spawn_thread("cantus-app-catalog", move || {
        let roots: Vec<_> = default_paths().collect();
        monitor(&updater, &roots);
    });
}

fn monitor(updater: &AppUpdater, roots: &[PathBuf]) {
    let (sender, events) = mpsc::sync_channel(1);
    // Register before scanning so changes during the initial scan remain queued.
    let mut watcher = watch(roots, &sender);
    let mut previous = None;
    loop {
        let apps = scan_desktop_apps(roots);
        let sent = if previous.as_ref() == Some(&apps) {
            send_update(updater, |app| app.launcher.refresh_icons())
        } else {
            previous = Some(apps.clone());
            let catalog = AppCatalog::new(apps);
            send_update(updater, move |app| app.launcher.set_catalog(catalog))
        };
        if !sent {
            break;
        }
        match events.recv_timeout(RECONCILE) {
            Ok(()) => {
                thread::sleep(DEBOUNCE);
                while events.try_recv().is_ok() {}
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Rearm even when a path looks the same: a directory or profile may have
        // been atomically replaced, leaving inotify attached to the previous inode.
        let replacement = watch(roots, &sender);
        drop(mem::replace(&mut watcher, replacement));
    }
}

fn watch(roots: &[PathBuf], sender: &mpsc::SyncSender<()>) -> Option<RecommendedWatcher> {
    let mut paths = BTreeMap::new();
    let mut dependencies = HashSet::new();
    let mut pending: VecDeque<_> = roots.iter().cloned().collect();
    // Include the parents and targets of profile symlinks, so replacing a Nix
    // generation or creating a previously missing applications folder wakes us.
    while let Some(path) = pending.pop_front() {
        if dependencies.len() >= 128 || !dependencies.insert(path.clone()) {
            continue;
        }
        if let Ok(canonical) = path.canonicalize()
            && canonical.is_dir()
        {
            dependencies.insert(canonical.clone());
            paths.insert(canonical, RecursiveMode::Recursive);
        }
        for ancestor in path.ancestors() {
            if let Ok(target) = fs::read_link(ancestor) {
                let target = ancestor.parent().unwrap_or_else(|| Path::new("/")).join(target);
                pending.push_back(target.join(path.strip_prefix(ancestor).unwrap()));
            }
            if let Some(parent) = ancestor.parent().filter(|parent| parent.parent().is_some())
                && let Ok(canonical) = parent.canonicalize()
            {
                paths.entry(canonical).or_insert(RecursiveMode::NonRecursive);
            }
        }
    }
    let sender = sender.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        let changed = match event {
            Ok(event) => {
                !matches!(event.kind, EventKind::Access(_))
                    && (event.need_rescan()
                        || event.paths.is_empty()
                        || event.paths.iter().any(|path| {
                            dependencies.iter().any(|root| path.starts_with(root) || root.starts_with(path))
                        }))
            }
            Err(_) => true,
        };
        if changed {
            let _ = sender.try_send(());
        }
    })
    .map_err(|error| warn!(%error, "Cannot watch applications; using periodic refresh"))
    .ok()?;
    for (path, mode) in paths {
        if let Err(error) = watcher.watch(&path, mode) {
            warn!(%error, ?path, "Cannot watch application directory; periodic refresh remains active");
        }
    }
    Some(watcher)
}

fn scan_desktop_apps(roots: &[PathBuf]) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    let locales = get_languages_from_env();
    Iter::new(roots.iter().cloned())
        .entries(Some(&locales))
        .filter(|entry| seen.insert(entry.id().to_owned()))
        .filter(|entry| !entry.no_display() && !entry.hidden() && !entry.terminal())
        .filter_map(|entry| {
            let exec = entry.parse_exec().ok()?;
            let mut search_terms = vec![entry.appid.clone()];
            for languages in [locales.as_slice(), &[]] {
                search_terms.extend(entry.name(languages).map(std::borrow::Cow::into_owned));
                search_terms.extend(entry.generic_name(languages).map(std::borrow::Cow::into_owned));
                search_terms.extend(
                    entry.keywords(languages).unwrap_or_default().into_iter().map(std::borrow::Cow::into_owned),
                );
            }
            search_terms.extend(entry.startup_wm_class().map(str::to_owned));
            if let Some(program) = exec.first().and_then(|s| Path::new(s).file_name()).and_then(|s| s.to_str()) {
                search_terms.push(program.to_owned());
            }
            search_terms.sort();
            search_terms.dedup();
            let action = entry.actions().and_then(|actions| {
                let action = actions.into_iter().find(|action| !action.is_empty())?;
                let name = entry.action_entry_localized(action, "Name", &locales)?;
                Some((name.into_owned(), entry.parse_exec_action(action).ok()?))
            });
            Some(DesktopApp {
                id: entry.id().to_owned(),
                name: entry.name(&locales)?.into_owned(),
                search_terms,
                exec,
                comment: entry.comment(&locales).unwrap_or_default().into_owned(),
                action,
                icon: entry.icon().map(str::to_owned),
            })
        })
        .collect()
}

pub async fn load_desktop_icons(sources: Vec<String>) -> Vec<(String, Option<Image>)> {
    spawn_blocking(move || {
        sources
            .into_iter()
            .map(|source| {
                let path = if Path::new(&source).is_absolute() {
                    Some(PathBuf::from(&source))
                } else {
                    freedesktop_icons::lookup(&source).with_size(64).find()
                };
                let image = path.and_then(|path| fs::read(path).ok()).and_then(|bytes| decode_icon(&bytes));
                (source, image)
            })
            .collect()
    })
    .await
    .expect("desktop icon loader panicked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, os::unix::fs::symlink, process};

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("cantus-catalog-{}-{:x}", process::id(), fastrand::u64(..)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn desktop(path: &Path, name: &str, hidden: bool) {
        fs::write(
            path,
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=steam\nHidden={hidden}\nIcon=steam\n"),
        )
        .unwrap();
    }

    #[test]
    fn follows_and_rearms_profile_symlink_replacements() {
        let directory = Directory::new();
        let profiles = directory.0.join("profiles");
        fs::create_dir_all(&profiles).unwrap();
        for (generation, name) in [("one", "Old Steam"), ("two", "New Steam")] {
            let apps = directory.0.join(generation).join("share/applications");
            fs::create_dir_all(&apps).unwrap();
            desktop(&apps.join("steam.desktop"), name, false);
        }
        symlink(directory.0.join("one"), profiles.join("current")).unwrap();
        symlink(profiles.join("current"), directory.0.join("profile")).unwrap();
        let roots = vec![directory.0.join("profile/share/applications")];
        let (sender, events) = mpsc::sync_channel(1);
        let first = watch(&roots, &sender).unwrap();
        assert_eq!(scan_desktop_apps(&roots)[0].name, "Old Steam");
        symlink(directory.0.join("two"), profiles.join("next")).unwrap();
        fs::rename(profiles.join("next"), profiles.join("current")).unwrap();
        events.recv_timeout(Duration::from_secs(5)).expect("missing profile replacement notification");
        let replacement = watch(&roots, &sender).unwrap();
        drop(first);
        assert_eq!(scan_desktop_apps(&roots)[0].name, "New Steam");
        while events.try_recv().is_ok() {}
        desktop(&directory.0.join("two/share/applications/steam.desktop"), "Edited Steam", false);
        events.recv_timeout(Duration::from_secs(5)).expect("missing notification from new generation");
        assert_eq!(scan_desktop_apps(&roots)[0].name, "Edited Steam");
        drop(replacement);
    }
}
