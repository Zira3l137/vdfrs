//! An open archive: the tree mounted from a `.vdf` or `.mod`, the header to write it back with, and
//! every edit the explorer makes to it. Nothing here knows about the UI.

use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::io::BufWriter;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use rayon::prelude::*;
use zenkit::Error;
use zenkit::GameVersion;
use zenkit::Result;
use zenkit::ResultExt;
use zenkit::vfs::DiskInfo;
use zenkit::vfs::DiskOptions;
use zenkit::vfs::Node;
use zenkit::vfs::Overwrite;
use zenkit::vfs::Vfs;

use crate::kind::FileKind;

/// The longest name a VDF catalog entry holds, in Latin-1 bytes.
pub const MAX_NAME: usize = 64;

/// Snapshots kept for undo are whole trees. Their files share the archive's bytes, so a snapshot
/// costs only its nodes; this caps how many nodes all snapshots together may hold.
const UNDO_BUDGET: usize = 2_000_000;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: Arc<str>,
    pub path: Arc<str>,
    pub is_dir: bool,
    /// Bytes for a file, the number of children for a directory.
    pub size: u64,
    pub time: SystemTime,
    pub kind: FileKind,
}

impl Entry {
    fn of(node: &Node) -> Self {
        Self {
            name: node.name().into(),
            path: node.path().into(),
            is_dir: node.is_dir(),
            size: match node.size() {
                Some(size) => size as u64,
                None => node.children().len() as u64,
            },
            time: node.time(),
            kind: if node.is_dir() { FileKind::Folder } else { FileKind::of(node.name()) },
        }
    }

    pub fn parent(&self) -> &str {
        parent_of(&self.path)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    /// Directories holding no file anywhere below them, which a VDF cannot store.
    pub empty_dirs: usize,
}

/// What changed in an edit, so that the explorer can select it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub paths: Vec<String>,
    pub replaced: usize,
    pub skipped: usize,
}

#[derive(Clone)]
struct Snapshot {
    vfs: Vfs,
    nodes: usize,
    dirty: bool,
}

pub struct Archive {
    vfs: Vfs,
    path: Option<PathBuf>,
    options: DiskOptions,
    dirty: bool,
    /// Bumped on every change, so that views know when what they show is out of date.
    generation: u64,
    load_time: Duration,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    nodes: usize,
    /// Counting walks the whole tree, and the window asks on every frame it draws the totals.
    stats: Cell<Option<(u64, Stats)>>,
}

