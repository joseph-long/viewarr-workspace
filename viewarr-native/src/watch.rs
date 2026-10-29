//! Noticing when an open file changes on disk.
//!
//! Folders are watched rather than the files themselves. Plenty of writers
//! replace a file instead of rewriting it — save to a temporary name, then
//! rename over the original — and a watch on the original would follow the
//! old file away and never see the new one. A folder sees both kinds.
//!
//! Events are not acted on as they arrive. Writing a file of any size raises
//! a burst of them, and reading partway through would show half an image or
//! fail outright, so a file is only reported once it has been quiet for
//! [`SETTLE`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long a file must go without changing before it is taken to be written.
const SETTLE: Duration = Duration::from_millis(300);

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    /// Folders currently watched.
    folders: HashSet<PathBuf>,
    /// Files that changed and have not settled yet, with when they last did.
    unsettled: HashMap<PathBuf, Instant>,
}

impl FileWatcher {
    /// `None` when the platform will not give us a watcher at all, in which
    /// case files simply are not followed.
    pub fn new() -> Option<Self> {
        let (sender, events) = channel();
        let watcher = notify::recommended_watcher(sender)
            .map_err(|err| eprintln!("viewarr: not following files for changes: {err}"))
            .ok()?;
        Some(Self {
            watcher,
            events,
            folders: HashSet::new(),
            unsettled: HashMap::new(),
        })
    }

    /// Watch exactly the folders holding `files`, adding and dropping watches
    /// as tabs come and go. Paths should already be canonical, since events
    /// report them that way.
    pub fn follow<'a>(&mut self, files: impl IntoIterator<Item = &'a Path>) {
        let wanted: HashSet<PathBuf> = files
            .into_iter()
            .filter_map(|file| file.parent().map(Path::to_path_buf))
            .collect();
        for folder in self.folders.difference(&wanted) {
            // It may be gone already, and there is nothing to do if so.
            let _ = self.watcher.unwatch(folder);
        }
        self.folders.retain(|folder| wanted.contains(folder));
        for folder in wanted {
            if self.folders.contains(&folder) {
                continue;
            }
            match self.watcher.watch(&folder, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.folders.insert(folder);
                }
                Err(err) => eprintln!(
                    "viewarr: not following {} for changes: {err}",
                    folder.display()
                ),
            }
        }
    }

    /// Files that changed and have since been quiet long enough to read. A
    /// path here is anything in a watched folder; it is up to the caller to
    /// ignore the ones it has no tab for.
    pub fn settled(&mut self) -> Vec<PathBuf> {
        let now = Instant::now();
        for event in self.events.try_iter().flatten() {
            // Our own reads raise access events on some platforms; reacting to
            // them would reload forever.
            if matches!(event.kind, EventKind::Access(_)) {
                continue;
            }
            for path in event.paths {
                self.unsettled.insert(path, now);
            }
        }
        let settled: Vec<PathBuf> = self
            .unsettled
            .iter()
            .filter(|(_, &changed)| now.duration_since(changed) >= SETTLE)
            .map(|(path, _)| path.clone())
            .collect();
        for path in &settled {
            self.unsettled.remove(path);
        }
        settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Real events from the real platform watcher, so this checks what the
    /// rest cannot: that a rewrite is noticed at all, and that the path it is
    /// reported under is the resolved one tabs compare against — on macOS the
    /// temporary folder sits behind the `/var` -> `/private/var` symlink.
    #[test]
    fn a_rewritten_file_is_reported_once_it_settles() {
        let dir = std::env::temp_dir().join(format!("viewarr-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.fits");
        std::fs::write(&file, b"before").unwrap();
        let file = file.canonicalize().unwrap();

        let mut watcher = FileWatcher::new().expect("this platform has a watcher");
        watcher.follow([file.as_path()]);
        // Give the platform a moment to start delivering for the new watch.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&file, b"after").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while Instant::now() < deadline && !seen {
            seen = watcher.settled().contains(&file);
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(seen, "no settled change reported for {}", file.display());
    }
}
