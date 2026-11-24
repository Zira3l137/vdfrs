//! vdfrs in Windows Explorer: a "vdfrs" submenu in the context menu of archives, files and folders,
//! the archive icon, and optionally opening archives with a double click.
//!
//! Everything is a registry entry under `HKEY_CURRENT_USER\Software\Classes`, which Windows reads
//! as part of `HKEY_CLASSES_ROOT`. So it needs no administrator rights, applies to this user only,
//! and is gone without a trace once its keys are deleted. The submenus are Windows 7's static
//! cascading menus: a verb whose `ExtendedSubCommandsKey` names a key holding the verbs inside it.
//!
//! What goes where is plain data, built by [`entries`], so that it can be tested anywhere; the
//! Windows part only writes it.

// Elsewhere only the tests read the layout.
#![cfg_attr(not(windows), allow(dead_code))]

/// The file type of `.vdf` and `.mod` files once they are associated with vdfrs.
pub const PROG_ID: &str = "vdfrs.archive";
/// The keys holding the verbs of the two submenus.
const ARCHIVE_MENU: &str = "vdfrs.menu.archive";
const ITEMS_MENU: &str = "vdfrs.menu.items";
const EXTENSIONS: [&str; 2] = [".vdf", ".mod"];
/// The submenu for any file hides on archives, which have their own.
const NOT_ARCHIVES: &str = "NOT System.FileExtension:=.vdf AND NOT System.FileExtension:=.mod";

/// One registry key under `Software\Classes`, and the string values to set in it. The value
/// named `""` is the key's default value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub values: Vec<(&'static str, String)>,
}

