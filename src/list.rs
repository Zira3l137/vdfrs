//! The file list: a header to sort by, and one row per entry of the folder or the search, drawn
//! only for the rows on screen.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::AnyElement;
use gpui_kit::Context;
use gpui_kit::ElementId;
use gpui_kit::ExternalPaths;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::MouseButton;
use gpui_kit::MouseDownEvent;
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
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use gpui_kit::relative;
use gpui_kit::uniform_list;

use crate::actions::*;
use crate::archive_view::ArchiveView;
use crate::archive_view::Column;
use crate::drag::DraggedEntries;
use crate::format;

pub const ROW_HEIGHT: f32 = 26.;
const KIND_WIDTH: f32 = 110.;
const SIZE_WIDTH: f32 = 80.;
const TIME_WIDTH: f32 = 126.;
/// Of the row's width, for the folder column of search results.
const FOLDER_SHARE: f32 = 0.28;

impl ArchiveView {
    pub(crate) fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let searching = !self.query.is_empty();
        let dir = self.dir.to_string();
        let focus = self.focus.clone();
        let view = cx.entity().downgrade();
        let this_tab = cx.entity_id();

        // A new id after each edit gives GPUI Fast a new list, whose rows it cannot mistake for
        // those it kept from before: it kept them across a drop onto a folder.
        let list = uniform_list(
            ("rows", self.archive.generation()),
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, window, cx| {
                range.map(|ix| this.render_row(ix, window, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full()
        .track_scroll(&self.list_scroll);

        let empty = self.rows.is_empty().then(|| {
            let (title, hint) = if searching {
                (
                    format!("Nothing matches “{}”", self.query),
                    "Try part of a name, or a pattern like *-C.TEX",
                )
            } else {
                ("This folder is empty".to_owned(), "Drop files or folders here to add them")
            };
            v_flex()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .gap_1()
                .child(Icon::new(IconName::FolderOpen).large().text_color(theme.muted_foreground))
                .child(div().text_sm().child(title))
                .child(div().text_xs().text_color(theme.muted_foreground).child(hint))
        });

        v_flex()
            .size_full()
            .child(self.render_header(searching, cx))
            .child(
                div()
                    .id("list-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .children(empty)
                    .vertical_scrollbar(&self.list_scroll)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            if !std::mem::take(&mut this.row_pressed) {
                                this.clear_selection(window, cx);
                            }
                        }),
                    )
                    .drag_over::<DraggedEntries>(move |style, dragged, _, cx| {
                        // Rows dragged within this tab drop into the folder they are in only
                        // when they come from a search; tinting the list would promise more.
                        if dragged.source_id == this_tab {
                            style
                        } else {
                            style.bg(cx.theme().drop_target.opacity(0.35))
                        }
                    })
                    .drag_over::<ExternalPaths>(|style, _, _, cx| {
                        style.bg(cx.theme().drop_target.opacity(0.35))
                    })
                    .on_drop(cx.listener({
                        let dir = dir.clone();
                        move |this, dragged: &DraggedEntries, window, cx| {
                            this.drop_entries(dragged, &dir, window, cx)
                        }
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drop_paths(paths, &dir, window, cx)
                    }))
                    .context_menu(move |menu, _, cx| {
                        let Some(view) = view.upgrade() else { return menu };
                        let this = view.read(cx);
                        let any = !this.selected.is_empty();
                        let single = this.selected.len() == 1;
                        let can_export = this.can_export();
                        let opens = this.single_selected().is_some();
                        let can_paste = ArchiveView::can_paste(cx);
                        let can_undo = this.archive.can_undo();
                        let can_redo = this.archive.can_redo();
                        menu.action_context(focus.clone())
                            .when(opens, |menu| {
                                menu.menu_with_icon(
                                    "Open",
                                    IconName::FolderOpen,
                                    Box::new(OpenSelected),
                                )
                            })
                            .menu_with_icon(
                                if any { "Extract…" } else { "Extract all…" },
                                IconName::FolderOutput,
                                Box::new(ExtractSelected),
                            )
                            .when(can_export, |menu| {
                                menu.menu_with_icon(
                                    "Export as PNG…",
                                    IconName::FileImage,
                                    Box::new(ExportPng),
                                )
                                .menu_with_icon(
                                    "Export as TGA…",
                                    IconName::FileImage,
                                    Box::new(ExportTga),
                                )
                            })
                            .separator()
                            .menu_with_disabled("Cut", Box::new(CutEntries), !any)
                            .menu_with_disabled("Copy", Box::new(CopyEntries), !any)
                            .menu_with_disabled("Paste", Box::new(PasteEntries), !can_paste)
                            .separator()
                            .menu_with_disabled("Rename", Box::new(RenameSelected), !single)
                            .menu_with_icon_and_disabled(
                                "Delete",
                                IconName::Trash,
                                Box::new(DeleteSelected),
                                !any,
                            )
                            .separator()
                            .menu_with_icon("New folder", IconName::FolderPlus, Box::new(NewFolder))
                            .menu_with_icon("Add files…", IconName::FilePlus, Box::new(AddFiles))
                            .menu_with_icon(
                                "Add folder…",
                                IconName::FolderInput,
                                Box::new(AddFolder),
                            )
                            .separator()
                            .menu_with_disabled("Undo", Box::new(Undo), !can_undo)
                            .menu_with_disabled("Redo", Box::new(Redo), !can_redo)
                    }),
            )
            .into_any_element()
    }

    fn render_header(&self, searching: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sort = self.sort;
        let cell = |column: Column, label: &'static str, cx: &mut Context<Self>| {
            let active = sort.column == column;
            h_flex()
                .id(label)
                .h_full()
                .px_2()
                .gap_1()
                .when(matches!(column, Column::Size), |this| this.justify_end())
                .hover(|this| this.bg(cx.theme().list_hover))
                .child(label)
                .when(active, |this| {
                    let icon =
                        if sort.descending { IconName::ChevronDown } else { IconName::ChevronUp };
                    this.child(Icon::new(icon).xsmall())
                })
                .on_click(cx.listener(move |this, _, window, cx| this.set_sort(column, window, cx)))
        };
        h_flex()
            .h(px(28.))
            .flex_shrink_0()
            .px_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.list_head)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(cell(Column::Name, "Name", cx).flex_1().min_w_0())
            .when(searching, |this| {
                this.child(cell(Column::Folder, "Folder", cx).w(relative(FOLDER_SHARE)))
            })
            .child(cell(Column::Kind, "Type", cx).w(px(KIND_WIDTH)))
            .child(cell(Column::Size, "Size", cx).w(px(SIZE_WIDTH)))
            // Search results make room for the folder they are in instead: within one archive
            // every file usually carries the same date anyway.
            .when(!searching, |this| {
                this.child(cell(Column::Modified, "Modified", cx).w(px(TIME_WIDTH)))
            })
    }

    fn render_row(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[ix];
        let path: Arc<str> = row.path.clone();
        let selected = self.selected.contains(&path);
        let focused = self.focus.is_focused(window);
        let cursor = focused && self.cursor.as_ref() == Some(&path);
        let renaming = self.rename.as_ref().filter(|rename| rename.path == path);
        let searching = !self.query.is_empty();

        let dragged = DraggedEntries {
            source: cx.entity().downgrade(),
            source_id: cx.entity_id(),
            paths: if selected { self.selection.clone() } else { std::rc::Rc::new([path.clone()]) },
            icon: row.kind.icon(),
            label: if selected && self.selection.len() > 1 {
                format::items(self.selection.len() as u64).into()
            } else {
                row.name.clone().into()
            },
            handed_out: std::rc::Rc::default(),
        };

        let name: AnyElement = match renaming {
            Some(rename) => Input::new(&rename.input).xsmall().into_any_element(),
            None => div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(row.name.clone()))
                .into_any_element(),
        };
        let muted = theme.muted_foreground;
        let row_bg = if selected {
            if focused { theme.list_active } else { theme.list_active.opacity(0.6) }
        } else if ix % 2 == 1 {
            theme.list_even
        } else {
            theme.transparent
        };

        h_flex()
            .id(ElementId::Name(SharedString::from(path.clone())))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_1()
            .text_sm()
            .bg(row_bg)
            .border_1()
            .border_color(if cursor { theme.list_active_border } else { theme.transparent })
            .when(!selected, |this| this.hover(|this| this.bg(theme.list_hover)))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .gap_2()
                    .child(Icon::new(row.kind.icon()).small().text_color(row.kind.color(cx)))
                    .child(name),
            )
            .when(searching, |this| {
                this.child(
                    div()
                        .w(relative(FOLDER_SHARE))
                        .px_2()
                        .text_xs()
                        .text_color(muted)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(row.parent().trim_start_matches('/').to_owned()),
                )
            })
            .child(
                div().w(px(KIND_WIDTH)).px_2().text_xs().text_color(muted).child(row.kind.label()),
            )
            .child(
                div()
                    .w(px(SIZE_WIDTH))
                    .px_2()
                    .flex()
                    .justify_end()
                    .text_xs()
                    .text_color(muted)
                    .child(if row.is_dir {
                        format::items(row.size)
                    } else {
                        format::size(row.size)
                    }),
            )
            .when(!searching, |this| {
                this.child(
                    div()
                        .w(px(TIME_WIDTH))
                        .px_2()
                        .text_xs()
                        .text_color(muted)
                        .child(format::time(row.time)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let path = path.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.row_pressed = true;
                        this.row_mouse_down(&path, event.modifiers, false, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let path = path.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        this.row_mouse_down(&path, event.modifiers, true, window, cx);
                    }
                }),
            )
            .on_click(cx.listener({
                let path = path.clone();
                move |this, event, window, cx| this.row_click(&path, event, window, cx)
            }))
            .on_drag(dragged, |dragged, _, _, cx| dragged.ghost(cx))
            .external_drag_payload(|dragged: &DraggedEntries, _, cx| dragged.external_payload(cx))
            .when(row.is_dir, |this| {
                let dest = path.to_string();
                let drop_bg = theme.drop_target;
                this.drag_over::<DraggedEntries>(move |style, _, _, _| style.bg(drop_bg))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_bg))
                    .on_drop(cx.listener({
                        let dest = dest.clone();
                        move |this, dragged: &DraggedEntries, window, cx| {
                            this.drop_entries(dragged, &dest, window, cx)
                        }
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drop_paths(paths, &dest, window, cx)
                    }))
            })
            .into_any_element()
    }
}
