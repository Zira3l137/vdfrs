//! vdfrs: browse, edit and pack the ZenGin's `.vdf` and `.mod` archives.
//!
//! ```text
//! vdfrs [ARCHIVE...]
//! ```

// No console window behind the explorer on Windows, except in debug builds where its log helps.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod archive;
mod archive_view;
mod drag;
mod format;
mod kind;
mod list;
mod preview;
mod settings;
mod tree;
mod workspace;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui_kit::AppContext as _;
use gpui_kit::AssetSource;
use gpui_kit::SharedString;
use gpui_kit::WindowBounds;
use gpui_kit::WindowOptions;
use gpui_kit::component::Theme;
use gpui_kit::component::TitleBar;
use gpui_kit::px;
use gpui_kit::size;

use crate::workspace::Workspace;

// The default asset source carries only the components' own icons.
gpui_kit::assets::icon_assets!(
    ExplorerIcons,
    [
        ArrowLeft,
        ArrowRight,
        ArrowUp,
        AudioLines,
        Box,
        ChevronDown,
        ChevronRight,
        ChevronUp,
        Drama,
        Earth,
        File,
        FileCode,
        FileImage,
        FilePlus,
        FileText,
        Film,
        Folder,
        FolderInput,
        FolderOpen,
        FolderOutput,
        FolderPlus,
        Footprints,
        Layers,
        MessageSquareText,
        Moon,
        Music,
        Package,
        PanelRight,
        PersonStanding,
        Plus,
        ScrollText,
        Search,
        Settings,
        Sun,
        Trash,
        Type,
        X
    ]
);

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match ExplorerIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExplorerIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    std::thread::spawn(drag::clean_old_drags);

    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        actions::bind(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(None, size(px(1280.), px(800.)), cx)),
            titlebar: Some(TitleBar::title_bar_options()),
            window_min_size: Some(size(px(720.), px(420.))),
            app_id: Some("vdfrs".into()),
            ..WindowOptions::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.new(|cx| Workspace::new(paths, window, cx))
        });
        if let Err(error) = opened {
            eprintln!("cannot open the window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
