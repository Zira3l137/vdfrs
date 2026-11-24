//! Brings together the paths of one multi-selection in Windows Explorer.
//!
//! A context menu command registered in the registry starts one process per selected item, each
//! with one path. Packing ten files into one archive needs all ten paths in one process, so every
//! process drops its path in a spool folder, and the first to claim the folder waits until the
//! others have stopped arriving, then takes them all. The others exit.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

/// How long the spool must stay unchanged before the first process takes what is in it. Explorer
/// starts the processes of one selection within a few milliseconds of each other.
const QUIET: Duration = Duration::from_millis(400);
/// The longest the first process waits for the others, however many keep coming.
const MAX_WAIT: Duration = Duration::from_secs(5);
/// A claim older than this belongs to a process that died; the next one takes over.
const STALE: Duration = Duration::from_secs(30);

/// Returns every path of the selection to the first process of it, and `None` to the others,
/// which have nothing left to do. `command` keeps different commands' selections apart.
pub fn gather(command: &str, paths: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
    let root = std::env::temp_dir().join("vdfrs-gather");
    gather_in(&root, command, paths, QUIET)
}

fn gather_in(
    root: &Path,
    command: &str,
    paths: Vec<PathBuf>,
    quiet: Duration,
) -> Option<Vec<PathBuf>> {
    let Some(first) = paths.first() else { return Some(paths) };
    // One selection is in one folder, so the folder and the command name its spool.
    let mut hasher = DefaultHasher::new();
    (command, first.parent()).hash(&mut hasher);
    let spool = root.join(format!("{:016x}", hasher.finish()));

    let written = fs::create_dir_all(&spool).and_then(|()| {
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let name = format!("{:024}-{}", stamp.as_nanos(), std::process::id());
        let list: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        // Written under another name and then renamed, so the first process never reads half.
        let part = spool.join(format!("{name}.part"));
        fs::write(&part, list.join("\n"))?;
        fs::rename(&part, spool.join(name))
    });
    // Without a spool, each process carries on with its own paths.
    if written.is_err() {
        return Some(paths);
    }

    if !claim(&spool) {
        return None;
    }
    let start = Instant::now();
    let mut last = entries(&spool).len();
    let mut settled = Instant::now();
    while settled.elapsed() < quiet && start.elapsed() < MAX_WAIT {
        std::thread::sleep(Duration::from_millis(40));
        let now = entries(&spool).len();
        if now != last {
            last = now;
            settled = Instant::now();
        }
    }

    let mut files = entries(&spool);
    files.sort();
    let gathered = files
        .iter()
        .filter_map(|file| fs::read_to_string(file).ok())
        .flat_map(|list| list.lines().map(PathBuf::from).collect::<Vec<_>>())
        .collect();
    let _ = fs::remove_dir_all(&spool);
    Some(gathered)
}

fn claim(spool: &Path) -> bool {
    let leader = spool.join("leader");
    for _ in 0..2 {
        if fs::File::create_new(&leader).is_ok() {
            return true;
        }
        let stale = fs::metadata(&leader)
            .and_then(|meta| meta.modified())
            .is_ok_and(|time| time.elapsed().unwrap_or_default() > STALE);
        if !stale {
            return false;
        }
        let _ = fs::remove_file(&leader);
    }
    false
}

/// The finished entries of a spool, without the claim and anything still being written.
fn entries(spool: &Path) -> Vec<PathBuf> {
    fs::read_dir(spool)
        .map(|dir| {
            dir.flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_none() && !path.ends_with("leader"))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_process_gets_every_path_and_the_others_none() {
        let root = std::env::temp_dir().join(format!("vdfrs-gather-test-{}", std::process::id()));
        let quiet = Duration::from_millis(300);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let root = root.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(i * 15));
                    let path = PathBuf::from(format!("/folder/item{i}"));
                    gather_in(&root, "pack", vec![path], quiet)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let leaders: Vec<&Vec<PathBuf>> = results.iter().flatten().collect();
        assert_eq!(leaders.len(), 1, "{results:?}");
        let mut all = leaders[0].clone();
        all.sort();
        assert_eq!(
            all,
            (0..8).map(|i| PathBuf::from(format!("/folder/item{i}"))).collect::<Vec<_>>()
        );
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "the spool is cleaned up");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn other_commands_and_folders_gather_apart() {
        let root = std::env::temp_dir().join(format!("vdfrs-gather-apart-{}", std::process::id()));
        let quiet = Duration::from_millis(50);
        let a = gather_in(&root, "pack", vec!["/a/x".into()], quiet);
        let b = gather_in(&root, "extract", vec!["/a/y".into()], quiet);
        let c = gather_in(&root, "pack", vec!["/b/z".into()], quiet);
        assert_eq!((a.unwrap().len(), b.unwrap().len(), c.unwrap().len()), (1, 1, 1));
        let _ = fs::remove_dir_all(&root);
    }
}