impl Archive {
    pub fn new() -> Self {
        Self {
            vfs: Vfs::new(),
            path: None,
            options: DiskOptions::new(GameVersion::Gothic2),
            dirty: false,
            generation: 0,
            load_time: Duration::ZERO,
            undo: Vec::new(),
            redo: Vec::new(),
            nodes: 0,
            stats: Cell::new(None),
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let start = Instant::now();
        let path = &std::path::absolute(path).map_err(Error::io(path))?;
        let info = DiskInfo::read(path)?;
        let mut vfs = Vfs::new();
        vfs.mount_disk(path, Overwrite::Older)?;
        let nodes = vfs.nodes().count();
        Ok(Self {
            vfs,
            path: Some(path.to_owned()),
            options: info.options(),
            load_time: start.elapsed(),
            nodes,
            ..Self::new()
        })
    }

    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(path) => path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            None => "Untitled.vdf".into(),
        }
    }

    pub fn options(&self) -> &DiskOptions {
        &self.options
    }

    pub fn set_options(&mut self, options: DiskOptions) {
        if options != self.options {
            self.options = options;
            self.touch();
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn load_time(&self) -> Duration {
        self.load_time
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.vfs.resolve(path).is_some()
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.vfs.resolve(path).is_some_and(Node::is_dir)
    }

    pub fn list(&self, dir: &str) -> Vec<Entry> {
        self.vfs.resolve(dir).map_or_else(Vec::new, |dir| dir.children().map(Entry::of).collect())
    }

    /// Every node below `dir` whose name matches `query`, in tree order, up to `limit` of them. A
    /// query with `*` or `?` is a wildcard pattern for the whole name; anything else matches
    /// anywhere in it. Case never matters.
    pub fn search(&self, dir: &str, query: &str, limit: usize) -> Vec<Entry> {
        let query = query.trim().to_ascii_lowercase();
        let Some(dir) = self.vfs.resolve(dir).filter(|_| !query.is_empty()) else {
            return Vec::new();
        };
        let wildcard = query.contains(['*', '?']);
        let mut name = String::new();
        dir.descendants()
            .filter(|node| {
                name.clear();
                name.extend(node.name().chars().map(|c| c.to_ascii_lowercase()));
                if wildcard {
                    glob(query.as_bytes(), name.as_bytes())
                } else {
                    name.contains(&query)
                }
            })
            .take(limit)
            .map(Entry::of)
            .collect()
    }

    pub fn stats(&self) -> Stats {
        if let Some((generation, stats)) = self.stats.get()
            && generation == self.generation
        {
            return stats;
        }
        let stats = self.count();
        self.stats.set(Some((self.generation, stats)));
        stats
    }

    fn count(&self) -> Stats {
        fn walk(node: &Node, stats: &mut Stats) -> bool {
            let mut has_files = false;
            for child in node.children() {
                if let Some(size) = child.size() {
                    stats.files += 1;
                    stats.bytes += size as u64;
                    has_files = true;
                } else {
                    stats.dirs += 1;
                    has_files |= walk(child, stats);
                }
            }
            if !has_files && node.path() != "/" {
                stats.empty_dirs += 1;
            }
            has_files
        }
        let mut stats = Stats::default();
        walk(self.vfs.root(), &mut stats);
        stats
    }

    /// The directories of `dir`, for the folder tree.
    pub fn subdirs(&self, dir: &str) -> Vec<Entry> {
        self.vfs.resolve(dir).map_or_else(Vec::new, |dir| {
            dir.children().filter(|child| child.is_dir()).map(Entry::of).collect()
        })
    }

    pub fn has_subdirs(&self, dir: &str) -> bool {
        self.vfs.resolve(dir).is_some_and(|dir| dir.children().any(Node::is_dir))
    }

    /// Clones the nodes at `paths`, to copy them into another archive or onto the clipboard.
    /// Cheap: the copies share the archive's bytes.
    pub fn take_copies(&self, paths: &[String]) -> Vec<Node> {
        paths.iter().filter_map(|path| self.vfs.resolve(path)).cloned().collect()
    }

    /// Puts `nodes` into `dest`, replacing files of the same name and merging directories.
    pub fn insert(&mut self, nodes: Vec<Node>, dest: &str) -> Result<Outcome> {
        if nodes.is_empty() {
            return Ok(Outcome::default());
        }
        self.check_dir(dest)?;
        for node in &nodes {
            check_name(node.name())?;
        }
        self.checkpoint();
        let mut outcome = Outcome::default();
        for node in nodes {
            outcome.replaced += usize::from(self.child_exists(dest, node.name()));
            outcome.paths.push(join(dest, node.name()));
            self.vfs.mount(node, dest, Overwrite::All)?;
        }
        self.changed();
        Ok(outcome)
    }

    /// Moves the nodes at `paths` into `dest`. Nodes already there, and directories that would
    /// end up inside themselves, are skipped.
    pub fn move_into(&mut self, paths: &[String], dest: &str) -> Result<Outcome> {
        self.check_dir(dest)?;
        let dest_key = dest.to_ascii_lowercase();
        let movable: Vec<&String> = paths
            .iter()
            .filter(|path| {
                let key = path.to_ascii_lowercase();
                let inside = dest_key == key || dest_key.starts_with(&format!("{key}/"));
                !inside && !parent_of(path).eq_ignore_ascii_case(dest) && self.exists(path)
            })
            .collect();
        let mut outcome = Outcome { skipped: paths.len() - movable.len(), ..Outcome::default() };
        if movable.is_empty() {
            return Ok(outcome);
        }

        self.checkpoint();
        for path in movable {
            let Some(node) = self.vfs.remove(path) else { continue };
            outcome.replaced += usize::from(self.child_exists(dest, node.name()));
            outcome.paths.push(join(dest, node.name()));
            self.vfs.mount(node, dest, Overwrite::All)?;
        }
        self.changed();
        Ok(outcome)
    }

    pub fn rename(&mut self, path: &str, name: &str) -> Result<String> {
        let name = name.trim();
        check_name(name)?;
        let current = self.vfs.resolve(path).ok_or_else(|| Error::NotFound(path.into()))?;
        if current.name() == name {
            return Ok(path.into());
        }
        let parent = parent_of(path).to_owned();
        if !current.name().eq_ignore_ascii_case(name) && self.child_exists(&parent, name) {
            return Err(Error::AlreadyExists(name.into()));
        }
        self.checkpoint();
        let path = self.vfs.rename(path, name)?.path().to_owned();
        self.changed();
        Ok(path)
    }

    pub fn delete(&mut self, paths: &[String]) -> usize {
        let paths: Vec<&String> = paths.iter().filter(|path| self.exists(path)).collect();
        if paths.is_empty() {
            return 0;
        }
        self.checkpoint();
        let removed = paths.into_iter().filter(|path| self.vfs.remove(path).is_some()).count();
        self.changed();
        removed
    }

    /// Makes a new directory in `parent` under the first free name like `NEW FOLDER (2)`.
    pub fn new_folder(&mut self, parent: &str) -> Result<String> {
        self.check_dir(parent)?;
        let name = (1..)
            .map(|n| if n == 1 { "NEW FOLDER".to_owned() } else { format!("NEW FOLDER ({n})") })
            .find(|name| !self.child_exists(parent, name))
            .unwrap_or_default();
        self.checkpoint();
        let path = self.vfs.mkdir(&join(parent, &name))?.path().to_owned();
        self.changed();
        Ok(path)
    }

    pub fn undo(&mut self) -> bool {
        self.step(true)
    }

    pub fn redo(&mut self) -> bool {
        self.step(false)
    }

    fn step(&mut self, back: bool) -> bool {
        let (from, to) =
            if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(snapshot) = from.pop() else { return false };
        let current = Snapshot {
            vfs: std::mem::replace(&mut self.vfs, snapshot.vfs),
            nodes: self.nodes,
            dirty: self.dirty,
        };
        to.push(current);
        self.nodes = snapshot.nodes;
        self.dirty = snapshot.dirty;
        self.generation += 1;
        true
    }

    /// Writes the archive to `path`, through a file beside it that then takes its place, so that
    /// a failed save leaves the old archive whole. The tree being written is a copy, so the
    /// explorer can go on while this runs on another thread.
    pub fn save_job(&self, path: PathBuf) -> SaveJob {
        SaveJob { vfs: self.vfs.clone(), options: self.options.clone(), path }
    }

    pub fn saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.dirty = false;
        for snapshot in self.undo.iter_mut().chain(&mut self.redo) {
            snapshot.dirty = true;
        }
        self.generation += 1;
    }

    fn check_dir(&self, path: &str) -> Result<()> {
        match self.vfs.resolve(path) {
            Some(node) if node.is_dir() => Ok(()),
            Some(_) => Err(Error::NotADirectory(path.into())),
            None => Err(Error::NotFound(path.into())),
        }
    }

    fn child_exists(&self, dir: &str, name: &str) -> bool {
        self.vfs.resolve(dir).and_then(|dir| dir.child(name)).is_some()
    }

    fn checkpoint(&mut self) {
        self.redo.clear();
        self.undo.push(Snapshot { vfs: self.vfs.clone(), nodes: self.nodes, dirty: self.dirty });
        let mut total: usize = self.undo.iter().map(|s| s.nodes).sum();
        while total > UNDO_BUDGET && self.undo.len() > 1 {
            total -= self.undo.remove(0).nodes;
        }
    }

    fn changed(&mut self) {
        self.nodes = self.vfs.nodes().count();
        self.touch();
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.generation += 1;
    }
}

