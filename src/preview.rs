//! The pane beside the list that shows the selected file: textures as pictures, text as text, and
//! anything else as a hex dump. Files are read and decoded on a background thread.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AnyElement;
use gpui_kit::App;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::ObjectFit;
use gpui_kit::ParentElement as _;
use gpui_kit::RenderImage;
use gpui_kit::SharedString;
use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::Styled as _;
use gpui_kit::StyledImage as _;
use gpui_kit::Task;
use gpui_kit::UniformListScrollHandle;
use gpui_kit::Window;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::h_flex;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use gpui_kit::img;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use gpui_kit::uniform_list;
use smallvec::smallvec;
use zenkit::texture::Texture;
use zenkit::vfs::Node;

use crate::archive::Entry;
use crate::archive_view::ArchiveView;
use crate::format;
use crate::kind::FileKind;

/// Decoded textures are scaled down to the largest mipmap no wider or taller than this.
const MAX_IMAGE_SIDE: u32 = 1024;
const MAX_TEXT_BYTES: usize = 1 << 20;
const MAX_HEX_BYTES: usize = 64 << 10;

#[derive(Default)]
pub enum Content {
    #[default]
    Empty,
    Loading,
    Image {
        image: Arc<RenderImage>,
        details: SharedString,
    },
    Lines {
        lines: Rc<Vec<SharedString>>,
        mono: bool,
        note: Option<SharedString>,
    },
    Failed(SharedString),
}

#[derive(Default)]
pub struct Preview {
    /// The file shown, with the archive generation it was read at.
    key: Option<(Arc<str>, u64)>,
    pub content: Content,
    pub scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
}

/// What the background thread hands back. RenderImage is made on the window's thread, which owns
/// the image atlas.
enum Decoded {
    Image { width: u32, height: u32, bgra: Vec<u8>, details: String },
    Lines { lines: Vec<String>, mono: bool, note: Option<String> },
    Failed(String),
}

impl ArchiveView {
    /// Shows `entry` in the preview, reading it again only if it or the archive changed.
    pub(crate) fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self.single_selected().filter(|entry| !entry.is_dir);
        let key = entry.as_ref().map(|entry| (entry.path.clone(), self.archive.generation()));
        if key == self.preview.key {
            return;
        }
        self.preview.key = key;
        self.preview.scroll = UniformListScrollHandle::new();
        self.set_preview(Content::Empty, window, cx);

