//! One open archive in its tab: where the user is in it, what is selected, and every command on
//! it. The list, the folder tree and the preview each render from this state in their own
//! module.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::ClickEvent;
use gpui_kit::Context;
use gpui_kit::Entity;
use gpui_kit::ExternalPaths;
use gpui_kit::FocusHandle;
use gpui_kit::Focusable;
use gpui_kit::Global;
use gpui_kit::InteractiveElement as _;
use gpui_kit::IntoElement;
use gpui_kit::Modifiers;
use gpui_kit::ParentElement as _;
use gpui_kit::PathPromptOptions;
use gpui_kit::Render;
use gpui_kit::ScrollStrategy;
use gpui_kit::SharedString;
use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::Styled as _;
use gpui_kit::Subscription;
use gpui_kit::Task;
use gpui_kit::UniformListScrollHandle;
use gpui_kit::WeakEntity;
use gpui_kit::Window;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::h_flex;
use gpui_kit::component::h_resizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable_panel;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::px;
use zenkit::vfs::Node;

use crate::actions::*;
use crate::archive;
use crate::archive::Archive;
use crate::archive::Entry;
use crate::archive::Outcome;
use crate::drag::DraggedEntries;
use crate::export;
use crate::export::Picture;
use crate::format;
use crate::kind::FileKind;
use crate::preview::Preview;

/// Search results stop here, so that a one-letter query on a huge archive stays instant.
const SEARCH_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Folder,
    Kind,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

pub struct TreeRow {
    pub path: Arc<str>,
    pub name: SharedString,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
}

pub struct Rename {
    pub path: Arc<str>,
    pub input: Entity<InputState>,
    _subscription: Subscription,
}

/// Entries copied or cut, waiting to be pasted into any tab. They are copies of the nodes, so
/// they paste the same even after their archive changes or closes.
#[derive(Default)]
struct Clipboard {
    nodes: Vec<Node>,
    cut: Option<(WeakEntity<ArchiveView>, Vec<String>)>,
}

impl Global for Clipboard {}

pub enum ArchiveEvent {
    Saved,
    SaveFailed,
}

pub struct ArchiveView {
    pub(crate) archive: Archive,
    pub(crate) focus: FocusHandle,
    pub(crate) dir: Arc<str>,
    back: Vec<Arc<str>>,
    forward: Vec<Arc<str>>,
    pub(crate) rows: Rc<Vec<Entry>>,
    index: HashMap<Arc<str>, usize>,
    pub(crate) sort: Sort,
    pub(crate) selected: HashSet<Arc<str>>,
    /// The selection in list order, shared with every row's drag so that rendering a row does
    /// not copy it.
    pub(crate) selection: Rc<[Arc<str>]>,
    /// Files, folders and bytes selected, counted once per change of the selection.
    pub(crate) selection_totals: (usize, usize, u64),
    anchor: Option<Arc<str>>,
    pub(crate) cursor: Option<Arc<str>>,
    /// A plain press on a row of a larger selection keeps the selection, so that it can be
    /// dragged; the click that follows without a drag then selects that row alone.
    pending_single: Option<Arc<str>>,
    /// Set by a row's press, so that the list's own press handler, which runs after it, knows
    /// not to clear the selection.
    pub(crate) row_pressed: bool,
    pub(crate) search: Entity<InputState>,
    pub(crate) query: String,
    pub(crate) expanded: HashSet<String>,
    pub(crate) tree_rows: Rc<Vec<TreeRow>>,
    /// The archive generation the tree rows were built for.
    tree_generation: Option<u64>,
    /// Whether the search stopped at its limit with more matches left.
    pub(crate) truncated: bool,
    pub(crate) list_scroll: UniformListScrollHandle,
    pub(crate) tree_scroll: UniformListScrollHandle,
    pub(crate) rename: Option<Rename>,
    pub(crate) preview: Preview,
    pub(crate) show_preview: bool,
    pub(crate) busy: Option<SharedString>,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ArchiveView {
    pub fn new(archive: Archive, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search this folder"));
        let subscription = cx.subscribe_in(&search, window, |this, state, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().trim().to_owned();
                this.refresh(window, cx);
                this.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
        });
        let mut view = Self {
            archive,
            focus: cx.focus_handle(),
            dir: "/".into(),
            back: Vec::new(),
            forward: Vec::new(),
            rows: Rc::default(),
            index: HashMap::new(),
            sort: Sort { column: Column::Name, descending: false },
            selected: HashSet::new(),
            selection: Rc::new([]),
            selection_totals: (0, 0, 0),
            anchor: None,
            cursor: None,
            pending_single: None,
            row_pressed: false,
            search,
            query: String::new(),
            expanded: HashSet::new(),
            tree_rows: Rc::default(),
            tree_generation: None,
            truncated: false,
            list_scroll: UniformListScrollHandle::new(),
            tree_scroll: UniformListScrollHandle::new(),
            rename: None,
            preview: Preview::default(),
            show_preview: true,
            busy: None,
            _task: None,
            _subscriptions: vec![subscription],
        };
        // An archive usually holds one folder, like _WORK; open it so the tree shows something.
        if let [only] = view.archive.subdirs("/").as_slice() {
            view.expanded.insert(only.path.to_ascii_lowercase());
        }
        view.refresh(window, cx);
        view
    }