pub struct SaveJob {
    vfs: Vfs,
    options: DiskOptions,
    path: PathBuf,
}

impl SaveJob {
    pub fn run(self) -> Result<PathBuf> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let name = self.path.file_name().unwrap_or_default().to_string_lossy();
        let temp = dir.join(format!(".{name}.saving"));
        let file = fs::File::create(&temp).map_err(Error::io_write(&temp))?;
        let written = self.vfs.write_disk(BufWriter::new(file), &self.options);
        match written
            .and_then(|()| fs::rename(&temp, &self.path).map_err(Error::io_write(&self.path)))
        {
            Ok(()) => Ok(self.path),
            Err(error) => {
                let _ = fs::remove_file(&temp);
                Err(error).context(|| format!("cannot save {}", self.path.display()))
            }
        }
    }
}

/// Reads files and folders from the host into nodes, ready for [`Archive::insert`]. Folders keep
/// their name and everything in them; files are read in parallel.
pub fn import(paths: &[PathBuf]) -> Result<Vec<Node>> {
    paths
        .par_iter()
        .map(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let meta = fs::metadata(path).map_err(Error::io(path))?;
            let time = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if meta.is_dir() {
                let mut vfs = Vfs::new();
                vfs.mkdir(&name)?;
                vfs.mount_host(path, &name, Overwrite::All)?;
                Ok(vfs.root_mut().remove(&name).expect("the folder was just made"))
            } else {
                let data = fs::read(path).map_err(Error::io(path))?;
                Ok(Node::file(name, data, time))
            }
        })
        .collect()
}