        let Some(entry) = entry else { return };
        let Some(node) = self.archive.vfs().resolve(&entry.path).cloned() else { return };
        if !self.show_preview {
            return;
        }
        let key = self.preview.key.clone();
        self.set_preview(Content::Loading, window, cx);
        let decoding = cx.background_spawn(async move { decode(&node, entry.kind) });
        self.preview._task = Some(cx.spawn_in(window, async move |this, cx| {
            let decoded = decoding.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.preview.key != key {
                    return;
                }
                let content = match decoded {
                    Decoded::Image { width, height, bgra, details } => {
                        match image::RgbaImage::from_raw(width, height, bgra) {
                            Some(buffer) => Content::Image {
                                image: Arc::new(RenderImage::new(smallvec![image::Frame::new(
                                    buffer
                                )])),
                                details: details.into(),
                            },
                            None => Content::Failed("the texture's size does not add up".into()),
                        }
                    }
                    Decoded::Lines { lines, mono, note } => Content::Lines {
                        lines: Rc::new(lines.into_iter().map(SharedString::from).collect()),
                        mono,
                        note: note.map(SharedString::from),
                    },
                    Decoded::Failed(reason) => Content::Failed(reason.into()),
                };
                this.set_preview(content, window, cx);
            });
        }));
    }

    /// Replaces what the preview shows, giving the atlas back the space of a texture shown before.
    fn set_preview(&mut self, content: Content, window: &mut Window, cx: &mut Context<Self>) {
        if let Content::Image { image, .. } = std::mem::replace(&mut self.preview.content, content)
        {
            cx.drop_image(image, Some(window));
        }
        cx.notify();
    }

    pub(crate) fn clear_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview.key = None;
        self.preview._task = None;
        self.set_preview(Content::Empty, window, cx);
    }

    pub(crate) fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let header = |icon: Icon, title: SharedString, subtitle: SharedString| {
            v_flex()
                .gap_1()
                .p_3()
                .border_b_1()
                .border_color(theme.border)
                .child(h_flex().gap_2().child(icon).child(
                    div().text_sm().font_semibold().text_ellipsis().overflow_hidden().child(title),
                ))
                .child(div().text_xs().text_color(theme.muted_foreground).child(subtitle))
        };

        let (head, body): (AnyElement, AnyElement) = match self.single_selected() {
            None if self.selection.is_empty() => {
                let stats = self.archive.stats();
                let subtitle = format!(
                    "{} · {} · {}",
                    format::count(stats.files, "file", "files"),
                    format::count(stats.dirs, "folder", "folders"),
                    format::size(stats.bytes)
                );
                let icon = Icon::new(gpui_kit::assets::IconName::Package).text_color(theme.primary);
                (
                    header(icon, self.archive.title().into(), subtitle.into()).into_any_element(),
                    self.render_archive_details(cx),
                )
            }
            Some(entry) => {
                let icon = Icon::new(entry.kind.icon()).text_color(entry.kind.color(cx));
                let subtitle = if entry.is_dir {
                    format!("Folder · {}", format::items(entry.size))
                } else {
                    format!("{} · {}", entry.kind.label(), format::size(entry.size))
                };
                let body = if entry.is_dir {
                    self.render_folder_details(&entry, cx)
                } else {
                    self.render_content(cx)
                };
                (header(icon, entry.name.clone().into(), subtitle.into()).into_any_element(), body)
            }
            None => {
                let (files, dirs, bytes) = self.selection_totals;
                let subtitle = format!(
                    "{} · {} · {}",
                    format::count(files, "file", "files"),
                    format::count(dirs, "folder", "folders"),
                    format::size(bytes)
                );
                let icon = Icon::new(gpui_kit::assets::IconName::Layers).text_color(theme.primary);
                let title = format!("{} selected", format::items(self.selection.len() as u64));
                (
                    header(icon, title.into(), subtitle.into()).into_any_element(),
                    div().into_any_element(),
                )
            }
        };

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(head)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn render_content(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        match &self.preview.content {
            Content::Empty if !self.show_preview => div().into_any_element(),
            Content::Empty | Content::Loading => div()
                .p_3()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Reading…")
                .into_any_element(),
            Content::Failed(reason) => div()
                .p_3()
                .text_xs()
                .text_color(theme.danger)
                .child(reason.clone())
                .into_any_element(),
            Content::Image { image, details } => v_flex()
                .size_full()
                .p_3()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .p_2()
                        .child(img(image.clone()).size_full().object_fit(ObjectFit::Contain)),
                )
                .child(div().text_xs().text_color(theme.muted_foreground).child(details.clone()))
                .into_any_element(),
            Content::Lines { lines, mono, note } => {
                let lines = lines.clone();
                let mono = *mono;
                let font = theme.mono_font_family.clone();
                let color = theme.foreground;
                v_flex()
                    .size_full()
                    .when_some(note.clone(), |this, note| {
                        this.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .border_b_1()
                                .border_color(theme.border)
                                .child(note),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(
                                uniform_list("preview-lines", lines.len(), move |range, _, _| {
                                    range
                                        .map(|ix| {
                                            div()
                                                .id(ix)
                                                .px_3()
                                                .h(px(18.))
                                                .text_xs()
                                                .whitespace_nowrap()
                                                .text_color(color)
                                                .when(mono, |this| this.font_family(font.clone()))
                                                .child(lines[ix].clone())
                                        })
                                        .collect()
                                })
                                .size_full()
                                .track_scroll(&self.preview.scroll),
                            )
                            .vertical_scrollbar(&self.preview.scroll),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_folder_details(&self, entry: &Entry, cx: &App) -> AnyElement {
        let Some(node) = self.archive.vfs().resolve(&entry.path) else {
            return div().into_any_element();
        };
        let (mut files, mut dirs, mut bytes) = (0, 0, 0);
        for node in node.descendants() {
            match node.size() {
                Some(size) => {
                    files += 1;
                    bytes += size as u64;
                }
                None => dirs += 1,
            }
        }
        details(
            [
                ("Files", files.to_string()),
                ("Folders", dirs.to_string()),
                ("Size", format::size(bytes)),
                ("Path", entry.path.to_string()),
            ],
            cx,
        )
    }

    fn render_archive_details(&self, cx: &App) -> AnyElement {
        let options = self.archive.options();
        let stats = self.archive.stats();
        let mut rows = vec![
            ("Game", game_name(options.game).to_string()),
            ("Compression", if options.compressed { "Union (zlib)" } else { "None" }.to_string()),
            ("Comment", options.comment.clone()),
        ];
        if let Some(path) = self.archive.path() {
            rows.push(("Location", path.display().to_string()));
            rows.push(("Opened in", format!("{} ms", self.archive.load_time().as_millis())));
        }
        if stats.empty_dirs > 0 {
            rows.push((
                "Not saved",
                format!(
                    "{}: a VDF cannot hold folders without files",
                    format::count(stats.empty_dirs, "empty folder", "empty folders")
                ),
            ));
        }
        details(rows, cx)
    }
}

pub fn game_name(game: zenkit::GameVersion) -> &'static str {
    match game {
        zenkit::GameVersion::Gothic1 => "Gothic",
        zenkit::GameVersion::Gothic2 => "Gothic II",
    }
}

fn details(rows: impl IntoIterator<Item = (&'static str, String)>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .id("details")
        .p_3()
        .gap_2()
        .overflow_y_scroll()
        .children(rows.into_iter().map(|(label, value)| {
            v_flex()
                .gap_0p5()
                .child(div().text_xs().text_color(theme.muted_foreground).child(label))
                .child(div().text_sm().child(value))
        }))
        .into_any_element()
}

fn decode(node: &Node, kind: FileKind) -> Decoded {
    let data = match node.try_data() {
        Ok(data) => data.unwrap_or_default(),
        Err(error) => return Decoded::Failed(error.report().to_string()),
    };
    if kind == FileKind::Texture {
        return match decode_texture(data) {
            Ok(decoded) => decoded,
            Err(error) => {
                Decoded::Failed(format!("Not a texture the library reads: {}", error.report()))
            }
        };
    }
    if looks_like_text(data) {
        let shown = &data[..data.len().min(MAX_TEXT_BYTES)];
        // The game's files are Latin-1, where every byte is the character of the same number.
        let text: String = shown.iter().map(|&b| char::from(b)).collect();
        let lines = text.lines().map(|line| line.replace('\t', "    ")).collect();
        let note = (data.len() > shown.len()).then(|| {
            format!(
                "First {} of {}",
                format::size(shown.len() as u64),
                format::size(data.len() as u64)
            )
        });
        return Decoded::Lines { lines, mono: true, note };
    }
    let shown = &data[..data.len().min(MAX_HEX_BYTES)];
    let lines = shown.chunks(8).enumerate().map(|(row, chunk)| hex_line(row * 8, chunk)).collect();
    let note = (data.len() > shown.len()).then(|| {
        format!("First {} of {}", format::size(shown.len() as u64), format::size(data.len() as u64))
    });
    Decoded::Lines { lines, mono: true, note }
}

fn decode_texture(data: &[u8]) -> zenkit::Result<Decoded> {
    let texture = Texture::from_bytes(data)?;
    let level = (0..texture.mipmap_count())
        .find(|&level| {
            let (w, h) = texture.mipmap_dimensions(level);
            w.max(h) <= MAX_IMAGE_SIDE
        })
        .unwrap_or(texture.mipmap_count().saturating_sub(1));
    let (width, height) = texture.mipmap_dimensions(level);
    let mut bgra = texture.to_rgba8(level)?;
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let mut details = format!(
        "{}×{} · {:?} · {}",
        texture.width,
        texture.height,
        texture.format,
        format::count(texture.mipmap_count(), "mipmap", "mipmaps")
    );
    if level > 0 {
        details.push_str(&format!(" · shown at {width}×{height}"));
    }
    if texture.format.is_refused_by_game() {
        details.push_str(" · the game refuses this format");
    }
    Ok(Decoded::Image { width, height, bgra, details })
}

/// Text if the start has no NUL bytes and is mostly printable, which holds for scripts, model
/// scripts, ASCII ZenGin archives and the like.
fn looks_like_text(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    if head.is_empty() || head.contains(&0) {
        return head.is_empty();
    }
    let printable =
        head.iter().filter(|&&b| b >= 0x20 || matches!(b, b'\n' | b'\r' | b'\t')).count();
    printable * 100 >= head.len() * 95
}

/// Eight bytes a line, so that a line fits the preview at its usual width.
fn hex_line(offset: usize, chunk: &[u8]) -> String {
    let mut line = format!("{offset:08X}  ");
    for i in 0..8 {
        match chunk.get(i) {
            Some(byte) => line.push_str(&format!("{byte:02X} ")),
            None => line.push_str("   "),
        }
    }
    line.push(' ');
    line.extend(
        chunk.iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '·' }),
    );
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_lines_line_up() {
        assert_eq!(hex_line(16, b"MZ\x90\x00"), "00000010  4D 5A 90 00              MZ··");
        assert_eq!(hex_line(0, &[b'a'; 8]).len(), hex_line(0, &[b'a'; 3]).len() + 5);
    }

    #[test]
    fn sniffs_text() {
        assert!(looks_like_text(b"INSTANCE PC_HERO (C_NPC)\r\n{\r\n\tname = \"Ich\";\r\n}"));
        assert!(!looks_like_text(b"ZTEX\x00\x00\x00\x00"));
        assert!(!looks_like_text(&[0x01; 100]));
    }

    #[test]
    fn decodes_a_texture_to_bgra() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/erz.tex");
        let node = Node::file("ERZ.TEX", std::fs::read(path).unwrap(), std::time::UNIX_EPOCH);
        let Decoded::Image { width, height, bgra, details } = decode(&node, FileKind::Texture)
        else {
            panic!("not decoded as an image");
        };
        assert_eq!(bgra.len(), (width * height * 4) as usize);
        assert!(details.contains("mipmap"), "{details}");
    }
}
