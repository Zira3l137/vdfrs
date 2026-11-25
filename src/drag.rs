//! What is dragged around the explorer, how it looks under the pointer, and how it leaves the
//! window as real files.
//!
//! Inside the window a drag is GPUI's own: rows carry a [`DraggedEntries`] to folders, the tree
//! and other tabs. Programs outside only take files on disk, so once the pointer leaves the
//! window the entries are extracted to a temporary folder and those files are handed to the
//! system's drag instead. GPUI does that hand-over itself where its backend can (Wayland); on
//! Windows, whose backend cannot yet, [`windows::start`] runs the shell's drag.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use gpui_kit::App;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use gpui_kit::EntityId;
use gpui_kit::ExternalDragPayload;
use gpui_kit::FileDragPaths;
use gpui_kit::IntoElement;
use gpui_kit::ParentElement as _;
use gpui_kit::Render;
use gpui_kit::SharedString;
use gpui_kit::Styled as _;
use gpui_kit::WeakEntity;
use gpui_kit::Window;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use zenkit::vfs::Node;

use crate::archive;
use crate::archive_view::ArchiveView;

/// The most a drag out of the window extracts. The pointer waits while it does, so a larger drag
/// is refused and the user pointed to Extract instead.
pub const DRAG_OUT_LIMIT: u64 = 512 << 20;

#[derive(Clone)]
pub struct DraggedEntries {
    pub source: WeakEntity<ArchiveView>,
    pub source_id: EntityId,
    pub paths: Rc<[Arc<str>]>,
    pub icon: IconName,
    pub label: SharedString,
    /// Set once the drag has left the window and become the system's. Only Windows hands a drag
    /// out itself.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub handed_out: Rc<Cell<bool>>,
}

impl DraggedEntries {
    pub fn paths(&self) -> Vec<String> {
        self.paths.iter().map(|path| path.to_string()).collect()
    }

    pub fn ghost(&self, cx: &mut App) -> gpui_kit::Entity<DragGhost> {
        let ghost =
            DragGhost { icon: self.icon, label: self.label.clone(), count: self.paths.len() };
        cx.new(|_| ghost)
    }

    /// The source's nodes, copied; empty once the source tab is closed.
    pub fn nodes(&self, cx: &App) -> Vec<Node> {
        self.source
            .upgrade()
            .map(|source| source.read(cx).archive().take_copies(&self.paths()))
            .unwrap_or_default()
    }

    /// Extracts the dragged entries for a drag leaving the window, see the module's docs.
    pub fn external_payload(&self, cx: &App) -> Option<ExternalDragPayload> {
        let nodes = self.nodes(cx);
        let total: u64 = nodes
            .iter()
            .flat_map(|node| std::iter::once(node).chain(node.descendants()))
            .filter_map(|node| node.size())
            .map(|size| size as u64)
            .sum();
        if nodes.is_empty() || total > DRAG_OUT_LIMIT {
            return None;
        }
        let dir = new_drag_dir().ok()?;
        let paths = archive::extract(&nodes, &dir, &Default::default()).ok()?;
        let entries = paths.into_iter().zip(nodes.iter().map(Node::is_dir));
        Some(ExternalDragPayload::Files(FileDragPaths::new(entries)))
    }
}

pub struct DragGhost {
    icon: IconName,
    label: SharedString,
    count: usize,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_sm()
            .text_color(theme.popover_foreground)
            .child(Icon::new(self.icon).small())
            .child(self.label.clone())
            .when(self.count > 1, |this| {
                this.child(
                    h_flex()
                        .px_1p5()
                        .h(px(18.))
                        .rounded(px(9.))
                        .bg(theme.primary)
                        .text_xs()
                        .text_color(theme.primary_foreground)
                        .child(self.count.to_string()),
                )
            })
    }
}

fn drag_root() -> PathBuf {
    std::env::temp_dir().join("vdfrs")
}