/// Writes copies of archive nodes to `dest` on the host: files as files, directories as folders
/// with everything in them. Files are written in parallel, each counted in `written` once it is
/// on disk. Returns the paths of what was written at the top, one per node.
pub fn extract(nodes: &[Node], dest: &Path, written: &AtomicUsize) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut top = Vec::with_capacity(nodes.len());
    for node in nodes {
        let target = dest.join(host_name(node.name()));
        top.push(target.clone());
        if node.is_file() {
            files.push((node, target));
            continue;
        }
        dirs.push(target.clone());
        // Descendant paths are relative to the node once it is out of the tree, but a node
        // cloned from a tree keeps its absolute ones, so strip the node's own path off.
        let base = node.path().len();
        for child in node.descendants() {
            let relative = child.path().get(base..).unwrap_or_default().trim_start_matches('/');
            let mut path = target.clone();
            path.extend(relative.split('/').map(host_name));
            if child.is_dir() { dirs.push(path) } else { files.push((child, path)) }
        }
    }

    for dir in &dirs {
        fs::create_dir_all(dir).map_err(Error::io_write(dir))?;
    }
    files.par_iter().try_for_each(|(node, path)| {
        let data = node.try_data()?.unwrap_or_default();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(Error::io_write(parent))?;
        }
        fs::write(path, data).map_err(Error::io_write(path))?;
        let time = fs::FileTimes::new().set_modified(node.time());
        let _ = fs::File::options().write(true).open(path).and_then(|file| file.set_times(time));
        written.fetch_add(1, Ordering::Relaxed);
        Ok::<_, Error>(())
    })?;
    Ok(top)
}

