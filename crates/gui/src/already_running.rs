// SPDX-License-Identifier: GPL-3.0-only

//! What a second copy of the app shows before it exits (see
//! `txbm_core::instance`).

pub const MESSAGE: &str = "TinyXbox360BackupManager is already running.";

// On Linux `rfd` draws message boxes by running `zenity`, which many desktops
// and the Flatpak runtime do not have: the call then returns at once and the
// second copy quit without a word. So Linux gets a small window of our own.
//
// It is declared inline rather than under `ui/`: a second window exported from
// the main `.slint` tree would make every `app.global::<…>()` call ambiguous,
// each global then belonging to two components.
#[cfg(target_os = "linux")]
mod window {
    slint::slint! {
        import { Button, Palette } from "std-widgets.slint";

        export component AlreadyRunningWindow inherits Window {
            in property <string> message;
            callback dismissed();

            title: "TinyXbox360BackupManager";
            icon: @image-url("../../../package/windows/TinyXbox360BackupManager-64x64.png");
            background: Palette.background;
            default-font-size: 14px;

            VerticalLayout {
                padding: 20px;
                spacing: 20px;

                Text {
                    text: root.message;
                    horizontal-alignment: center;
                }

                HorizontalLayout {
                    alignment: center;

                    Button {
                        text: "Ok";
                        primary: true;
                        clicked => {
                            root.dismissed();
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn show() {
    use slint::ComponentHandle;

    let _ = slint::set_xdg_app_id("fr.dechriste.TinyXbox360BackupManager");

    let Ok(window) = window::AlreadyRunningWindow::new() else {
        return;
    };
    window.set_message(MESSAGE.into());
    window.on_dismissed(|| {
        let _ = slint::quit_event_loop();
    });
    // Returns when the button is clicked or the window is closed.
    let _ = window.run();
}

#[cfg(not(target_os = "linux"))]
pub fn show() {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title("TinyXbox360BackupManager")
        .set_description(MESSAGE)
        .show();
}
