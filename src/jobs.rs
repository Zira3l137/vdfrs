//! What the context menu commands do, without a window: extracting archives and packing files
//! into one. The progress window runs these on a background thread.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use zenkit::Result;
use zenkit::ResultExt;
use zenkit::vfs::Node;
use zenkit::vfs::Overwrite;
use zenkit::vfs::Vfs;

use crate::archive;
use crate::archive::Archive;

/// Where an archive's files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Into the folder the archive is in.
    Here,
    /// Into a folder beside the archive, named like it: `Anims.vdf` into `Anims\`.
    OwnFolder,
    Folder(PathBuf),
}

/// Files done out of files to do, shared with the window that shows it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

impl Progress {
    /// From 0 to 1, or `None` while the total is not known yet.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// Extracts every file of `archive`. Returns the folder they went into.
pub fn extract(archive: &Path, dest: &Destination, progress: &Progress) -> Result<PathBuf> {
    let mut vfs = Vfs::new();
    vfs.mount_disk(archive, Overwrite::Older)?;
    let parent = archive.parent().unwrap_or(Path::new("."));
    let target = match dest {
        Destination::Here => parent.to_owned(),
        Destination::OwnFolder => {
            parent.join(archive.file_stem().unwrap_or_else(|| "extracted".as_ref()))
        }
        Destination::Folder(folder) => folder.clone(),
    };
    progress.total.fetch_add(vfs.files().count(), Ordering::Relaxed);
    let nodes: Vec<Node> = vfs.root().children().cloned().collect();
    archive::extract(&nodes, &target, &progress.done)
        .context(|| format!("cannot extract {}", archive.display()))?;
    Ok(target)
}

/// Packs files and folders into a new archive at `out`, each at the archive's top level as it is
/// on disk. Gothic II's signature, no compression: what every game and tool reads.
pub fn pack(paths: &[PathBuf], out: &Path, progress: &Progress) -> Result<PathBuf> {
    let nodes = archive::import(paths)?;
    let mut archive = Archive::new();
    archive.insert(nodes, "/")?;
    let files = archive.stats().files;
    progress.total.store(files, Ordering::Relaxed);
    let saved = archive.save_job(out.to_owned()).run()?;
    progress.done.store(files, Ordering::Relaxed);
    Ok(saved)
}

/// The archive a pack of `paths` makes by default, beside them: named after the one item packed,
/// or after the folder they are in for several. An existing archive is never overwritten; the
/// name gets a number instead, as in `Textures (2).vdf`.
pub fn pack_output(paths: &[PathBuf]) -> PathBuf {
    let first = paths.first().map(PathBuf::as_path).unwrap_or(Path::new("archive"));
    let parent = first.parent().unwrap_or(Path::new("."));
    let stem = match paths {
        [one] if one.is_dir() => one.file_name(),
        [one] => one.file_stem(),
        _ => parent.file_name(),
    };
    let stem = stem.map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
    (1..)
        .map(|n| {
            let name = if n == 1 { format!("{stem}.vdf") } else { format!("{stem} ({n}).vdf") };
            parent.join(name)
        })
        .find(|path| !path.exists())
        .expect("some numbered name is free")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vdfrs-jobs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_into_each_destination() {
        let dir = temp("extract");
        let archive = dir.join("Basic.vdf");
        fs::copy("samples/basic.vdf", &archive).unwrap();

        let progress = Progress::default();
        assert_eq!(
            extract(&archive, &Destination::OwnFolder, &progress).unwrap(),
            dir.join("Basic")
        );
        assert!(dir.join("Basic/LICENSES/GPL/LGPL-3.0.MD").is_file());
        assert_eq!(progress.fraction(), Some(1.0));

        extract(&archive, &Destination::Here, &Progress::default()).unwrap();
        assert!(dir.join("README.MD").is_file());

        let elsewhere = dir.join("out");
        extract(&archive, &Destination::Folder(elsewhere.clone()), &Progress::default()).unwrap();
        assert!(elsewhere.join("CONFIG.YML").is_file());

        assert!(
            extract(&dir.join("missing.vdf"), &Destination::Here, &Progress::default()).is_err()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn packs_beside_the_items_without_overwriting() {
        let dir = temp("pack");
        extract(
            Path::new("samples/basic.vdf"),
            &Destination::Folder(dir.join("_WORK")),
            &Progress::default(),
        )
        .unwrap();
        let items = vec![dir.join("_WORK")];

        let out = pack_output(&items);
        assert_eq!(out, dir.join("_WORK.vdf"));
        let progress = Progress::default();
        pack(&items, &out, &progress).unwrap();
        assert_eq!(progress.fraction(), Some(1.0));
        assert_eq!(pack_output(&items), dir.join("_WORK (2).vdf"));

        let packed = Archive::open(&out).unwrap();
        assert!(packed.exists("/_WORK/LICENSES/MIT.MD"));
        assert_eq!(packed.stats().files, 5);

        let files = vec![dir.join("_WORK/README.MD"), dir.join("_WORK/CONFIG.YML")];
        assert_eq!(pack_output(&files), dir.join("_WORK/_WORK.vdf"));
        assert_eq!(pack_output(&files[..1]), dir.join("_WORK/README.vdf"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