/// Whether a name can be stored in a VDF, and if not, why.
pub fn check_name(name: &str) -> Result<()> {
    let reason = if name.is_empty() {
        Some("a name cannot be empty".to_owned())
    } else if name.contains(['/', '\\']) {
        Some(format!("{name:?} has a slash, which separates folders"))
    } else if name.chars().any(|c| c as u32 > 0xFF || c.is_control()) {
        Some(format!("{name:?} has characters beyond Latin-1, which a VDF cannot store"))
    } else if name.chars().count() > MAX_NAME {
        Some(format!("{name:?} is longer than the {MAX_NAME} characters a VDF stores"))
    } else {
        None
    };
    reason.map_or(Ok(()), |reason| Err(Error::unwritable("VDF archive", reason)))
}

pub fn parent_of(path: &str) -> &str {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir == "/" || dir.is_empty() { format!("/{name}") } else { format!("{dir}/{name}") }
}

/// Archive names may hold characters the host refuses in file names; they become `_`.
pub(crate) fn host_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') { '_' } else { c })
        .collect();
    match name.trim_end_matches(['.', ' ']) {
        "" => "_".into(),
        trimmed => trimmed.into(),
    }
}

/// Matches a whole name against a pattern of `*` (any run) and `?` (any one character).
fn glob(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star = None;
    while n < name.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == b'?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

/// The paths of `paths` that are not inside another one of them.
pub fn outermost(paths: &[String]) -> Vec<String> {
    let keys: HashSet<String> = paths.iter().map(|p| p.to_ascii_lowercase()).collect();
    paths
        .iter()
        .filter(|path| {
            let key = path.to_ascii_lowercase();
            let mut parent = parent_of(&key);
            while parent != "/" {
                if keys.contains(parent) {
                    return false;
                }
                parent = parent_of(parent);
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    fn sample() -> Archive {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/basic.vdf");
        Archive::open(&path).unwrap()
    }

    fn names(archive: &Archive, dir: &str) -> Vec<String> {
        archive.list(dir).iter().map(|e| e.name.to_string()).collect()
    }

    #[test]
    fn opens_with_header_and_stats() {
        let archive = sample();
        assert_eq!(archive.options().game, GameVersion::Gothic2);
        assert!(archive.options().comment.starts_with("Sample VDF"));
        let stats = archive.stats();
        assert_eq!((stats.files, stats.dirs, stats.empty_dirs), (5, 2, 0));
        assert!(!archive.is_dirty());
    }

    #[test]
    fn searches_by_substring_and_wildcard() {
        let archive = sample();
        let found = |q: &str| -> Vec<String> {
            archive.search("/", q, 100).iter().map(|e| e.name.to_ascii_uppercase()).collect()
        };
        assert_eq!(found("gpl"), ["GPL", "GPL-3.0.MD", "LGPL-3.0.MD"]);
        assert_eq!(found("*.md").len(), 4);
        assert_eq!(found("?it.md"), ["MIT.MD"]);
        assert!(found("   ").is_empty());
        assert_eq!(archive.search("/", "md", 2).len(), 2);
        assert_eq!(archive.search("/LICENSES/GPL", "md", 10).len(), 2);
    }

    #[test]
    fn moves_renames_deletes_and_undoes() {
        let mut archive = sample();
        let before = names(&archive, "/");

        let moved = archive.move_into(&["/README.MD".into()], "/LICENSES/GPL").unwrap();
        assert_eq!(moved.paths, ["/LICENSES/GPL/README.MD"]);
        assert!(archive.is_dirty());

        let skipped = archive.move_into(&["/LICENSES".into()], "/LICENSES/GPL").unwrap();
        assert_eq!((skipped.paths.len(), skipped.skipped), (0, 1));

        let renamed = archive.rename("/LICENSES/GPL", "Open").unwrap();
        assert_eq!(renamed, "/LICENSES/Open");
        assert!(archive.exists("/licenses/open/readme.md"));
        assert!(archive.rename("/LICENSES/Open", "mit.md").is_err(), "taken by a sibling");
        assert!(archive.rename("/LICENSES/Open", "a/b").is_err());

        assert_eq!(archive.delete(&["/CONFIG.YML".into(), "/nope".into()]), 1);

        while archive.undo() {}
        assert_eq!(names(&archive, "/"), before);
        assert!(!archive.is_dirty());
        assert!(archive.redo());
        assert!(archive.exists("/LICENSES/GPL/README.MD"));
    }

    #[test]
    fn copies_between_archives_without_touching_the_source() {
        let source = sample();
        let mut target = Archive::new();
        let copies = source.take_copies(&["/LICENSES".into()]);
        let outcome = target.insert(copies, "/").unwrap();
        assert_eq!(outcome.paths, ["/LICENSES"]);
        assert_eq!(
            target.vfs().resolve("/licenses/gpl/lgpl-3.0.md").unwrap().path(),
            "/LICENSES/GPL/LGPL-3.0.MD"
        );
        assert_eq!(target.stats().files, 3);
        assert!(source.exists("/LICENSES"));

        let again = target.insert(source.take_copies(&["/LICENSES/MIT.MD".into()]), "/LICENSES");
        assert_eq!(again.unwrap().replaced, 1);
    }

    #[test]
    fn new_folders_take_free_names_and_count_as_empty() {
        let mut archive = Archive::new();
        assert_eq!(archive.new_folder("/").unwrap(), "/NEW FOLDER");
        assert_eq!(archive.new_folder("/").unwrap(), "/NEW FOLDER (2)");
        assert_eq!(archive.stats().empty_dirs, 2);
        assert!(archive.new_folder("/missing").is_err());
    }

    #[test]
    fn saves_and_reopens() {
        let dir = std::env::temp_dir().join(format!("vdfrs-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut archive = sample();
        archive.rename("/README.MD", "Readme.txt").unwrap();
        let path = archive.save_job(dir.join("out.vdf")).run().unwrap();
        archive.saved(path.clone());
        assert!(!archive.is_dirty());

        let reopened = Archive::open(&path).unwrap();
        assert!(reopened.exists("/README.TXT"));
        assert_eq!(reopened.options(), archive.options());
        assert!(!dir.join(".out.vdf.saving").exists());

        let written = AtomicUsize::new(0);
        let extracted = extract(&reopened.take_copies(&["/LICENSES".into()]), &dir, &written);
        assert_eq!(extracted.unwrap(), [dir.join("LICENSES")]);
        assert_eq!(written.into_inner(), 3);
        let mit = fs::read_to_string(dir.join("LICENSES/MIT.MD")).unwrap();
        assert!(mit.starts_with("# The MIT license"));
        assert!(dir.join("LICENSES/GPL/LGPL-3.0.MD").is_file());

        let imported = import(&[dir.join("LICENSES")]).unwrap();
        let mut fresh = Archive::new();
        fresh.insert(imported, "/").unwrap();
        assert_eq!(fresh.stats().files, 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_the_format_cannot_hold_are_refused() {
        assert!(check_name("TEXTURE-C.TEX").is_ok());
        assert!(check_name("\u{c4}PFEL").is_ok());
        for bad in ["", "a/b", "a\\b", "\u{3b1}", &"N".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        let mut archive = Archive::new();
        let node = Node::file("\u{1F600}", vec![], UNIX_EPOCH);
        assert!(archive.insert(vec![node], "/").is_err());
        assert!(!archive.is_dirty());
    }

    #[test]
    fn paths() {
        assert_eq!(parent_of("/A/B"), "/A");
        assert_eq!(parent_of("/A"), "/");
        assert_eq!(parent_of("/"), "/");
        assert_eq!(join("/", "A"), "/A");
        assert_eq!(join("/A", "B"), "/A/B");
        assert_eq!(host_name("a:b?."), "a_b_");
        let paths = ["/A".into(), "/a/B".into(), "/C/D".into(), "/C".into()];
        assert_eq!(outermost(&paths), ["/A", "/C"]);
        assert!(glob(b"*-c.tex", b"stone-c.tex"));
        assert!(!glob(b"*-c.tex", b"stone.tex"));
        assert!(glob(b"a*b*c", b"axxbyyc"));
    }
}
