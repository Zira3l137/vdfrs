//! vdfrs: browse, edit and pack the ZenGin's `.vdf` and `.mod` archives.
//!
//! ```text
//! vdfrs [ARCHIVE...]
//! vdfrs extract|pack|new|shell ...
//! ```

// No console window behind the explorer on Windows, except in debug builds where its log helps.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod archive;
mod archive_view;
mod cli;
mod drag;
mod format;
mod gather;
mod jobs;
mod kind;
mod list;
mod preview;
mod settings;
mod shell;
mod task;
mod tree;
mod workspace;

use std::borrow::Cow;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser as _;

use gpui_kit::AppContext as _;
use gpui_kit::AssetSource;
use gpui_kit::SharedString;
use gpui_kit::WindowBounds;
use gpui_kit::WindowOptions;
use gpui_kit::component::Theme;
use gpui_kit::component::TitleBar;
use gpui_kit::px;
use gpui_kit::size;

use crate::cli::Cli;
use crate::cli::Command;
use crate::cli::ShellAction;
use crate::jobs::Destination;
use crate::task::Job;
use crate::workspace::Startup;
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
        CircleAlert,
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
        FolderCog,
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

fn main() -> ExitCode {
    // In release builds on Windows the program has no console of its own, so output goes to the
    // terminal it was started from, if any; from Explorer there is none, and nothing changes.
    #[cfg(windows)]
    attach_console();

    let cli = Cli::parse();
    let run = match cli.command {
        None => match gathered("open", cli.archives, cli.gather) {
            Some(archives) => Run::Explorer(Startup::Open(archives)),
            None => return ExitCode::SUCCESS,
        },
        Some(Command::New { paths, gather }) => match gathered("new", paths, gather) {
            Some(paths) => Run::Explorer(Startup::New(paths)),
            None => return ExitCode::SUCCESS,
        },
        Some(Command::Extract { archives, here, to, pick, gather }) => {
            let dest = match (here, to, pick) {
                (true, ..) => Some(Destination::Here),
                (_, Some(folder), _) => Some(Destination::Folder(folder)),
                (_, None, true) => None,
                _ => Some(Destination::OwnFolder),
            };
            match gathered("extract", archives, gather) {
                Some(archives) => Run::Task(Job::Extract { archives, dest }),
                None => return ExitCode::SUCCESS,
            }
        }
        Some(Command::Pack { paths, out, gather }) => match gathered("pack", paths, gather) {
            Some(paths) => {
                let out = out.unwrap_or_else(|| jobs::pack_output(&paths));
                Run::Task(Job::Pack { paths, out })
            }
            None => return ExitCode::SUCCESS,
        },
        Some(Command::Shell { action }) => return shell_command(action),
    };
    run_app(run);
    ExitCode::SUCCESS
}

enum Run {
    Explorer(Startup),
    Task(Job),
}

/// The paths of the whole selection if this process is the one to act on it, see [`gather`].
fn gathered(command: &str, paths: Vec<PathBuf>, gather: bool) -> Option<Vec<PathBuf>> {
    if gather { gather::gather(command, paths) } else { Some(paths) }
}

#[allow(clippy::print_stdout)]
fn shell_command(action: ShellAction) -> ExitCode {
    let result = match action {
        ShellAction::Install { associate } => shell::install(associate).map(|()| {
            println!(
                "vdfrs is in Explorer's context menu{}",
                if associate { ", and opens .vdf and .mod files" } else { "" }
            );
        }),
        ShellAction::Uninstall => shell::uninstall().map(|()| println!("vdfrs is out of Explorer")),
        ShellAction::Status => {
            match shell::status() {
                shell::Status::NotInstalled => println!("not in Explorer"),
                shell::Status::Installed { exe, associated } => {
                    println!(
                        "in Explorer, for {exe}{}",
                        if associated { ", opening archives" } else { "" }
                    )
                }
            }
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vdfrs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_app(run: Run) {
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

        let opened = match run {
            Run::Task(job) => task::open(job, cx),
            Run::Explorer(startup) => {
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::centered(
                        None,
                        size(px(1280.), px(800.)),
                        cx,
                    )),
                    titlebar: Some(TitleBar::title_bar_options()),
                    window_min_size: Some(size(px(720.), px(420.))),
                    app_id: Some("vdfrs".into()),
                    ..WindowOptions::default()
                };
                gpui_kit::open_window(options, cx, |window, cx| {
                    Theme::sync_system_appearance(Some(window), cx);
                    cx.new(|cx| Workspace::new(startup, window, cx))
                })
                .map(drop)
            }
        };
        if let Err(error) = opened {
            eprintln!("vdfrs: cannot open the window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    use windows::Win32::System::Console::ATTACH_PARENT_PROCESS;
    use windows::Win32::System::Console::AttachConsole;
    // SAFETY: a plain request about this process; it fails harmlessly when there is no parent
    // console or one is attached already.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}
