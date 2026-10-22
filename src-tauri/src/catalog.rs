use crate::files;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use tauri::Emitter;

#[derive(Clone, PartialEq, Serialize)]
struct CatalogNames {
    maps: Vec<String>,
    mods: Vec<String>,
}

impl CatalogNames {
    fn read(root: &Path) -> Self {
        Self {
            maps: files::names_with_extension(&root.join("map_source"), "map"),
            mods: files::directories(&root.join("mods")),
        }
    }
}

/// Only watch directory entries, not every asset inside every mod. Watching the
/// root too lets us attach when mods/map_source is created after Launcher opens.
fn update_watches(watcher: &mut RecommendedWatcher, root: &Path, watched: &mut BTreeSet<PathBuf>) {
    let wanted: BTreeSet<PathBuf> = [root.to_owned(), root.join("mods"), root.join("map_source")]
        .into_iter()
        .filter(|path| path.is_dir())
        .collect();

    for path in watched.difference(&wanted) {
        let _ = watcher.unwatch(path);
    }
    watched.retain(|path| wanted.contains(path));

    for path in wanted {
        if !watched.contains(&path) {
            match watcher.watch(&path, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watched.insert(path);
                }
                Err(error) => {
                    eprintln!("Cannot watch {}: {error}", path.display());
                }
            }
        }
    }
}

pub fn start(app: tauri::AppHandle, root: PathBuf) -> notify::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(sender)?;
    let mut watched = BTreeSet::new();
    update_watches(&mut watcher, &root, &mut watched);
    let mut previous = CatalogNames::read(&root);

    std::thread::spawn(move || {
        // Group bursts from map/mod creators into one list update. The worker
        // never saves settings, reloads a selected mod or starts a build.
        let mut changed = false;
        loop {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(event) => {
                    changed = true;
                    if let Err(error) = event {
                        eprintln!("Launcher folder watcher: {error}");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if changed {
                        changed = false;
                        update_watches(&mut watcher, &root, &mut watched);
                        let current = CatalogNames::read(&root);
                        if current != previous {
                            previous = current.clone();
                            let _ = app.emit("launcher-catalog-changed", current);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    });

    Ok(())
}