    pub fn archive(&self) -> &Archive {
        &self.archive
    }

    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    // ----- Rows and selection -----

    /// Rebuilds what the list and the tree show from the archive, keeping the selection where its
    /// entries are still listed.
    pub(crate) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.archive.is_dir(&self.dir) {
            let mut dir: &str = &self.dir;
            while !self.archive.is_dir(dir) {
                dir = archive::parent_of(dir);
            }
            self.dir = dir.into();
        }
        let mut rows = if self.query.is_empty() {
            self.archive.list(&self.dir)
        } else {
            self.archive.search(&self.dir, &self.query, SEARCH_LIMIT + 1)
        };
        self.truncated = rows.len() > SEARCH_LIMIT;
        rows.truncate(SEARCH_LIMIT);
        sort_rows(&mut rows, self.sort);
        self.index = rows.iter().enumerate().map(|(ix, row)| (row.path.clone(), ix)).collect();
        self.rows = Rc::new(rows);
        self.selected.retain(|path| self.index.contains_key(path));
        for slot in [&mut self.anchor, &mut self.cursor] {
            if slot.as_ref().is_some_and(|path| !self.index.contains_key(path)) {
                *slot = None;
            }
        }
        self.selection_changed(window, cx);
        if self.tree_generation != Some(self.archive.generation()) {
            self.rebuild_tree();
        }
        cx.notify();
    }

    fn rebuild_tree(&mut self) {
        self.tree_generation = Some(self.archive.generation());
        fn walk(view: &ArchiveView, dir: &str, depth: usize, rows: &mut Vec<TreeRow>) {
            for entry in view.archive.subdirs(dir) {
                let expanded = view.expanded.contains(&entry.path.to_ascii_lowercase());
                let has_children = view.archive.has_subdirs(&entry.path);
                rows.push(TreeRow {
                    name: entry.name.clone().into(),
                    path: entry.path.clone(),
                    depth,
                    has_children,
                    expanded,
                });
                if expanded && has_children {
                    walk(view, &entry.path, depth + 1, rows);
                }
            }
        }
        let mut rows = vec![TreeRow {
            path: "/".into(),
            name: self.archive.title().into(),
            depth: 0,
            has_children: self.archive.has_subdirs("/"),
            expanded: true,
        }];
        walk(self, "/", 1, &mut rows);
        self.tree_rows = Rc::new(rows);
    }

    fn selection_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut selection: Vec<Arc<str>> = self.selected.iter().cloned().collect();
        selection.sort_by_key(|path| self.index.get(path).copied().unwrap_or(usize::MAX));
        self.selection_totals = selection.iter().filter_map(|path| self.index.get(path)).fold(
            (0, 0, 0),
            |(files, dirs, bytes), &ix| match &self.rows[ix] {
                row if row.is_dir => (files, dirs + 1, bytes),
                row => (files + 1, dirs, bytes + row.size),
            },
        );
        self.selection = selection.into();
        self.update_preview(window, cx);
        cx.notify();
    }

    pub(crate) fn single_selected(&self) -> Option<Entry> {
        match &*self.selection {
            [path] => self.index.get(path).map(|&ix| self.rows[ix].clone()),
            _ => None,
        }
    }

    fn selected_paths(&self) -> Vec<String> {
        archive::outermost(&self.selection.iter().map(|path| path.to_string()).collect::<Vec<_>>())
    }

    fn cursor_index(&self) -> Option<usize> {
        self.cursor.as_ref().and_then(|path| self.index.get(path)).copied()
    }

    pub(crate) fn select_only(
        &mut self,
        paths: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected.clear();
        for path in paths {
            if let Some((key, _)) = self.index.get_key_value(path.as_str()) {
                self.selected.insert(key.clone());
            }
        }
        self.cursor = self.selection_in_order().first().cloned();
        self.anchor = self.cursor.clone();
        if let Some(ix) = self.cursor_index() {
            self.list_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.selection_changed(window, cx);
    }

    fn selection_in_order(&self) -> Vec<Arc<str>> {
        let mut paths: Vec<Arc<str>> = self.selected.iter().cloned().collect();
        paths.sort_by_key(|path| self.index.get(path).copied().unwrap_or(usize::MAX));
        paths
    }

    /// Moves the cursor to row `ix`, selecting it alone or, with `extend`, everything from the
    /// anchor to it.
    fn move_cursor(
        &mut self,
        ix: usize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.get(ix) else { return };
        let path = row.path.clone();
        if extend {
            let anchor =
                self.anchor.as_ref().and_then(|a| self.index.get(a)).copied().unwrap_or(ix);
            let (start, end) = (anchor.min(ix), anchor.max(ix));
            self.selected = self.rows[start..=end].iter().map(|row| row.path.clone()).collect();
        } else {
            self.selected = HashSet::from([path.clone()]);
            self.anchor = Some(path.clone());
        }
        self.cursor = Some(path);
        self.list_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        self.selection_changed(window, cx);
    }

    fn step(&mut self, delta: isize, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() || self.rename.is_some() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        let ix = match self.cursor_index() {
            Some(ix) => (ix as isize + delta).clamp(0, last),
            None if delta < 0 => last,
            None => 0,
        };
        self.move_cursor(ix as usize, extend, window, cx);
    }

    pub(crate) fn row_mouse_down(
        &mut self,
        path: &Arc<str>,
        modifiers: Modifiers,
        right: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let Some(&ix) = self.index.get(path) else { return };
        self.pending_single = None;
        if modifiers.shift {
            self.move_cursor(ix, true, window, cx);
        } else if modifiers.secondary(cx) && !right {
            if !self.selected.remove(path) {
                self.selected.insert(path.clone());
            }
            self.anchor = Some(path.clone());
            self.cursor = Some(path.clone());
            self.selection_changed(window, cx);
        } else if self.selected.contains(path) {
            if !right && self.selected.len() > 1 {
                self.pending_single = Some(path.clone());
            }
            self.cursor = Some(path.clone());
            cx.notify();
        } else {
            self.move_cursor(ix, false, window, cx);
        }
    }

    pub(crate) fn row_click(
        &mut self,
        path: &Arc<str>,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(&ix) = self.index.get(path) else { return };
        if event.click_count() >= 2 {
            self.open_row(ix, window, cx);
        } else if self.pending_single.take().as_ref() == Some(path) {
            self.move_cursor(ix, false, window, cx);
        }
    }

    pub(crate) fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.selected.clear();
            self.selection_changed(window, cx);
        }
    }

    pub(crate) fn set_sort(&mut self, column: Column, window: &mut Window, cx: &mut Context<Self>) {
        self.sort =
            Sort { column, descending: self.sort.column == column && !self.sort.descending };
        self.refresh(window, cx);
    }

    // ----- Navigation -----

    pub(crate) fn navigate(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.archive.is_dir(dir) {
            return;
        }
        if !dir.eq_ignore_ascii_case(&self.dir) {
            self.back.push(self.dir.clone());
            self.forward.clear();
        }
        self.go(dir.into(), window, cx);
    }

    fn go(&mut self, dir: Arc<str>, window: &mut Window, cx: &mut Context<Self>) {
        let from = std::mem::replace(&mut self.dir, dir);
        // Reveal the folder in the tree.
        let mut parent = archive::parent_of(&self.dir);
        while parent != "/" {
            self.expanded.insert(parent.to_ascii_lowercase());
            parent = archive::parent_of(parent);
        }
        self.tree_generation = None;
        self.cancel_rename(window, cx);
        if !self.query.is_empty() {
            self.query.clear();
            self.search.update(cx, |search, cx| search.set_value("", window, cx));
        }
        self.selected.clear();
        self.refresh(window, cx);
        // Coming up out of a folder selects it, as Explorer does.
        if archive::parent_of(&from).eq_ignore_ascii_case(&self.dir) {
            self.select_only(&[from.to_string()], window, cx);
        } else {
            self.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        if let Some(ix) =
            self.tree_rows.iter().position(|row| row.path.eq_ignore_ascii_case(&self.dir))
        {
            self.tree_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = self.back.pop() {
            self.forward.push(self.dir.clone());
            self.go(dir, window, cx);
        }
    }

    fn go_forward(&mut self, _: &GoForward, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = self.forward.pop() {
            self.back.push(self.dir.clone());
            self.go(dir, window, cx);
        }
    }

    pub(crate) fn go_up(&mut self, _: &GoUp, window: &mut Window, cx: &mut Context<Self>) {
        if &*self.dir != "/" && self.rename.is_none() {
            let parent = archive::parent_of(&self.dir).to_owned();
            self.navigate(&parent, window, cx);
        }
    }

    pub(crate) fn toggle_expanded(&mut self, path: &str, cx: &mut Context<Self>) {
        let key = path.to_ascii_lowercase();
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        self.rebuild_tree();
        cx.notify();
    }

    fn open_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).cloned() else { return };
        if row.is_dir {
            self.navigate(&row.path, window, cx);
        } else {
            self.open_with_system(&row, window, cx);
        }
    }

    /// Opens a file in the program the system has for it, from a temporary copy.
    fn open_with_system(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.archive.take_copies(&[entry.path.to_string()]);
        let opened =
            crate::drag::new_drag_dir().map_err(|error| error.to_string()).and_then(|dir| {
                archive::extract(&nodes, &dir, &Default::default())
                    .map_err(|e| e.report().to_string())
            });
        match opened {
            Ok(paths) => paths.iter().for_each(|path| cx.open_with_system(path)),
            Err(reason) => window.push_notification(Notification::error(reason), cx),
        }
    }

    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        // The rename box lets Enter through once it has taken it.
        if self.rename.is_some() {
            return;
        }
        if let Some(ix) = self.cursor_index().filter(|_| self.selected.len() <= 1) {
            self.open_row(ix, window, cx);
        }
    }

    // ----- Editing -----

    /// Runs an edit, then selects what it touched and tells the user about anything it could not
    /// do.
    fn apply(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Archive) -> zenkit::Result<Outcome>,
    ) {
        match edit(&mut self.archive) {
            Ok(outcome) => {
                self.refresh(window, cx);
                self.select_only(&outcome.paths, window, cx);
                if outcome.replaced > 0 {
                    let note =
                        format!("Replaced {}", format::count(outcome.replaced, "entry", "entries"));
                    window.push_notification(Notification::info(note), cx);
                }
            }
            Err(error) => {
                window.push_notification(Notification::error(error.report().to_string()), cx)
            }
        }
    }

    pub(crate) fn drop_entries(
        &mut self,
        dragged: &DraggedEntries,
        dest: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dest = dest.to_owned();
        let copy = window.modifiers().secondary(cx) || window.modifiers().alt;
        if dragged.source_id == cx.entity_id() && !copy {
            let paths = archive::outermost(&dragged.paths());
            self.apply(window, cx, |archive| archive.move_into(&paths, &dest));
        } else {
            let nodes = if dragged.source_id == cx.entity_id() {
                self.archive.take_copies(&dragged.paths())
            } else {
                dragged.nodes(cx)
            };
            self.apply(window, cx, |archive| archive.insert(nodes, &dest));
        }
        window.focus(&self.focus, cx);
    }

    pub(crate) fn drop_paths(
        &mut self,
        paths: &ExternalPaths,
        dest: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = paths.paths().to_vec();
        // Our own drag, back over the window it left: nothing to add where it came from.
        if paths.iter().all(|path| crate::drag::is_drag_file(path))
            && paths.iter().all(|path| self.came_from(path, dest))
        {
            return;
        }
        self.add_paths(paths, dest.to_owned(), window, cx);
    }

    fn came_from(&self, path: &std::path::Path, dest: &str) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.archive.exists(&archive::join(dest, &name))
    }

    /// Reads host files and folders on a background thread, then adds them to `dest`.
    pub(crate) fn add_paths(
        &mut self,
        paths: Vec<PathBuf>,
        dest: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        self.busy = Some(format!("Adding {}…", format::count(paths.len(), "item", "items")).into());
        cx.notify();
        let reading = cx.background_spawn(async move { archive::import(&paths) });
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let nodes = reading.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match nodes {
                    Ok(nodes) => this.apply(window, cx, |archive| archive.insert(nodes, &dest)),
                    Err(error) => {
                        window
                            .push_notification(Notification::error(error.report().to_string()), cx);
                        cx.notify();
                    }
                }
            });
        }));
    }

    fn prompt_add(&mut self, directories: bool, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: true,
            prompt: Some("Add".into()),
        });
        let dest = self.dir.to_string();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ =
                    this.update_in(cx, |this, window, cx| this.add_paths(paths, dest, window, cx));
            }
        })
        .detach();
    }

    fn add_files(&mut self, _: &AddFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_add(false, window, cx);
    }

    fn add_folder(&mut self, _: &AddFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_add(true, window, cx);
    }

    fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.is_empty() {
            self.query.clear();
            self.search.update(cx, |search, cx| search.set_value("", window, cx));
        }
        match self.archive.new_folder(&self.dir) {
            Ok(path) => {
                self.refresh(window, cx);
                self.select_only(&[path], window, cx);
                self.start_rename(window, cx);
            }
            Err(error) => {
                window.push_notification(Notification::error(error.report().to_string()), cx)
            }
        }
    }

    fn delete_selected(&mut self, _: &DeleteSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        let paths = self.selected_paths();
        let Some(first) = paths.first() else { return };
        let (title, description) = match paths.len() {
            1 => (
                format!("Delete “{}”?", first.rsplit('/').next().unwrap_or(first)),
                "Undo brings it back while the archive is open.",
            ),
            n => (format!("Delete {n} items?"), "Undo brings them back while the archive is open."),
        };
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let alert = alert.bg(crate::appearance::dialog_fill(cx));
            let view = view.clone();
            let paths = paths.clone();
            alert
                .title(title.clone())
                .description(description)
                .confirm()
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.archive.delete(&paths);
                        this.refresh(window, cx);
                        window.focus(&this.focus, cx);
                    });
                    true
                })
        });
    }

    pub(crate) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.single_selected() else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(entry.name.to_string()));
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_rename(window, cx),
                _ => {}
            });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            // Select the name without its extension, as Explorer does.
            let stem = entry
                .name
                .rfind('.')
                .filter(|&dot| dot > 0 && !entry.is_dir)
                .unwrap_or(entry.name.len());
            input.set_selected_range(0..stem, cx);
        });
        self.rename = Some(Rename { path: entry.path.clone(), input, _subscription: subscription });
        cx.notify();
    }

    fn rename_selected(&mut self, _: &RenameSelected, window: &mut Window, cx: &mut Context<Self>) {
        self.start_rename(window, cx);
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else { return };
        let name = rename.input.read(cx).value().to_string();
        window.focus(&self.focus, cx);
        match self.archive.rename(&rename.path, &name) {
            Ok(path) => {
                self.refresh(window, cx);
                self.select_only(&[path], window, cx);
            }
            Err(error) => {
                window.push_notification(Notification::error(error.report().to_string()), cx);
                cx.notify();
            }
        }
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            self.cancel_rename(window, cx);
        } else if !self.query.is_empty() {
            self.search.update(cx, |search, cx| search.set_value("", window, cx));
            self.query.clear();
            self.refresh(window, cx);
            window.focus(&self.focus, cx);
        } else {
            self.clear_selection(window, cx);
        }
    }

    fn copy_entries(&mut self, _: &CopyEntries, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        let nodes = self.archive.take_copies(&paths);
        cx.set_global(Clipboard { nodes, cut: None });
    }

    fn cut_entries(&mut self, _: &CutEntries, _: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        let nodes = self.archive.take_copies(&paths);
        let source = cx.entity().downgrade();
        cx.set_global(Clipboard { nodes, cut: Some((source, paths)) });
    }

    pub(crate) fn can_paste(cx: &gpui_kit::App) -> bool {
        cx.try_global::<Clipboard>().is_some_and(|clipboard| !clipboard.nodes.is_empty())
    }

    fn paste_entries(&mut self, _: &PasteEntries, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clipboard) = cx.try_global::<Clipboard>() else { return };
        let nodes = clipboard.nodes.clone();
        let cut = clipboard.cut.clone();
        let dest = self.dir.to_string();
        if let Some((source, paths)) = &cut
            && source.entity_id() == cx.entity_id()
        {
            cx.set_global(Clipboard::default());
            self.apply(window, cx, |archive| archive.move_into(paths, &dest));
            return;
        }
        let generation = self.archive.generation();
        self.apply(window, cx, |archive| archive.insert(nodes, &dest));
        // A cut leaves its archive only once the entries have arrived here.
        if let Some((source, paths)) = cut
            && self.archive.generation() != generation
        {
            cx.set_global(Clipboard::default());
            let _ = source.update(cx, |source, cx| {
                source.archive.delete(&paths);
                source.refresh(window, cx);
            });
        }
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.archive.undo() {
            self.refresh(window, cx);
        }
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.archive.redo() {
            self.refresh(window, cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        self.selected = self.rows.iter().map(|row| row.path.clone()).collect();
        self.selection_changed(window, cx);
    }

    // ----- Extracting and saving -----

    fn extract(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.archive.take_copies(&paths);
        if nodes.is_empty() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Extract here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(dirs))) = prompt.await else { return };
            let Some(dest) = dirs.into_iter().next() else { return };
            let count = nodes.len();
            let _ = this.update(cx, |this, cx| {
                this.busy =
                    Some(format!("Extracting {}…", format::count(count, "item", "items")).into());
                cx.notify();
            });
            let target = dest.clone();
            let result = cx
                .background_spawn(
                    async move { archive::extract(&nodes, &target, &Default::default()) },
                )
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                let note = match result {
                    Ok(_) => Notification::success(format!("Extracted to {}", dest.display())),
                    Err(error) => Notification::error(error.report().to_string()),
                };
                window.push_notification(note, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn extract_selected(
        &mut self,
        _: &ExtractSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            self.extract_all(&ExtractAll, window, cx);
        } else {
            self.extract(paths, window, cx);
        }
    }

    fn extract_all(&mut self, _: &ExtractAll, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.archive.list("/").iter().map(|e| e.path.to_string()).collect();
        self.extract(paths, window, cx);
    }

    /// Whether the selection may hold textures: a texture, or a folder, which is only searched
    /// when exporting.
    pub(crate) fn can_export(&self) -> bool {
        self.selection.iter().any(|path| {
            self.index.get(path).is_some_and(|&ix| {
                let entry = &self.rows[ix];
                entry.is_dir || entry.kind == FileKind::Texture
            })
        })
    }

    fn export_png(&mut self, _: &ExportPng, window: &mut Window, cx: &mut Context<Self>) {
        self.export(Picture::Png, window, cx);
    }

    fn export_tga(&mut self, _: &ExportTga, window: &mut Window, cx: &mut Context<Self>) {
        self.export(Picture::Tga, window, cx);
    }

    /// Exports the selected textures, and those in the selected folders, as pictures.
    fn export(&mut self, picture: Picture, window: &mut Window, cx: &mut Context<Self>) {
        let nodes = self.archive.take_copies(&self.selected_paths());
        let count = export::textures(&nodes, picture).len();
        if count == 0 {
            window.push_notification(Notification::info("There are no textures to export."), cx);
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Export here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(dirs))) = prompt.await else { return };
            let Some(dest) = dirs.into_iter().next() else { return };
            let textures = format::count(count, "texture", "textures");
            let _ = this.update(cx, |this, cx| {
                this.busy = Some(format!("Exporting {textures} as {}…", picture.label()).into());
                cx.notify();
            });
            let target = dest.clone();
            let exported =
                cx.background_spawn(async move { export::export(&nodes, &target, picture) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                let note = match exported.failed.first() {
                    None => {
                        Notification::success(format!("Exported {textures} to {}", dest.display()))
                    }
                    Some((path, reason)) => {
                        let more = exported.failed.len() - 1;
                        let more =
                            if more > 0 { format!(", and {more} more") } else { String::new() };
                        Notification::error(format!(
                            "Exported {} of {textures} to {}. {path}: {reason}{more}",
                            exported.written,
                            dest.display()
                        ))
                    }
                };
                window.push_notification(note, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        match self.archive.path().map(ToOwned::to_owned) {
            Some(path) => self.save_to(path, window, cx),
            None => self.save_as(&SaveAs, window, cx),
        }
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        let dir =
            self.archive.path().and_then(|p| p.parent()).map(ToOwned::to_owned).unwrap_or_default();
        let name = self.archive.title();
        let prompt = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let chosen = prompt.await.ok().and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| match chosen {
                Some(path) => this.save_to(path, window, cx),
                None => cx.emit(ArchiveEvent::SaveFailed),
            });
        })
        .detach();
    }

    fn save_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let stats = self.archive.stats();
        if stats.files == 0 {
            let note = "Nothing to save: a VDF needs at least one file";
            window.push_notification(Notification::warning(note), cx);
            cx.emit(ArchiveEvent::SaveFailed);
            return;
        }
        let job = self.archive.save_job(path);
        let generation = self.archive.generation();
        self.busy = Some("Saving…".into());
        cx.notify();
        let saving = cx.background_spawn(async move { job.run() });
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = saving.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(path) => {
                        if this.archive.generation() == generation {
                            this.archive.saved(path.clone());
                        }
                        let mut note =
                            format!("Saved {}", format::count(stats.files, "file", "files"));
                        if stats.empty_dirs > 0 {
                            note += &format!(
                                "; left out {}",
                                format::count(stats.empty_dirs, "empty folder", "empty folders")
                            );
                        }
                        window.push_notification(Notification::success(note), cx);
                        this.rebuild_tree();
                        cx.emit(ArchiveEvent::Saved);
                    }
                    Err(error) => {
                        window
                            .push_notification(Notification::error(error.report().to_string()), cx);
                        cx.emit(ArchiveEvent::SaveFailed);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn archive_settings(
        &mut self,
        _: &ArchiveSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::settings::open(cx.entity(), window, cx);
    }

    pub(crate) fn set_options(
        &mut self,
        options: zenkit::vfs::DiskOptions,
        cx: &mut Context<Self>,
    ) {
        self.archive.set_options(options);
        cx.notify();
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| {
            search.focus(window, cx);
            search.select_all(window, cx);
        });
    }

    fn toggle_preview(&mut self, _: &TogglePreview, window: &mut Window, cx: &mut Context<Self>) {
        self.show_preview = !self.show_preview;
        if self.show_preview {
            self.preview = Preview::default();
            self.update_preview(window, cx);
        } else {
            self.clear_preview(window, cx);
        }
        cx.notify();
    }

    /// A drag of rows that leaves the window becomes the system's drag of extracted files. GPUI
    /// does this itself where its backend can; on Windows the explorer starts the shell's.
    #[cfg(windows)]
    fn drag_moved(
        &mut self,
        event: &gpui_kit::DragMoveEvent<DraggedEntries>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (position, size) = (event.event.position, window.viewport_size());
        let inside = position.x >= px(0.)
            && position.y >= px(0.)
            && position.x < size.width
            && position.y < size.height;
        let dragged = event.drag(cx);
        if inside || dragged.handed_out.replace(true) {
            return;
        }
        // The drag started here, and this view is being updated while this runs: reading it
        // through its entity, as the dragged entries otherwise do, would panic.
        let nodes = if dragged.source_id == cx.entity_id() {
            self.archive.take_copies(&dragged.paths())
        } else {
            dragged.nodes(cx)
        };
        let payload = crate::drag::external_payload(&nodes);
        // The drag ends once this mouse move has been dispatched, not while it is: the listeners
        // after this one, and GPUI Fast's bookkeeping, still take the drag to be there.
        cx.defer_in(window, move |_, window, cx| {
            cx.stop_active_drag(window);
            match payload {
                Some(gpui_kit::ExternalDragPayload::Files(files)) => {
                    crate::drag::windows::start(
                        files.entries().iter().map(|(path, _)| path.clone()).collect(),
                    );
                }
                None => {
                    let note = "Too much to drag out at once; use Extract instead";
                    window.push_notification(Notification::warning(note), cx);
                }
            }
        });
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let crumbs = breadcrumbs(&self.dir, &self.archive.title());
        let view = cx.entity().downgrade();
        h_flex()
            .h(px(44.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("back")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .disabled(!self.can_go_back())
                    .tooltip_with_action("Back", &GoBack, Some(ARCHIVE))
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(&GoBack, window, cx))),
            )
            .child(
                Button::new("forward")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowRight)
                    .disabled(!self.can_go_forward())
                    .tooltip_with_action("Forward", &GoForward, Some(ARCHIVE))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.go_forward(&GoForward, window, cx)),
                    ),
            )
            .child(
                Button::new("up")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowUp)
                    .disabled(&*self.dir == "/")
                    .tooltip_with_action("Up", &GoUp, Some(ARCHIVE))
                    .on_click(cx.listener(|this, _, window, cx| this.go_up(&GoUp, window, cx))),
            )
            .child(
                h_flex()
                    .id("breadcrumbs")
                    .flex_1()
                    .min_w_0()
                    .h(px(30.))
                    .mx_1()
                    .px_1()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .overflow_x_scroll()
                    .children(crumbs.into_iter().enumerate().map(|(ix, (label, path))| {
                        let view = view.clone();
                        let last = path.eq_ignore_ascii_case(&self.dir);
                        h_flex()
                            .flex_shrink_0()
                            .when(ix > 0, |this| {
                                this.child(
                                    gpui_kit::component::Icon::new(IconName::ChevronRight)
                                        .xsmall()
                                        .text_color(theme.muted_foreground),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("crumb-{path}")))
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(theme.radius)
                                    .text_sm()
                                    .when(last, |this| {
                                        this.font_weight(gpui_kit::FontWeight::MEDIUM)
                                    })
                                    .when(!last, |this| this.text_color(theme.muted_foreground))
                                    .hover(|this| this.bg(theme.secondary_hover))
                                    .child(label)
                                    .drag_over::<DraggedEntries>(|style, _, _, cx| {
                                        style.bg(cx.theme().drop_target)
                                    })
                                    .on_drop({
                                        let view = view.clone();
                                        let path = path.clone();
                                        move |dragged: &DraggedEntries, window, cx| {
                                            let _ = view.update(cx, |this, cx| {
                                                this.drop_entries(dragged, &path, window, cx)
                                            });
                                        }
                                    })
                                    .on_click(move |_, window, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.navigate(&path, window, cx)
                                        });
                                    }),
                            )
                    })),
            )
            .child(
                Input::new(&self.search)
                    .small()
                    .w(px(220.))
                    .prefix(
                        gpui_kit::component::Icon::new(IconName::Search)
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .cleanable(true),
            )
            .child(div().w(px(6.)))
            .child(
                Button::new("new-folder")
                    .ghost()
                    .small()
                    .icon(IconName::FolderPlus)
                    .tooltip_with_action("New folder", &NewFolder, Some(ARCHIVE))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.new_folder(&NewFolder, window, cx)),
                    ),
            )
            .child(
                Button::new("add-files")
                    .ghost()
                    .small()
                    .icon(IconName::FilePlus)
                    .tooltip("Add files…")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.add_files(&AddFiles, window, cx)),
                    ),
            )
            .child(
                Button::new("extract")
                    .ghost()
                    .small()
                    .icon(IconName::FolderOutput)
                    .tooltip_with_action(
                        if self.selected.is_empty() {
                            "Extract all…"
                        } else {
                            "Extract selected…"
                        },
                        &ExtractSelected,
                        Some(ARCHIVE),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.extract_selected(&ExtractSelected, window, cx)
                    })),
            )
            .child(
                Button::new("delete")
                    .ghost()
                    .small()
                    .icon(IconName::Trash)
                    .disabled(self.selected.is_empty())
                    .tooltip("Delete")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.delete_selected(&DeleteSelected, window, cx)
                    })),
            )
            .child(
                Button::new("preview")
                    .ghost()
                    .small()
                    .icon(IconName::PanelRight)
                    .selected(self.show_preview)
                    .tooltip_with_action("Preview", &TogglePreview, Some(ARCHIVE))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_preview(&TogglePreview, window, cx)
                    })),
            )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let options = self.archive.options();
        let mut left = if self.query.is_empty() {
            format::items(self.rows.len() as u64)
        } else if self.truncated {
            format!("First {} matches", format::grouped(SEARCH_LIMIT as u64))
        } else {
            format::count(self.rows.len(), "match", "matches")
        };
        if !self.selected.is_empty() {
            let bytes = self.selection_totals.2;
            left += &format!(" · {} selected", self.selected.len());
            if bytes > 0 {
                left += &format!(" ({})", format::size(bytes));
            }
        }
        let mut right = vec![crate::preview::game_name(options.game).to_owned()];
        if options.compressed {
            right.push("Union compressed".into());
        }
        if self.archive.is_dirty() {
            right.push("Modified".into());
        }
        h_flex()
            .h(px(26.))
            .px_3()
            .gap_3()
            .justify_between()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.status_bar)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(left)
            .child(
                h_flex()
                    .gap_3()
                    .when_some(self.busy.clone(), |this, busy| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .text_color(theme.foreground)
                                .child(gpui_kit::component::spinner::Spinner::new().xsmall())
                                .child(busy),
                        )
                    })
                    .child(right.join(" · ")),
            )
    }
}

