//! The window: archives as tabs in the title bar, and a place to drop archives when none is open.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use gpui_kit::AppContext as _;
use gpui_kit::Context;
use gpui_kit::Entity;
use gpui_kit::EntityId;
use gpui_kit::ExternalPaths;
use gpui_kit::FocusHandle;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::MouseButton;
use gpui_kit::ParentElement as _;
use gpui_kit::PathPromptOptions;
use gpui_kit::Render;
use gpui_kit::SharedString;
use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::Styled as _;
use gpui_kit::Subscription;
use gpui_kit::Window;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::Theme;
use gpui_kit::component::ThemeMode;
use gpui_kit::component::TitleBar;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::h_flex;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;

use crate::actions::*;
use crate::archive::Archive;
use crate::archive_view::ArchiveEvent;
use crate::archive_view::ArchiveView;
use crate::drag::DraggedEntries;

/// What the window starts with.
pub enum Startup {
    Open(Vec<PathBuf>),
    /// A new archive holding these files and folders.
    New(Vec<PathBuf>),
}

pub struct Workspace {
    tabs: Vec<Entity<ArchiveView>>,
    /// The order each tab was asked for in, so that archives that load at different speeds
    /// still line up in the order they were opened.
    order: Vec<u64>,
    next_order: u64,
    active: usize,
    focus: FocusHandle,
    opening: Vec<SharedString>,
    /// Tabs waiting for their save to finish before they close.
    closing: HashSet<EntityId>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(startup: Startup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = Self {
            tabs: Vec::new(),
            order: Vec::new(),
            next_order: 0,
            active: 0,
            focus: cx.focus_handle(),
            opening: Vec::new(),
            closing: HashSet::new(),
            _subscriptions: Vec::new(),
        };
        window.focus(&workspace.focus, cx);
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.window_should_close(window, cx)).unwrap_or(true)
        });
        let entity = cx.entity();
        window.defer(cx, move |window, cx| {
            entity.update(cx, |this, cx| match startup {
                Startup::Open(paths) => this.open_paths(paths, window, cx),
                Startup::New(paths) => this.new_archive_with(paths, window, cx),
            });
        });
        workspace
    }

    fn active_tab(&self) -> Option<&Entity<ArchiveView>> {
        self.tabs.get(self.active)
    }

    pub fn open_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            let already = self.tabs.iter().position(|tab| {
                tab.read(cx).archive().path().is_some_and(|open| same_file(open, &path))
            });
            if let Some(ix) = already {
                self.activate(ix, window, cx);
                continue;
            }
            let name: SharedString =
                path.file_name().unwrap_or_default().to_string_lossy().into_owned().into();
            let order = self.take_order();
            self.opening.push(name.clone());
            cx.notify();
            let loading = cx.background_spawn(async move { Archive::open(&path) });
            cx.spawn_in(window, async move |this, cx| {
                let archive = loading.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(ix) = this.opening.iter().position(|n| *n == name) {
                        this.opening.remove(ix);
                    }
                    match archive {
                        Ok(archive) => this.add_tab(archive, order, window, cx),
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.report().to_string()),
                                cx,
                            );
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
        }
    }

    fn take_order(&mut self) -> u64 {
        self.next_order += 1;
        self.next_order
    }

    fn add_tab(
        &mut self,
        archive: Archive,
        order: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = cx.new(|cx| ArchiveView::new(archive, window, cx));
        self._subscriptions.push(cx.observe_in(&tab, window, |this, _, window, cx| {
            this.update_title(window, cx);
            cx.notify();
        }));
        self._subscriptions.push(cx.subscribe_in(&tab, window, |this, tab, event, window, cx| {
            let id = tab.entity_id();
            if !this.closing.remove(&id) {
                return;
            }
            if matches!(event, ArchiveEvent::Saved)
                && let Some(ix) = this.tabs.iter().position(|t| t.entity_id() == id)
            {
                this.remove_tab(ix, window, cx);
            }
        }));
        let ix = self.order.partition_point(|&other| other < order);
        self.tabs.insert(ix, tab);
        self.order.insert(ix, order);
        self.activate(ix, window, cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(ix) {
            self.active = ix;
            let focus = tab.read(cx).focus.clone();
            window.focus(&focus, cx);
            self.update_title(window, cx);
            cx.notify();
        }
    }

    fn update_title(&self, window: &mut Window, cx: &Context<Self>) {
        let title = match self.active_tab() {
            Some(tab) => {
                let archive = tab.read(cx).archive();
                let dirty = if archive.is_dirty() { "• " } else { "" };
                format!("{dirty}{} — vdfrs", archive.title())
            }
            None => "vdfrs".into(),
        };
        window.set_window_title(&title);
    }

    fn open_archive(&mut self, _: &OpenArchive, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ = this.update_in(cx, |this, window, cx| this.open_paths(paths, window, cx));
            }
        })
        .detach();
    }

    fn new_archive(&mut self, _: &NewArchive, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.take_order();
        self.add_tab(Archive::new(), order, window, cx);
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix).cloned() else { return };
        if !tab.read(cx).archive().is_dirty() {
            self.remove_tab(ix, window, cx);
            return;
        }
        let title = format!("Save changes to “{}”?", tab.read(cx).archive().title());
        let this = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (this, tab, discard) = (this.clone(), tab.clone(), this.clone());
            let id = tab.entity_id();
            dialog
                .title(title.clone())
                .w(px(420.))
                .child("Changes you don't save are lost.")
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("discard").label("Don't save").on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = discard.update(cx, |this, cx| {
                                    if let Some(ix) =
                                        this.tabs.iter().position(|t| t.entity_id() == id)
                                    {
                                        this.remove_tab(ix, window, cx);
                                    }
                                });
                            },
                        ))
                        .child(Button::new("save").primary().label("Save").on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = this.update(cx, |this, _| this.closing.insert(id));
                                tab.update(cx, |tab, cx| tab.save(&Save, window, cx));
                            },
                        )),
                )
        });
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.remove(ix);
        self.order.remove(ix);
        if self.tabs.is_empty() {
            self.active = 0;
            window.focus(&self.focus, cx);
            self.update_title(window, cx);
            cx.notify();
        } else {
            let next = if self.active > ix || self.active == self.tabs.len() {
                self.active - 1
            } else {
                self.active
            };
            self.activate(next.min(self.tabs.len() - 1), window, cx);
        }
    }

    fn window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dirty: Vec<String> = self
            .tabs
            .iter()
            .map(|tab| tab.read(cx).archive())
            .filter(|archive| archive.is_dirty())
            .map(|archive| archive.title())
            .collect();
        if dirty.is_empty() {
            return true;
        }
        let title = match dirty.as_slice() {
            [one] => format!("Close without saving “{one}”?"),
            many => format!("Close without saving {} archives?", many.len()),
        };
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(title.clone())
                .description("Changes you don't save are lost.")
                .confirm()
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("Discard changes")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let _ = this.update(cx, |this, _| {
                        this.tabs.clear();
                        this.order.clear();
                    });
                    window.remove_window();
                    true
                })
        });
        false
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_should_close(window, cx) {
            cx.quit();
        }
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
        Theme::change(mode, Some(window), cx);
    }

    fn step_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            let len = self.tabs.len() as isize;
            let ix = (self.active as isize + delta).rem_euclid(len);
            self.activate(ix as usize, window, cx);
        }
    }

    /// Dropped files: archives open in tabs, anything else is packed into a new archive.
    fn drop_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let (archives, others): (Vec<PathBuf>, Vec<PathBuf>) =
            paths.paths().iter().cloned().partition(|path| is_archive(path));
        self.open_paths(archives, window, cx);
        self.new_archive_with(others, window, cx);
    }

    /// Opens a new archive holding `paths`, read from disk in the background.
    pub fn new_archive_with(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let order = self.take_order();
        self.add_tab(Archive::new(), order, window, cx);
        if let Some(tab) = self.active_tab().cloned() {
            tab.update(cx, |tab, cx| tab.add_paths(paths, "/".into(), window, cx));
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .id("tabs")
            .h_full()
            .min_w_0()
            .gap_1()
            .overflow_x_scroll()
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let active = ix == self.active;
                let view = tab.read(cx);
                let archive = view.archive();
                let tab_id = tab.entity_id();
                let target = tab.downgrade();
                let tooltip = archive
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Not saved yet".into());
                h_flex()
                    .id(("tab", tab_id.as_u64()))
                    .h(px(30.))
                    .pl_3()
                    .pr_1()
                    .gap_2()
                    .max_w(px(240.))
                    .rounded(theme.radius)
                    .text_sm()
                    .when(active, |this| {
                        this.bg(theme.background).border_1().border_color(theme.border)
                    })
                    .when(!active, |this| {
                        this.text_color(theme.muted_foreground)
                            .hover(|this| this.bg(theme.secondary_hover))
                    })
                    .child(Icon::new(IconName::Package).small().text_color(if active {
                        theme.primary
                    } else {
                        theme.muted_foreground
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(archive.title()),
                    )
                    .when(archive.is_dirty(), |this| {
                        this.child(div().size(px(6.)).rounded_full().bg(theme.warning))
                    })
                    .when(view.is_busy(), |this| this.child(Spinner::new().xsmall()))
                    .child(
                        Button::new(("close-tab", tab_id.as_u64()))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip_with_action("Close", &CloseTab, Some(WORKSPACE))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(ix) =
                                    this.tabs.iter().position(|t| t.entity_id() == tab_id)
                                {
                                    this.close_tab(ix, window, cx);
                                }
                            })),
                    )
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(ix, window, cx)),
                    )
                    .drag_over::<DraggedEntries>(move |style, dragged, _, cx| {
                        if dragged.source_id == tab_id {
                            style
                        } else {
                            style.bg(cx.theme().drop_target)
                        }
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedEntries, window, cx| {
                        if dragged.source_id == tab_id {
                            return;
                        }
                        if let Some(target) = target.upgrade() {
                            let dir = target.read(cx).dir.to_string();
                            target
                                .update(cx, |tab, cx| tab.drop_entries(dragged, &dir, window, cx));
                        }
                        this.activate(ix, window, cx);
                    }))
            }))
            .children(self.opening.iter().map(|name| {
                h_flex()
                    .h(px(30.))
                    .px_3()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new().xsmall())
                    .child(name.clone())
            }))
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("welcome")
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .drag_over::<ExternalPaths>(|style, _, _, cx| {
                style.bg(cx.theme().drop_target.opacity(0.35))
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.drop_paths(paths, window, cx)
            }))
            .child(
                v_flex()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .w(px(440.))
                    .h(px(260.))
                    .rounded(theme.radius_lg)
                    .border_2()
                    .border_dashed()
                    .border_color(theme.border)
                    .child(Icon::new(IconName::Package).size(px(48.)).text_color(theme.primary))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("Drop a .vdf or .mod here"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Other files and folders start a new archive"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .pt_2()
                            .child(
                                Button::new("open")
                                    .primary()
                                    .icon(IconName::FolderOpen)
                                    .label("Open…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_archive(&OpenArchive, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("new")
                                    .icon(IconName::Plus)
                                    .label("New archive")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_archive(&NewArchive, window, cx)
                                    })),
                            ),
                    ),
            )
            .when(!self.opening.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(Spinner::new().small())
                        .child(format!("Opening {}…", self.opening.join(", "))),
                )
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dark = theme.is_dark();
        let count = self.tabs.len();
        v_flex()
            .id("workspace")
            .key_context(WORKSPACE)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::open_archive))
            .on_action(cx.listener(Self::new_archive))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(
                cx.listener(|this, _: &CloseTab, window, cx| {
                    this.close_tab(this.active, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step_tab(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousTab, window, cx| this.step_tab(-1, window, cx)),
            )
            .child(
                TitleBar::new().child(
                    h_flex()
                        .id("title")
                        .size_full()
                        .min_w_0()
                        .pr_2()
                        .gap_2()
                        .drag_over::<ExternalPaths>(|style, _, _, cx| {
                            style.bg(cx.theme().drop_target.opacity(0.35))
                        })
                        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                            this.drop_paths(paths, window, cx)
                        }))
                        .child(self.render_tabs(cx))
                        .child(
                            Button::new("open-tab")
                                .ghost()
                                .small()
                                .icon(IconName::FolderOpen)
                                .tooltip_with_action("Open archive…", &OpenArchive, Some(WORKSPACE))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_archive(&OpenArchive, window, cx)
                                })),
                        )
                        .child(
                            Button::new("new-tab")
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .tooltip_with_action("New archive", &NewArchive, Some(WORKSPACE))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_archive(&NewArchive, window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .when(count > 0, |this| {
                            this.child(
                                Button::new("settings")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Settings)
                                    .tooltip("Archive settings…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(tab) = this.active_tab().cloned() {
                                            crate::settings::open(tab, window, cx);
                                        }
                                    })),
                            )
                        })
                        .when(cfg!(windows), |this| {
                            this.child(
                                Button::new("integration")
                                    .ghost()
                                    .small()
                                    .icon(IconName::FolderCog)
                                    .tooltip("Explorer integration…")
                                    .on_click(|_, window, cx| {
                                        crate::settings::open_integration(window, cx)
                                    }),
                            )
                        })
                        .child(
                            Button::new("theme")
                                .ghost()
                                .small()
                                .icon(if dark { IconName::Sun } else { IconName::Moon })
                                .tooltip(if dark { "Light theme" } else { "Dark theme" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_theme(&ToggleTheme, window, cx)
                                })),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().map(|this| match self.active_tab() {
                Some(tab) => this.child(tab.clone()),
                None => this.child(self.render_welcome(cx)),
            }))
    }
}

fn is_archive(path: &Path) -> bool {
    let ext = path.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    matches!(ext.as_deref(), Some("vdf" | "mod")) && path.is_file()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
