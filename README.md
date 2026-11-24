# vdfrs

A file explorer for the ZenGin's `.vdf` and `.mod` archives, built on
[zenkit-rs](https://github.com/Zira3l137/zenkit-rs) and [GPUI Kit](https://gpui-kit.com). Open an archive and work in it as in a folder: browse, search,
preview, drag files in and out, rename, delete, and save it back as the game reads it.

```sh
cargo run --release -- Data/Anims.vdf Data/ModVDF/MyMod.mod
```

It needs a recent stable Rust, and on Linux the packages GPUI builds against
(`libxkbcommon-dev`, `libwayland-dev`, `libxcb1-dev`, `libvulkan-dev`, `libfontconfig-dev`).

## What it does

- **Tabs.** Each archive opens in a tab of its own, read and parsed on a background thread. Drop
  `.vdf` or `.mod` files on the window to open them; drop anything else on the start page to pack
  it into a new archive.
- **Browsing.** A folder tree, a list that draws only the rows on screen, a breadcrumb bar whose
  parts are drop targets too, back, forward and up. The list sorts by name, type, size or date,
  folders first.
- **Search.** Searches the folder and everything below it as you type: part of a name, or a
  pattern like `*-C.TEX`. Stops at 20,000 matches.
- **Preview.** Textures (`.TEX`) as pictures, decoded by the library, with their size, format and
  mipmaps; scripts, model scripts and other text as text; anything else as a hex dump. Selecting a
  folder shows what is in it; selecting nothing shows the archive's header and totals.
- **Drag and drop.**
  - Rows onto folders, the tree or the breadcrumbs move them; hold Ctrl (Cmd) or Alt to copy.
  - Rows onto another archive's tab copy them into the folder open there.
  - Files and folders from the system are added where they are dropped.
  - Rows dragged out of the window become files wherever they are dropped: the explorer extracts
    them to a temporary folder as the pointer leaves the window (up to 512 MB at once).
- **Editing.** New folders, inline rename, delete, cut, copy and paste between any archives, and
  undo and redo for all of them.
- **Saving.** Writes the archive back with the header it had: the game's signature, the comment,
  and Union's compression. Archive settings change those. A save goes to a file beside the
  archive that then takes its place, so a failed save leaves the old archive whole.
- **Extracting** the selection or the whole archive to a folder, in parallel.
- Opening any file in the program the system has for it, from a temporary copy.
- Light and dark themes.

## Keys

| Keys                                | Does                                    |
|-------------------------------------|-----------------------------------------|
| Ctrl+O / Ctrl+N / Ctrl+W            | Open, new archive, close tab            |
| Ctrl+Tab / Ctrl+Shift+Tab           | Next and previous tab                   |
| Ctrl+S / Ctrl+Shift+S               | Save, save as                           |
| Ctrl+F                              | Search                                  |
| Enter / Backspace                   | Open the selection / go up              |
| Alt+Left / Alt+Right / Alt+Up       | Back, forward, up                       |
| Arrows, Home, End, Page Up/Down     | Move the selection; with Shift, extend  |
| Ctrl+A                              | Select all                              |
| F2 / Delete                         | Rename, delete                          |
| Ctrl+X / Ctrl+C / Ctrl+V            | Cut, copy, paste                        |
| Ctrl+Z / Ctrl+Y                     | Undo, redo                              |
| Ctrl+Shift+N                        | New folder                              |
| Ctrl+E / Ctrl+Shift+E               | Extract the selection, extract all      |
| Alt+P                               | Show or hide the preview                |
| Esc                                 | Cancel a rename, clear the search, then the selection |

On macOS, Cmd takes the place of Ctrl.

## What the format allows

A VDF stores names in upper case, in Latin-1, at most 64 characters, and cannot store a folder
with no files in it. The explorer refuses names it could not save, and says how many empty folders
a save left out.

## Limits

- Dragging out of the window works on Windows and on Linux under Wayland. GPUI has no outbound
  drag on X11, so there, use Extract.
- The file dialogs on Linux go through the desktop portal (`xdg-desktop-portal`).
