//! The window's look: the theme, and on Windows 11 the Mica materials behind it.

use gpui_kit::App;
use gpui_kit::Hsla;
use gpui_kit::Window;
use gpui_kit::WindowBackgroundAppearance;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Theme;
use gpui_kit::component::ThemeMode;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;

use crate::prefs::Material;
use crate::prefs::Prefs;
use crate::prefs::ThemeChoice;

/// Whether this system can show Mica: DWM only has the backdrop from Windows 11 22H2, build 22621.
/// GPUI asks for it anyway on older systems and draws the window see-through, so vdfrs must not.
#[cfg(windows)]
pub fn mica_supported() -> bool {
    static MICA: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| windows_build().is_some_and(|build| build >= 22621));
    *MICA
}

#[cfg(not(windows))]
pub fn mica_supported() -> bool {
    false
}

/// Applies the saved theme and material to `window`. Called again whenever either changes, and
/// when the system's appearance does.
pub fn apply(window: &mut Window, cx: &mut App) {
    let prefs = *cx.global::<Prefs>();
    let mode = match prefs.theme {
        ThemeChoice::System => ThemeMode::from(window.appearance()),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    };
    // Loads the theme afresh, without whatever a material made see-through before.
    Theme::change(mode, Some(&mut *window), cx);
    let material = if mica_supported() { prefs.material } else { Material::Solid };
    if material != Material::Solid {
        Theme::update(cx, see_through);
    }
    window.set_background_appearance(match material {
        Material::Solid => WindowBackgroundAppearance::Opaque,
        Material::Mica => WindowBackgroundAppearance::MicaBackdrop,
        Material::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
    });
    #[cfg(windows)]
    dark_frame(window, mode.is_dark());
}

/// Changes the settings, saves them and applies them to `window`.
pub fn change(window: &mut Window, cx: &mut App, edit: impl FnOnce(&mut Prefs)) {
    edit(cx.global_mut::<Prefs>());
    if let Err(error) = cx.global::<Prefs>().save() {
        let note = Notification::error(format!("The settings were not saved: {error}"));
        window.push_notification(note, cx);
    }
    apply(window, cx);
}

/// The fill for dialogs. They take the theme's background otherwise, which a material clears.
pub fn dialog_fill(cx: &App) -> Hsla {
    cx.theme().popover
}

/// Lets the material show through the window, and faintly through its panes, so they stay
/// apart. The background has to go: GPUI Kit paints it under the whole window.
fn see_through(theme: &mut Theme) {
    let clear = theme.transparent;
    theme.background = clear;
    theme.title_bar = clear;
    theme.status_bar = clear;
    theme.tab_bar = clear;
    theme.tab = clear;
    theme.tab_active = theme.tab_active.opacity(0.6);
    theme.sidebar = theme.sidebar.opacity(0.35);
    theme.list_head = theme.list_head.opacity(0.5);
}

#[cfg(windows)]
fn windows_build() -> Option<u32> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    key.get_value::<String, _>("CurrentBuildNumber").ok()?.parse().ok()
}

/// Tells DWM whether the window is dark, which decides Mica's tint. GPUI sets it from the
/// system's appearance, so it is wrong whenever the theme chosen here differs.
#[cfg(windows)]
#[allow(unsafe_code)]
fn dark_frame(window: &Window, dark: bool) {
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE;
    use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows::core::BOOL;

    // Spelled out, since Window has a `window_handle` of its own that gives GPUI's handle.
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    let dark = BOOL::from(dark);
    // SAFETY: `hwnd` is this window's, which lives as long as `window` is borrowed, and the
    // attribute is read as one BOOL, the size given.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            size_of::<BOOL>() as u32,
        )
    };
}