/// A fresh folder for one drag's files. Every drag gets its own, since the program that takes
/// the files may read them long after the drag.
pub fn new_drag_dir() -> std::io::Result<PathBuf> {
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let dir = drag_root().join(format!("drag-{:x}", stamp.as_nanos()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Whether `path` is one of the files a drag out of the window made, which a drop back into the
/// window should treat as the entries themselves.
pub fn is_drag_file(path: &std::path::Path) -> bool {
    path.starts_with(drag_root())
}

/// Removes the folders of drags from earlier runs. Those of the last day stay, in case a program
/// is still reading them.
pub fn clean_old_drags() {
    let Ok(entries) = std::fs::read_dir(drag_root()) else { return };
    let day = Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|time| time.elapsed().unwrap_or_default() > day);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows {
    //! The shell's drag, run on a thread of its own: `SHDoDragDrop` loops until the drop, and
    //! running that loop inside one of GPUI's event handlers would let window messages reach
    //! GPUI while it is busy with that handler.

    use std::path::Path;
    use std::path::PathBuf;

    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::DROPEFFECT_COPY;
    use windows::Win32::System::Ole::OleInitialize;
    use windows::Win32::System::Ole::OleUninitialize;
    use windows::Win32::System::Threading::AttachThreadInput;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::ILFindLastID;
    use windows::Win32::UI::Shell::SHCreateDataObject;
    use windows::Win32::UI::Shell::SHDoDragDrop;
    use windows::Win32::UI::Shell::SHParseDisplayName;
    use windows::core::HSTRING;

    /// An item ID list the shell allocated, freed when dropped.
    struct Pidl(*mut ITEMIDLIST);

    impl Drop for Pidl {
        fn drop(&mut self) {
            // SAFETY: the pointer came from SHParseDisplayName, which allocates with the COM
            // allocator, and nothing uses it after this.
            unsafe { CoTaskMemFree(Some(self.0.cast_const().cast())) }
        }
    }

    fn parse(path: &Path) -> windows::core::Result<Pidl> {
        let mut pidl = std::ptr::null_mut();
        // SAFETY: `pidl` is a valid place for the result, which Pidl then owns.
        unsafe { SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut pidl, 0, None)? };
        Ok(Pidl(pidl))
    }

    /// The data object Explorer itself would make for these files, which all share one folder.
    fn data_object(items: &[PathBuf]) -> windows::core::Result<IDataObject> {
        let folder = parse(items[0].parent().unwrap_or(Path::new("")))?;
        let absolute = items.iter().map(|item| parse(item)).collect::<Result<Vec<_>, _>>()?;
        // SAFETY: each absolute ID list is valid while `absolute` lives; ILFindLastID points into
        // it at the last item, which is the ID relative to `folder` the shell wants.
        let children: Vec<*const ITEMIDLIST> =
            absolute.iter().map(|pidl| unsafe { ILFindLastID(pidl.0) }.cast_const()).collect();
        // SAFETY: `folder` and `children` stay alive for the call, which copies them.
        unsafe { SHCreateDataObject(Some(folder.0.cast_const()), Some(children.as_slice()), None) }
    }

    /// Starts the shell's drag of `items`, which must be files in one folder. Call it from the
    /// window's thread while the mouse button is down.
    pub fn start(items: Vec<PathBuf>) {
        if items.is_empty() {
            return;
        }
        // SAFETY: a plain query of the calling thread's ID.
        let ui_thread = unsafe { GetCurrentThreadId() };
        // GPUI holds the mouse capture during its drag; the shell's drag needs it.
        // SAFETY: releases a capture this thread holds, if any; nothing relies on it afterwards.
        let _ = unsafe { ReleaseCapture() };

        std::thread::spawn(move || {
            // SAFETY: OLE is set up and torn down on this thread alone, around the one drag.
            // Attaching to the window's thread shares its input state, so the shell sees the
            // button that is still held down from the window.
            unsafe {
                if OleInitialize(None).is_err() {
                    return;
                }
                let me = GetCurrentThreadId();
                let attached = AttachThreadInput(me, ui_thread, true).as_bool();
                // The top bit of the state says whether the key is down now.
                let held = (GetAsyncKeyState(i32::from(VK_LBUTTON.0)) as u16 & 0x8000) != 0;
                if held && let Ok(data) = data_object(&items) {
                    let _ = SHDoDragDrop(None, &data, None, DROPEFFECT_COPY);
                }
                if attached {
                    let _ = AttachThreadInput(me, ui_thread, false);
                }
                OleUninitialize();
            }
        });
    }
}
