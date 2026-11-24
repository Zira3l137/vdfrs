//! The small window a context menu command shows while it extracts or packs, as 7-Zip does. It
//! closes itself when the work succeeds, and stays open to say what went wrong when it fails.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::App;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::ParentElement as _;
use gpui_kit::PathPromptOptions;
use gpui_kit::Render;
use gpui_kit::SharedString;
use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::Styled as _;
use gpui_kit::Task;
use gpui_kit::Window;
use gpui_kit::WindowBounds;
use gpui_kit::WindowOptions;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::h_flex;
use gpui_kit::component::progress::Progress as ProgressBar;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use gpui_kit::size;

use crate::format;
use crate::jobs;
use crate::jobs::Destination;
use crate::jobs::Progress;

const WIDTH: f32 = 460.;
const HEIGHT: f32 = 170.;
/// Tall enough for a few lines of what went wrong, and the button to close.
const FAILED_HEIGHT: f32 = 260.;

#[derive(Debug, Clone)]
pub enum Job {
    /// Extract archives; with no destination, ask for a folder first.
    Extract {
        archives: Vec<PathBuf>,
        dest: Option<Destination>,
    },
    Pack {
        paths: Vec<PathBuf>,
        out: PathBuf,
    },
}

enum State {
    Running,
    Failed(SharedString),
}

pub struct TaskView {
    title: SharedString,
    detail: SharedString,
    progress: Arc<Progress>,
    state: State,
    _work: Option<Task<()>>,
    _ticks: Option<Task<()>>,
}

impl TaskView {
    fn new(job: Job, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (title, detail) = describe(&job);
        let mut view = Self {
            title: title.into(),
            detail: detail.into(),
            progress: Arc::default(),
            state: State::Running,
            _work: None,
            _ticks: None,
        };
        match job {
            Job::Extract { archives, dest: None } => view.pick_folder(archives, window, cx),
            job => view.start(job, window, cx),
        }
        view
    }

    fn pick_folder(&mut self, archives: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Extract here".into()),
        });
        self._work = Some(cx.spawn_in(window, async move |this, cx| {
            let picked = prompt.await.ok().and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                match picked.and_then(|p| p.into_iter().next()) {
                    Some(folder) => {
                        let job =
                            Job::Extract { archives, dest: Some(Destination::Folder(folder)) };
                        this.start(job, window, cx);
                    }
                    None => window.remove_window(),
                }
            });
        }));
    }

    fn start(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        let progress = self.progress.clone();
        let work = cx.background_spawn(async move { run(job, &progress) });
        self._work = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => window.remove_window(),
                Err(reason) => {
                    window.resize(size(px(WIDTH), px(FAILED_HEIGHT)));
                    this.state = State::Failed(reason.into());
                    this._ticks = None;
                    cx.notify();
                }
            });
        }));
        // The work counts files on another thread; redraw a few times a second to show it.
        self._ticks = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }
}

/// Runs a job to the end, gathering the failures of a batch rather than stopping at the first.
pub fn run(job: Job, progress: &Progress) -> Result<(), String> {
    match job {
        Job::Extract { archives, dest } => {
            let dest = dest.unwrap_or(Destination::OwnFolder);
            let failures: Vec<String> = archives
                .iter()
                .filter_map(|archive| jobs::extract(archive, &dest, progress).err())
                .map(|error| error.report().to_string())
                .collect();
            if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
        }
        Job::Pack { paths, out } => {
            jobs::pack(&paths, &out, progress).map(drop).map_err(|e| e.report().to_string())
        }
    }
}

/// Runs a job from a terminal: no window, a line to say what it did, and the reason if it failed.
#[allow(clippy::print_stdout)]
pub fn run_in_terminal(job: Job) -> Result<(), String> {
    let (title, _) = describe(&job);
    let progress = Progress::default();
    let target = match &job {
        Job::Pack { out, .. } => Some(out.display().to_string()),
        Job::Extract { dest: Some(Destination::Folder(folder)), .. } => {
            Some(folder.display().to_string())
        }
        Job::Extract { .. } => None,
    };
    run(job, &progress)?;
    let done = format::count(progress.done.load(Ordering::Relaxed), "file", "files");
    let verb = title.split_whitespace().next().unwrap_or("Done");
    let past = if verb == "Packing" { "Packed" } else { "Extracted" };
    match target {
        Some(target) => println!("{past} {done} into {target}"),
        None => println!("{past} {done}"),
    }
    Ok(())
}

fn describe(job: &Job) -> (String, String) {
    let name = |path: &PathBuf| path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match job {
        Job::Extract { archives, .. } => match archives.as_slice() {
            [one] => (format!("Extracting {}", name(one)), one.display().to_string()),
            many => (
                format!("Extracting {}", format::count(many.len(), "archive", "archives")),
                many.iter().map(name).collect::<Vec<_>>().join(", "),
            ),
        },
        Job::Pack { paths, out } => (
            format!("Packing {}", name(out)),
            format!("{} into {}", format::count(paths.len(), "item", "items"), out.display()),
        ),
    }
}

impl Render for TaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let done = self.progress.done.load(Ordering::Relaxed);
        let total = self.progress.total.load(Ordering::Relaxed);
        let failed = matches!(self.state, State::Failed(_));
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Icon::new(if failed { IconName::CircleAlert } else { IconName::Package })
                            .large()
                            .text_color(if failed { theme.danger } else { theme.primary }),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().font_semibold().child(self.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(self.detail.clone()),
                            ),
                    ),
            )
            .map(|this| match &self.state {
                State::Running => this
                    .child(match self.progress.fraction() {
                        // Not the progress component: its fill eases towards each new value, and
                        // a value that changes ten times a second keeps it from ever arriving.
                        Some(fraction) => div()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(theme.progress_bar.opacity(0.2))
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui_kit::relative(fraction.clamp(0., 1.)))
                                    .rounded_full()
                                    .bg(theme.progress_bar),
                            )
                            .into_any_element(),
                        None => ProgressBar::new("progress").loading(true).into_any_element(),
                    })
                    .child(div().text_xs().text_color(theme.muted_foreground).child(match total {
                        0 => "Reading…".to_owned(),
                        _ => format!(
                            "{} of {}",
                            format::grouped(done as u64),
                            format::count(total, "file", "files")
                        ),
                    })),
                State::Failed(reason) => this
                    .child(
                        div()
                            .id("reason")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(reason.clone()),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Button::new("close")
                                .label("Close")
                                .on_click(|_, window, _| window.remove_window()),
                        ),
                    ),
            })
    }
}

pub fn open(job: Job, cx: &mut App) -> gpui_kit::Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx)),
        is_resizable: false,
        is_minimizable: true,
        app_id: Some("vdfrs".into()),
        ..WindowOptions::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("vdfrs");
        cx.new(|cx| TaskView::new(job, window, cx))
    })?;
    Ok(())
}