impl gpui_kit::EventEmitter<ArchiveEvent> for ArchiveView {}

impl Focusable for ArchiveView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ArchiveView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = v_flex().id("archive-view");
        #[cfg(windows)]
        let root = root.on_drag_move::<DraggedEntries>(cx.listener(Self::drag_moved));
        root.key_context(ARCHIVE)
            .size_full()
            .bg(theme.background)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::archive_settings))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_up))
            .on_action(cx.listener(Self::new_folder))
            .on_action(cx.listener(Self::add_files))
            .on_action(cx.listener(Self::add_folder))
            .on_action(cx.listener(Self::extract_selected))
            .on_action(cx.listener(Self::extract_all))
            .on_action(cx.listener(Self::export_png))
            .on_action(cx.listener(Self::export_tga))
            .on_action(cx.listener(Self::cancel))
            .child(self.render_toolbar(cx))
            .child(
                h_resizable("archive-split")
                    .child(
                        resizable_panel()
                            .size(px(240.))
                            .size_range(px(140.)..px(480.))
                            .child(self.render_tree(cx)),
                    )
                    .child(
                        resizable_panel().child(
                            div()
                                .id("file-list")
                                .key_context(FILE_LIST)
                                .track_focus(&self.focus)
                                .size_full()
                                .on_action(cx.listener(Self::open_selected))
                                .on_action(cx.listener(Self::select_all))
                                .on_action(cx.listener(|this, _: &SelectNext, window, cx| {
                                    this.step(1, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| {
                                    this.step(-1, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &ExtendNext, window, cx| {
                                    this.step(1, true, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &ExtendPrevious, window, cx| {
                                    this.step(-1, true, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &SelectFirst, window, cx| {
                                    this.step(isize::MIN / 2, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &SelectLast, window, cx| {
                                    this.step(isize::MAX / 2, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &ExtendFirst, window, cx| {
                                    this.step(isize::MIN / 2, true, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &ExtendLast, window, cx| {
                                    this.step(isize::MAX / 2, true, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &PageDown, window, cx| {
                                    this.step(20, false, window, cx)
                                }))
                                .on_action(cx.listener(|this, _: &PageUp, window, cx| {
                                    this.step(-20, false, window, cx)
                                }))
                                .on_action(cx.listener(Self::delete_selected))
                                .on_action(cx.listener(Self::rename_selected))
                                .on_action(cx.listener(Self::copy_entries))
                                .on_action(cx.listener(Self::cut_entries))
                                .on_action(cx.listener(Self::paste_entries))
                                .child(self.render_list(cx)),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .visible(self.show_preview)
                            .size(px(360.))
                            .size_range(px(220.)..px(720.))
                            .child(self.render_preview(cx)),
                    ),
            )
            .child(self.render_status(cx))
    }
}

/// `(label, path)` for the root and every folder down to `dir`.
fn breadcrumbs(dir: &str, title: &str) -> Vec<(SharedString, String)> {
    let mut crumbs = vec![(SharedString::from(title.to_owned()), "/".to_owned())];
    let mut path = String::new();
    for name in dir.split('/').filter(|name| !name.is_empty()) {
        path.push('/');
        path.push_str(name);
        crumbs.push((name.to_owned().into(), path.clone()));
    }
    crumbs
}

fn sort_rows(rows: &mut [Entry], sort: Sort) {
    rows.sort_by(|a, b| {
        let by_name = || a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase());
        let order = match sort.column {
            Column::Name => by_name(),
            Column::Folder => a
                .parent()
                .to_ascii_lowercase()
                .cmp(&b.parent().to_ascii_lowercase())
                .then_with(by_name),
            Column::Kind => a.kind.label().cmp(b.kind.label()).then_with(by_name),
            Column::Size => a.size.cmp(&b.size).then_with(by_name),
            Column::Modified => a.time.cmp(&b.time).then_with(by_name),
        };
        let order = if sort.descending { order.reverse() } else { order };
        // Folders always come first, as in Explorer.
        b.is_dir.cmp(&a.is_dir).then(order)
    });
}
