//! The folder tree beside the list. Only folders are in it; every row is a drop target.

use std::ops::Range;

use gpui_kit::AnyElement;
use gpui_kit::Context;
use gpui_kit::ElementId;
use gpui_kit::ExternalPaths;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::ParentElement as _;
use gpui_kit::SharedString;
use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::Styled as _;
use gpui_kit::Window;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::h_flex;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::div;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use gpui_kit::uniform_list;

use crate::archive_view::ArchiveView;
use crate::drag::DraggedEntries;

const INDENT: f32 = 14.;

impl ArchiveView {
    pub(crate) fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        div()
            .relative()
            .size_full()
            .bg(theme.sidebar)
            .child(
                // A new id after each edit, as the file list's.
                uniform_list(
                    ("tree", self.archive.generation()),
                    self.tree_rows.len(),
                    cx.processor(|this, range: Range<usize>, window, cx| {
                        range.map(|ix| this.render_tree_row(ix, window, cx)).collect::<Vec<_>>()
                    }),
                )
                .size_full()
                .py_1()
                .track_scroll(&self.tree_scroll),
            )
            .vertical_scrollbar(&self.tree_scroll)
            .into_any_element()
    }

    fn render_tree_row(&self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.tree_rows[ix];
        let path = row.path.to_string();
        let current = row.path.eq_ignore_ascii_case(&self.dir);
        let icon = match (ix, row.expanded && row.has_children) {
            (0, _) => IconName::Package,
            (_, true) => IconName::FolderOpen,
            _ => IconName::Folder,
        };
        let chevron = row.has_children && ix > 0;

        h_flex()
            .id(ElementId::Name(SharedString::from(format!("tree:{path}"))))
            .h(px(26.))
            .w_full()
            .mx_1()
            .pr_2()
            .pl(px(4. + INDENT * row.depth.saturating_sub(1) as f32))
            .gap_1()
            .rounded(theme.radius)
            .text_sm()
            .text_color(theme.sidebar_foreground)
            .when(current, |this| {
                this.bg(theme.sidebar_accent).text_color(theme.sidebar_accent_foreground)
            })
            .when(!current, |this| this.hover(|this| this.bg(theme.sidebar_accent.opacity(0.5))))
            .when(ix > 0, |this| {
                this.child(
                    div()
                        .id(ElementId::Name(SharedString::from(format!("toggle:{path}"))))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme.radius)
                        .when(chevron, |this| {
                            let icon = if row.expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            };
                            this.child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
                                .hover(|this| this.bg(theme.sidebar_accent))
                                .on_click(cx.listener({
                                    let path = path.clone();
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_expanded(&path, cx);
                                    }
                                }))
                        }),
                )
            })
            .child(Icon::new(icon).small().text_color(if ix == 0 {
                theme.primary
            } else {
                theme.yellow
            }))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(ix == 0, |this| this.font_weight(gpui_kit::FontWeight::MEDIUM))
                    .child(row.name.clone()),
            )
            .on_click(cx.listener({
                let path = path.clone();
                move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    if event.click_count() >= 2 && path != "/" {
                        this.toggle_expanded(&path, cx);
                    }
                    this.navigate(&path, window, cx);
                    window.focus(&this.focus, cx);
                }
            }))
            .drag_over::<DraggedEntries>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .on_drop(cx.listener({
                let path = path.clone();
                move |this, dragged: &DraggedEntries, window, cx| {
                    this.drop_entries(dragged, &path, window, cx)
                }
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_paths(paths, &path, window, cx)
            }))
            .into_any_element()
    }
}
