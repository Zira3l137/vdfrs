//! Exporting textures as pictures other programs open: PNG or TGA.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use image::ExtendedColorType;
use image::ImageFormat;
use rayon::prelude::*;
use zenkit::texture::Texture;
use zenkit::vfs::Node;

use crate::archive::host_name;
use crate::kind::FileKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Png,
    Tga,
}

impl Picture {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Tga => "TGA",
        }
    }

    fn format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Tga => ImageFormat::Tga,
        }
    }
}

/// What an export did: how many textures it wrote, and the archive paths of those it could not,
/// with why.
#[derive(Debug, Default)]
pub struct Exported {
    pub written: usize,
    pub failed: Vec<(String, String)>,
}

/// The textures among `nodes`, and in the folders among them, each with where it goes below the
/// destination: a texture on its own beside the others, a folder's in a folder of its name.
pub fn textures(nodes: &[Node], picture: Picture) -> Vec<(&Node, PathBuf)> {
    let mut found = Vec::new();
    for node in nodes {
        if node.is_file() {
            if is_texture(node) {
                found.push((node, PathBuf::from(picture_name(node.name(), picture))));
            }
            continue;
        }
        // As in extracting: a node copied out of the tree keeps its absolute path, and so do
        // its descendants, so the node's own path is stripped off theirs.
        let base = node.path().len();
        for child in node.descendants().filter(|child| child.is_file() && is_texture(child)) {
            let relative = child.path().get(base..).unwrap_or_default().trim_start_matches('/');
            let mut path = PathBuf::from(host_name(node.name()));
            if let Some((dirs, _)) = relative.rsplit_once('/') {
                path.extend(dirs.split('/').map(host_name));
            }
            path.push(picture_name(child.name(), picture));
            found.push((child, path));
        }
    }
    found
}

/// Writes the textures among `nodes` below `dest`, in parallel, at full size. One that fails
/// does not stop the others.
pub fn export(nodes: &[Node], dest: &Path, picture: Picture) -> Exported {
    let textures = textures(nodes, picture);
    let failed: Vec<_> = textures
        .par_iter()
        .filter_map(|(node, relative)| {
            let error = write(node, &dest.join(relative), picture).err()?;
            Some((node.path().to_owned(), error))
        })
        .collect();
    Exported { written: textures.len() - failed.len(), failed }
}

fn write(node: &Node, path: &Path, picture: Picture) -> Result<(), String> {
    let report = |error: zenkit::Error| error.report().to_string();
    let data = node.try_data().map_err(report)?.unwrap_or_default();
    let texture = Texture::from_bytes(data).map_err(report)?;
    let (width, height) = texture.mipmap_dimensions(0);
    let rgba = texture.to_rgba8(0).map_err(report)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    // Opaque textures are written without alpha, as the game's own sources are.
    let pixels = rgba.as_chunks::<4>().0;
    let result = if pixels.iter().all(|pixel| pixel[3] == 255) {
        let rgb: Vec<u8> = pixels.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
        image::save_buffer_with_format(
            path,
            &rgb,
            width,
            height,
            ExtendedColorType::Rgb8,
            picture.format(),
        )
    } else {
        image::save_buffer_with_format(
            path,
            &rgba,
            width,
            height,
            ExtendedColorType::Rgba8,
            picture.format(),
        )
    };
    result.map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn is_texture(node: &Node) -> bool {
    FileKind::of(node.name()) == FileKind::Texture
}

/// `STONE-C.TEX` becomes `STONE.PNG`: the game compiles `STONE.TGA` into `STONE-C.TEX`.
fn picture_name(name: &str, picture: Picture) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = stem.strip_suffix("-C").or_else(|| stem.strip_suffix("-c")).unwrap_or(stem);
    format!("{}.{}", host_name(stem), picture.label())
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    fn erz() -> Vec<u8> {
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/erz.tex")).unwrap()
    }

    #[test]
    fn names_pictures_after_their_sources() {
        assert_eq!(picture_name("STONE-C.TEX", Picture::Png), "STONE.PNG");
        assert_eq!(picture_name("stone-c.tex", Picture::Tga), "stone.TGA");
        assert_eq!(picture_name("PLAIN.TEX", Picture::Tga), "PLAIN.TGA");
        assert_eq!(picture_name("-C.TEX", Picture::Png), "_.PNG");
    }

    #[test]
    fn finds_textures_in_folders_and_keeps_their_layout() {
        let mut root = Node::dir("TEXTURES", UNIX_EPOCH);
        let mut sub = Node::dir("WORLD", UNIX_EPOCH);
        sub.insert(Node::file("ERZ-C.TEX", erz(), UNIX_EPOCH)).unwrap();
        sub.insert(Node::file("README.TXT", b"not a texture".to_vec(), UNIX_EPOCH)).unwrap();
        root.insert(sub).unwrap();
        let lone = Node::file("SKY-C.TEX", erz(), UNIX_EPOCH);
        let nodes = [root, lone];

        let found: Vec<_> =
            textures(&nodes, Picture::Png).into_iter().map(|(_, path)| path).collect();
        let expected =
            [Path::new("TEXTURES").join("WORLD").join("ERZ.PNG"), PathBuf::from("SKY.PNG")];
        assert_eq!(found, expected);
    }

    #[test]
    fn exports_pictures_the_size_of_the_texture() {
        let dest = std::env::temp_dir().join(format!("vdfrs-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        let texture = Texture::from_bytes(&erz()).unwrap();
        let broken = Node::file("BROKEN-C.TEX", b"ZTEX but not really".to_vec(), UNIX_EPOCH);
        let nodes = [Node::file("ERZ-C.TEX", erz(), UNIX_EPOCH), broken];

        for picture in [Picture::Png, Picture::Tga] {
            let exported = export(&nodes, &dest, picture);
            assert_eq!(exported.written, 1);
            assert_eq!(exported.failed.len(), 1, "{:?}", exported.failed);
            let path = dest.join(format!("ERZ.{}", picture.label()));
            let read = image::open(&path).unwrap();
            assert_eq!((read.width(), read.height()), (texture.width, texture.height));
        }
        fs::remove_dir_all(&dest).unwrap();
    }
}
