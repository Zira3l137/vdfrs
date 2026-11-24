//! The archive's settings: what its header says when it is saved.

use gpui_kit::AppContext as _;
use gpui_kit::Context;
use gpui_kit::Entity;
use gpui_kit::IntoElement;
use gpui_kit::ParentElement as _;
use gpui_kit::Render;
use gpui_kit::Styled as _;
use gpui_kit::Window;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::form::Field;
use gpui_kit::component::form::Form;
use gpui_kit::component::input::Input;
use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::v_flex;
use gpui_kit::div;
use zenkit::GameVersion;
use zenkit::vfs::DiskOptions;

use crate::archive_view::ArchiveView;
use crate::shell;

pub struct SettingsForm {
    comment: Entity<InputState>,
    game: GameVersion,
    compressed: bool,
}

impl SettingsForm {
    fn options(&self, cx: &gpui_kit::App) -> DiskOptions {
        let mut options = DiskOptions::new(self.game);
        options.comment = self.comment.read(cx).value().to_string();
        options.compressed = self.compressed;
        options
    }
}

impl Render for SettingsForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        Form::new()
            .child(
                Field::new().label("Game").child(
                    RadioGroup::horizontal("game")
                        .children(["Gothic", "Gothic II"])
                        .selected_index(Some(usize::from(self.game == GameVersion::Gothic2)))
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.game =
                                if *ix == 0 { GameVersion::Gothic1 } else { GameVersion::Gothic2 };
                            cx.notify();
                        })),
                ),
            )
            .child(
                Field::new()
                    .label("Comment")
                    .description("Up to 256 characters at the start of the file")
                    .child(Input::new(&self.comment)),
            )
            .child(
                Field::new().label_indent(false).child(
                    v_flex()
                        .gap_1()
                        .child(
                            Switch::new("compressed")
                                .label("Compress like Union")
                                .checked(self.compressed)
                                .on_change(cx.listener(|this, on: &bool, _, cx| {
                                    this.compressed = *on;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("Only games running Union can read compressed archives."),
                        ),
                ),
            )
    }
}

pub fn open(view: Entity<ArchiveView>, window: &mut Window, cx: &mut gpui_kit::App) {
    let options = view.read(cx).archive().options().clone();
    let form = cx.new(|cx| SettingsForm {
        comment: cx.new(|cx| InputState::new(window, cx).default_value(options.comment.clone())),
        game: options.game,
        compressed: options.compressed,
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let form = form.clone();
        let view = view.clone();
        dialog.title("Archive settings").w(gpui_kit::px(440.)).child(form.clone()).footer(
            DialogFooter::new()
                .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| {
                    window.close_dialog(cx);
                }))
                .child(Button::new("apply").primary().label("Apply").on_click(
                    move |_, window, cx| {
                        let options = form.read(cx).options(cx);
                        view.update(cx, |view, cx| view.set_options(options, cx));
                        window.close_dialog(cx);
                    },
                )),
        )
    });
}

/// Whether vdfrs is in Windows Explorer, as the integration dialog edits it.
pub struct IntegrationForm {
    menu: bool,
    associate: bool,
    status: shell::Status,
}

impl Render for IntegrationForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let elsewhere = match &self.status {
            shell::Status::Installed { exe, .. } if !self.status.is_current() => Some(exe.clone()),
            _ => None,
        };
        v_flex()
            .gap_3()
            .child(
                Switch::new("menu")
                    .label("Show vdfrs in Explorer's context menu")
                    .checked(self.menu)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.menu = *on;
                        this.associate &= *on;
                        cx.notify();
                    })),
            )
            .child(
                Switch::new("associate")
                    .label("Open .vdf and .mod files with vdfrs")
                    .checked(self.associate)
                    .disabled(!self.menu)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.associate = *on;
                        cx.notify();
                    })),
            )
            .child(div().text_xs().text_color(muted).child(
                "For this Windows user only, without administrator rights. On Windows 11 the \
                 menu is under Show more options.",
            ))
            .children(elsewhere.map(|exe| {
                div().text_xs().text_color(muted).child(format!(
                    "Explorer now runs {exe}; applying makes it run this copy of vdfrs."
                ))
            }))
    }
}

pub fn open_integration(window: &mut Window, cx: &mut gpui_kit::App) {
    let status = shell::status();
    let form = cx.new(|_| IntegrationForm {
        menu: status != shell::Status::NotInstalled,
        associate: matches!(status, shell::Status::Installed { associated: true, .. }),
        status,
    });
    window.open_dialog(cx, move |dialog, _, _| {
        let form = form.clone();
        dialog.title("Explorer integration").w(gpui_kit::px(460.)).child(form.clone()).footer(
            DialogFooter::new()
                .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| {
                    window.close_dialog(cx);
                }))
                .child(Button::new("apply").primary().label("Apply").on_click(
                    move |_, window, cx| {
                        let form = form.read(cx);
                        let (result, done) = if form.menu {
                            (shell::install(form.associate), "vdfrs is in Explorer's context menu")
                        } else {
                            (shell::uninstall(), "vdfrs is out of Explorer")
                        };
                        window.close_dialog(cx);
                        let note = match result {
                            Ok(()) => Notification::success(done),
                            Err(error) => {
                                Notification::error(format!("Explorer was not changed: {error}"))
                            }
                        };
                        window.push_notification(note, cx);
                    },
                )),
        )
    });
}
