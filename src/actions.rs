//! Every command with a keyboard shortcut. The same actions back the menus and buttons, so each
//! command behaves the same however it is given.

use std::rc::Rc;

use gpui_kit::Action;
use gpui_kit::App;
use gpui_kit::KeyBinding;
use gpui_kit::KeyBindingContextPredicate;
use gpui_kit::actions;

actions!(
    explorer,
    [
        OpenArchive,
        NewArchive,
        CloseTab,
        NextTab,
        PreviousTab,
        ToggleTheme,
        Quit,
        Save,
        SaveAs,
        ArchiveSettings,
        Undo,
        Redo,
        FocusSearch,
        TogglePreview,
        GoBack,
        GoForward,
        GoUp,
        OpenSelected,
        SelectAll,
        SelectNext,
        SelectPrevious,
        ExtendNext,
        ExtendPrevious,
        SelectFirst,
        SelectLast,
        ExtendFirst,
        ExtendLast,
        PageDown,
        PageUp,
        DeleteSelected,
        RenameSelected,
        NewFolder,
        AddFiles,
        AddFolder,
        ExtractSelected,
        ExtractAll,
        CopyEntries,
        CutEntries,
        PasteEntries,
        Cancel,
    ]
);

pub const WORKSPACE: &str = "Workspace";
pub const ARCHIVE: &str = "ArchiveView";
/// The file list itself, so that keys like Backspace and arrows stay with the search box while
/// it has focus.
pub const FILE_LIST: &str = "FileList";

/// `secondary-` stands for the platform's usual modifier, Cmd on macOS and Ctrl elsewhere, which
/// only a binding loaded against the running platform can resolve.
fn binding(keys: &str, action: impl Action, context: &str, cx: &App) -> KeyBinding {
    let context = KeyBindingContextPredicate::parse(context).ok().map(Rc::new);
    KeyBinding::load(keys, Box::new(action), context, false, None, cx).expect("a valid key binding")
}

pub fn bind(cx: &mut App) {
    let bindings = [
        binding("secondary-o", OpenArchive, WORKSPACE, cx),
        binding("secondary-n", NewArchive, WORKSPACE, cx),
        binding("secondary-w", CloseTab, WORKSPACE, cx),
        binding("ctrl-tab", NextTab, WORKSPACE, cx),
        binding("ctrl-shift-tab", PreviousTab, WORKSPACE, cx),
        binding("secondary-q", Quit, WORKSPACE, cx),
        binding("secondary-s", Save, ARCHIVE, cx),
        binding("secondary-shift-s", SaveAs, ARCHIVE, cx),
        binding("secondary-z", Undo, ARCHIVE, cx),
        binding("secondary-y", Redo, ARCHIVE, cx),
        binding("secondary-shift-z", Redo, ARCHIVE, cx),
        binding("secondary-f", FocusSearch, ARCHIVE, cx),
        binding("alt-p", TogglePreview, ARCHIVE, cx),
        binding("alt-left", GoBack, ARCHIVE, cx),
        binding("alt-right", GoForward, ARCHIVE, cx),
        binding("alt-up", GoUp, ARCHIVE, cx),
        binding("secondary-shift-n", NewFolder, ARCHIVE, cx),
        binding("secondary-e", ExtractSelected, ARCHIVE, cx),
        binding("secondary-shift-e", ExtractAll, ARCHIVE, cx),
        binding("escape", Cancel, ARCHIVE, cx),
        binding("enter", OpenSelected, FILE_LIST, cx),
        binding("backspace", GoUp, FILE_LIST, cx),
        binding("secondary-a", SelectAll, FILE_LIST, cx),
        binding("down", SelectNext, FILE_LIST, cx),
        binding("up", SelectPrevious, FILE_LIST, cx),
        binding("shift-down", ExtendNext, FILE_LIST, cx),
        binding("shift-up", ExtendPrevious, FILE_LIST, cx),
        binding("home", SelectFirst, FILE_LIST, cx),
        binding("end", SelectLast, FILE_LIST, cx),
        binding("shift-home", ExtendFirst, FILE_LIST, cx),
        binding("shift-end", ExtendLast, FILE_LIST, cx),
        binding("pagedown", PageDown, FILE_LIST, cx),
        binding("pageup", PageUp, FILE_LIST, cx),
        binding("delete", DeleteSelected, FILE_LIST, cx),
        binding("f2", RenameSelected, FILE_LIST, cx),
        binding("secondary-c", CopyEntries, FILE_LIST, cx),
        binding("secondary-x", CutEntries, FILE_LIST, cx),
        binding("secondary-v", PasteEntries, FILE_LIST, cx),
    ];
    cx.bind_keys(bindings);
}