fn entry(key: impl Into<String>, values: &[(&'static str, &str)]) -> Entry {
    Entry { key: key.into(), values: values.iter().map(|&(n, v)| (n, v.to_owned())).collect() }
}

/// Everything installing writes, for the program at `exe`. `associate` also makes vdfrs what
/// opens `.vdf` and `.mod` files.
pub fn entries(exe: &str, associate: bool) -> Vec<Entry> {
    let icon = format!("\"{exe}\",0");
    let open = format!("\"{exe}\" \"%1\"");
    let run = |args: &str| format!("\"{exe}\" {args} \"%1\"");
    let mut entries = vec![
        entry(PROG_ID, &[("", "ZenGin archive"), ("FriendlyTypeName", "ZenGin archive")]),
        entry(format!(r"{PROG_ID}\DefaultIcon"), &[("", &icon)]),
        entry(format!(r"{PROG_ID}\shell"), &[("", "open")]),
        entry(format!(r"{PROG_ID}\shell\open"), &[("Icon", &icon)]),
        entry(format!(r"{PROG_ID}\shell\open\command"), &[("", &open)]),
        // Lists vdfrs in "Open with" for both extensions, whatever opens them by default.
        entry(r"Applications\vdfrs.exe", &[("FriendlyAppName", "vdfrs")]),
        entry(r"Applications\vdfrs.exe\DefaultIcon", &[("", &icon)]),
        entry(r"Applications\vdfrs.exe\SupportedTypes", &[(".vdf", ""), (".mod", "")]),
        entry(r"Applications\vdfrs.exe\shell\open\command", &[("", &open)]),
    ];
    for ext in EXTENSIONS {
        entries.push(entry(format!(r"{ext}\OpenWithProgids"), &[(PROG_ID, "")]));
        if associate {
            entries.push(entry(ext, &[("", PROG_ID)]));
        }
        entries.push(submenu(
            format!(r"SystemFileAssociations\{ext}\shell\vdfrs"),
            &icon,
            ARCHIVE_MENU,
            None,
        ));
    }
    entries.push(submenu(r"*\shell\vdfrs".into(), &icon, ITEMS_MENU, Some(NOT_ARCHIVES)));
    entries.push(submenu(r"Directory\shell\vdfrs".into(), &icon, ITEMS_MENU, None));

    // Verbs are listed in the order of their key names, hence the numbers.
    let verbs = [
        (ARCHIVE_MENU, "1open", "Open", "Player", run("--gather")),
        (ARCHIVE_MENU, "2pick", "Extract files…", "Player", run("extract --pick --gather")),
        (ARCHIVE_MENU, "3here", "Extract here", "Player", run("extract --here --gather")),
        (ARCHIVE_MENU, "4folder", "Extract to own folder", "Player", run("extract --gather")),
        (ARCHIVE_MENU, "5new", "Add to new archive…", "Player", run("new --gather")),
        (ITEMS_MENU, "1new", "Add to new archive…", "Player", run("new --gather")),
        (ITEMS_MENU, "2pack", "Pack into .vdf", "Player", run("pack --gather")),
    ];
    for (menu, name, label, model, command) in verbs {
        let verb = format!(r"{menu}\shell\{name}");
        // Player keeps the verb for selections of up to a hundred items; with Document,
        // Explorer drops it past fifteen.
        entries.push(entry(&verb, &[("MUIVerb", label), ("MultiSelectModel", model)]));
        entries.push(entry(format!(r"{verb}\command"), &[("", &command)]));
    }
    entries
}

fn submenu(key: String, icon: &str, menu: &str, applies_to: Option<&str>) -> Entry {
    let mut values = vec![("MUIVerb", "vdfrs".to_owned()), ("Icon", icon.to_owned())];
    values.push(("ExtendedSubCommandsKey", menu.to_owned()));
    if let Some(query) = applies_to {
        values.push(("AppliesTo", query.to_owned()));
    }
    Entry { key, values }
}

/// The keys that belong to vdfrs alone, deleted whole on uninstalling. The extensions' own keys
/// are shared with other programs: only vdfrs's values are taken out of those.
pub fn owned_keys() -> Vec<String> {
    let mut keys = vec![
        PROG_ID.to_owned(),
        ARCHIVE_MENU.to_owned(),
        ITEMS_MENU.to_owned(),
        r"Applications\vdfrs.exe".to_owned(),
        r"*\shell\vdfrs".to_owned(),
        r"Directory\shell\vdfrs".to_owned(),
    ];
    keys.extend(EXTENSIONS.map(|ext| format!(r"SystemFileAssociations\{ext}\shell\vdfrs")));
    keys
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    /// Installed for the program at this path, opening archives on a double click or not.
    Installed {
        exe: String,
        associated: bool,
    },
}

impl Status {
    /// Installed, and for the program that is running.
    pub fn is_current(&self) -> bool {
        let Self::Installed { exe, .. } = self else { return false };
        std::env::current_exe().is_ok_and(|current| current.to_string_lossy() == exe.as_str())
    }
}

#[cfg(windows)]
pub use self::windows::install;
#[cfg(windows)]
pub use self::windows::status;
#[cfg(windows)]
pub use self::windows::uninstall;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use std::io;

    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    use super::*;

    fn classes() -> io::Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Software\Classes").map(|(key, _)| key)
    }

    pub fn install(associate: bool) -> io::Result<()> {
        let exe = std::env::current_exe()?.to_string_lossy().into_owned();
        // From a clean slate, so that installing again without association drops the old one.
        uninstall()?;
        let classes = classes()?;
        for Entry { key, values } in entries(&exe, associate) {
            let (key, _) = classes.create_subkey(&key)?;
            for (name, value) in values {
                key.set_value(name, &value)?;
            }
        }
        refresh_explorer();
        Ok(())
    }

    pub fn uninstall() -> io::Result<()> {
        let classes = classes()?;
        for key in owned_keys() {
            match classes.delete_subkey_all(&key) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        for ext in EXTENSIONS {
            if let Ok(key) = classes.open_subkey_with_flags(
                format!(r"{ext}\OpenWithProgids"),
                winreg::enums::KEY_ALL_ACCESS,
            ) {
                let _ = key.delete_value(PROG_ID);
            }
            // The extension goes back to opening with nothing, rather than with what it had
            // before, which Windows does not keep; it asks the next time such a file is opened.
            if let Ok(key) = classes.open_subkey_with_flags(ext, winreg::enums::KEY_ALL_ACCESS)
                && key.get_value::<String, _>("").is_ok_and(|id| id == PROG_ID)
            {
                key.delete_value("")?;
            }
        }
        refresh_explorer();
        Ok(())
    }

    pub fn status() -> Status {
        let Ok(classes) = classes() else { return Status::NotInstalled };
        let command = classes
            .open_subkey(format!(r"{PROG_ID}\shell\open\command"))
            .and_then(|key| key.get_value::<String, _>(""));
        let menu = classes.open_subkey(ITEMS_MENU);
        match (command, menu) {
            (Ok(command), Ok(_)) => Status::Installed {
                exe: command
                    .trim_start_matches('"')
                    .split('"')
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                associated: classes
                    .open_subkey(".vdf")
                    .and_then(|key| key.get_value::<String, _>(""))
                    .is_ok_and(|id| id == PROG_ID),
            },
            _ => Status::NotInstalled,
        }
    }

    /// Tells Explorer that file types changed, so it shows the new icon and menus without a
    /// restart.
    fn refresh_explorer() {
        use ::windows::Win32::UI::Shell::SHCNE_ASSOCCHANGED;
        use ::windows::Win32::UI::Shell::SHCNF_IDLIST;
        use ::windows::Win32::UI::Shell::SHChangeNotify;
        // SAFETY: this event takes no items; both item pointers are null, as documented.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

#[cfg(not(windows))]
pub fn install(_associate: bool) -> std::io::Result<()> {
    Err(std::io::Error::other("Explorer integration is only for Windows"))
}

#[cfg(not(windows))]
pub fn uninstall() -> std::io::Result<()> {
    Err(std::io::Error::other("Explorer integration is only for Windows"))
}

#[cfg(not(windows))]
pub fn status() -> Status {
    Status::NotInstalled
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Tools\vdfrs.exe";

    fn find<'a>(entries: &'a [Entry], key: &str) -> &'a Entry {
        entries.iter().find(|e| e.key == key).unwrap_or_else(|| panic!("no key {key}"))
    }

    fn value<'a>(entry: &'a Entry, name: &str) -> &'a str {
        entry.values.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str()).unwrap()
    }

    #[test]
    fn menus_run_the_command_line() {
        let entries = entries(EXE, false);
        let here = find(&entries, r"vdfrs.menu.archive\shell\3here\command");
        assert_eq!(value(here, ""), r#""C:\Tools\vdfrs.exe" extract --here --gather "%1""#);
        let pack = find(&entries, r"vdfrs.menu.items\shell\2pack\command");
        assert_eq!(value(pack, ""), r#""C:\Tools\vdfrs.exe" pack --gather "%1""#);
        let open = find(&entries, r"vdfrs.archive\shell\open\command");
        assert_eq!(value(open, ""), r#""C:\Tools\vdfrs.exe" "%1""#);

        // Every command parses as the command line it is meant to be.
        for entry in entries.iter().filter(|e| e.key.ends_with(r"\command")) {
            let line = value(entry, "").replace("%1", r"C:\Data\Anims.vdf");
            let args = split_command_line(&line);
            use clap::Parser as _;
            assert!(crate::cli::Cli::try_parse_from(&args).is_ok(), "{line}");
        }
    }

    /// Splits a command line as Windows does for these simple ones: on spaces, outside quotes.
    fn split_command_line(line: &str) -> Vec<String> {
        let (mut args, mut current, mut quoted) = (Vec::new(), String::new(), false);
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ' ' if !quoted => {
                    args.extend((!current.is_empty()).then(|| std::mem::take(&mut current)))
                }
                c => current.push(c),
            }
        }
        args.extend((!current.is_empty()).then_some(current));
        args
    }

    #[test]
    fn submenus_point_at_their_verbs() {
        let entries = entries(EXE, false);
        for key in
            [r"SystemFileAssociations\.vdf\shell\vdfrs", r"SystemFileAssociations\.mod\shell\vdfrs"]
        {
            let menu = find(&entries, key);
            let verbs = value(menu, "ExtendedSubCommandsKey");
            assert!(entries.iter().any(|e| e.key.starts_with(&format!(r"{verbs}\shell\"))));
        }
        let files = find(&entries, r"*\shell\vdfrs");
        assert_eq!(value(files, "AppliesTo"), NOT_ARCHIVES);
        assert_eq!(value(files, "ExtendedSubCommandsKey"), ITEMS_MENU);
        assert_eq!(
            value(find(&entries, r"Directory\shell\vdfrs"), "ExtendedSubCommandsKey"),
            ITEMS_MENU
        );
    }

    #[test]
    fn associating_is_a_choice() {
        assert!(!entries(EXE, false).iter().any(|e| e.key == ".vdf"));
        let associated = entries(EXE, true);
        assert_eq!(value(find(&associated, ".mod"), ""), PROG_ID);
    }

    #[test]
    fn uninstalling_covers_every_key_vdfrs_owns() {
        let owned = owned_keys();
        for entry in entries(EXE, true) {
            let shared = entry.key.starts_with('.');
            let covered = owned
                .iter()
                .any(|key| entry.key == *key || entry.key.starts_with(&format!(r"{key}\")));
            assert!(shared || covered, "{} is left behind", entry.key);
        }
    }
}
