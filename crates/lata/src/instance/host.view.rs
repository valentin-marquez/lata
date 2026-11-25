//! Dialogs around hosting a coop server: naming a new one, inviting a friend, and the heads-up
//! about mods friends can't download.

use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;

use super::host;
use crate::shell::Shell;
use crate::ui::{FancyButton, toast};

/// A dialog with a line of text, one field, Cancel and `confirm`. `submit` gets what was
/// typed; an error keeps the dialog open and says why.
fn prompt(
    title: &'static str,
    text: &'static str,
    placeholder: &'static str,
    confirm: &'static str,
    submit: impl Fn(String, &mut Window, &mut App) -> Result<(), String> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    let submit = std::rc::Rc::new(submit);
    window.open_dialog(cx, move |dialog, _, cx| {
        let (input, submit) = (input.clone(), submit.clone());
        let muted = cx.theme().muted_foreground;
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(title)
            .w(px(420.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_sm().text_color(muted).child(text))
                    .child(Input::new(&input)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .pt_4()
                    .child(
                        crate::ui::secondary("prompt-cancel", t!("Cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        FancyButton::new("prompt-confirm", confirm)
                            .small()
                            .on_click(move |_, window, cx| {
                                let typed = input.read(cx).value().trim().to_owned();
                                match submit(typed, window, cx) {
                                    Ok(()) => window.close_dialog(cx),
                                    Err(why) => window.push_notification(toast::failed(why), cx),
                                }
                            }),
                    ),
            )
    });
}

/// Asks for a new server's name and sets it up in the instance.
pub fn create(shell: WeakEntity<Shell>, id: SharedString, window: &mut Window, cx: &mut App) {
    prompt(
        t!("New server"),
        t!("Your friends see this name when they join. Letters, numbers and dashes."),
        "coop",
        t!("Create"),
        move |name, _, cx| {
            let name = if name.is_empty() { "coop".into() } else { name };
            if !host::valid_name(&name) {
                return Err(t!("Letters, numbers and dashes only, up to 32 of them").into());
            }
            let Ok(Some(instance)) = shell.read_with(cx, |shell, _| {
                shell.instances().iter().find(|i| i.id == id).cloned()
            }) else {
                return Ok(());
            };
            host::create(&instance.dir.join("zomboid"), &name).map_err(|err| err.to_string())?;
            let _ = shell.update(cx, |shell, cx| shell.reload(cx));
            Ok(())
        },
        window,
        cx,
    );
}

/// Asks for a friend's Steam ID and lets them in.
pub fn invite(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
    prompt(
        t!("Invite a friend"),
        t!(
            "Paste their Steam ID (17 digits) or their profile link with the number in it. \
             Then send them the join link."
        ),
        "76561197960287930",
        t!("Invite"),
        move |text, window, cx| {
            let friend = host::steam_id(&text).ok_or(t!(
                "That doesn't look like a Steam ID. Try the number from their profile"
            ))?;
            shell
                .update(cx, |shell, cx| shell.invite(friend, cx))
                .map_err(|err| err.to_string())?
                .map_err(|err| err.to_string())?;
            window.push_notification(
                toast::done(t!("They're on the list! Send them the join link")),
                cx,
            );
            Ok(())
        },
        window,
        cx,
    );
}

/// Says which active mods aren't on the Workshop before starting anyway.
pub fn local_mods(
    names: Vec<String>,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    let text = tn!(
        names.len(),
        "This mod is only on this computer, so your friends can't download it: {names}. \
         They'll need to get it some other way to join.",
        "These mods are only on this computer, so your friends can't download them: {names}. \
         They'll need to get them some other way to join.",
        names = names.join(", "),
    );
    let start = std::rc::Rc::new(start);
    window.open_dialog(cx, move |dialog, _, _| {
        let start = start.clone();
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(t!("Heads up, not every mod is on the Workshop"))
            .w(px(460.))
            .child(text.clone())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .pt_4()
                    .child(
                        crate::ui::secondary("local-cancel", t!("Cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        FancyButton::new("local-start", t!("Start anyway"))
                            .small()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                start(window, cx);
                            }),
                    ),
            )
    });
}
