//! The small window lata opens before its main one while it updates itself, as Discord does:
//! the logo, one line saying what is going on, and a bar. Only what it looks like lives
//! here; [`super::start`] decides the step.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::progress::Progress;
use gpui_kit::*;

use crate::instance::install;
use crate::ui::Loader;

const WIDTH: f32 = 300.;
const HEIGHT: f32 = 340.;
/// In a debug build, pins the window on one step and never leaves it, to style it:
/// `LATA_UPDATE_SPLASH=checking|downloading|installing|failed|launching`.
const PIN: &str = "LATA_UPDATE_SPLASH";

/// What the update is doing. The order they happen in: `Checking`, then with an update
/// `Downloading`, `Installing` and `Launching` (the new lata starts and this one leaves);
/// without one straight to `Launching` (the main window opens). `Failed` can follow
/// `Downloading` or `Installing`; it shows for a moment and the main window opens after it.
#[derive(Clone, PartialEq)]
pub enum Step {
    Checking,
    /// Bytes so far and in all.
    Downloading {
        done: u64,
        total: u64,
    },
    Installing,
    /// Why; lata goes on with the version it has.
    Failed(SharedString),
    Launching,
}

impl Step {
    /// The one status line.
    fn line(&self) -> &'static str {
        match self {
            Step::Checking => "Checking for updates",
            Step::Downloading { .. } => "Downloading update",
            Step::Installing => "Installing",
            Step::Failed(_) => "That update didn't make it",
            Step::Launching => "Starting",
        }
    }

    /// The small print under it, when there is something to add.
    fn detail(&self) -> Option<SharedString> {
        match self {
            Step::Downloading { done, total } => {
                Some(format!("{} of {}", install::size(*done), install::size(*total)).into())
            }
            Step::Failed(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// How full the bar is, 0 to 1; `None` while there is nothing to measure.
    fn fraction(&self) -> Option<f32> {
        match self {
            Step::Downloading { done, total } if *total > 0 => {
                Some((*done as f32 / *total as f32).min(1.))
            }
            _ => None,
        }
    }
}

/// The step named in `LATA_UPDATE_SPLASH`, with made-up numbers.
pub fn pinned() -> Option<Step> {
    if !cfg!(debug_assertions) {
        return None;
    }
    Some(match std::env::var(PIN).ok()?.as_str() {
        "checking" => Step::Checking,
        "downloading" => Step::Downloading {
            done: 11_300_000,
            total: 26_990_864,
        },
        "installing" => Step::Installing,
        "failed" => Step::Failed("the download doesn't match the release's checksum".into()),
        "launching" => Step::Launching,
        _ => return None,
    })
}

pub struct Splash {
    pub step: Step,
    /// Set while the update can still be called off: the window then offers to open lata
    /// as it is, and this is what the button does.
    pub on_skip: Option<Rc<dyn Fn()>>,
}

/// Opens the window on `step`.
pub fn open(step: Step, cx: &mut App) -> Result<(AnyWindowHandle, Entity<Splash>)> {
    let bounds = size(px(WIDTH), px(HEIGHT));
    let options = WindowOptions {
        app_id: Some(crate::APP_ID.into()),
        window_bounds: Some(WindowBounds::centered(bounds, cx)),
        window_min_size: Some(bounds),
        // A card in the middle of the screen: no title bar, nothing to resize or minimize.
        titlebar: None,
        kind: WindowKind::Floating,
        window_decorations: Some(WindowDecorations::Client),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        crate::theme::follow_system(window, cx);
        // A pinned download shows the button too; it has nothing to stop.
        let on_skip =
            matches!(step, Step::Downloading { .. }).then(|| Rc::new(|| ()) as Rc<dyn Fn()>);
        cx.new(|_| Splash { step, on_skip })
    })
}

impl Render for Splash {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can = super::can::can(&self.step, 76., window, cx);
        let theme = cx.theme();
        let failed = matches!(self.step, Step::Failed(_));
        let bar = match self.step.fraction() {
            Some(fraction) => Progress::new("update-progress")
                .value(fraction * 100.)
                .color(theme.primary)
                .into_any_element(),
            // Nothing to show after a failure; otherwise the loader that never fills.
            None if failed => div().into_any_element(),
            None => Loader.into_any_element(),
        };
        let skip = self.on_skip.clone().map(|on_skip| {
            crate::ui::secondary("skip-update", "Skip for now").on_click(move |_, _, _| on_skip())
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(can)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.step.line()),
                    )
                    // Always a line tall, so the bar doesn't jump when the detail comes and goes.
                    .child(
                        div()
                            .min_h(px(16.))
                            .text_xs()
                            .text_center()
                            .text_color(theme.muted_foreground)
                            .children(self.step.detail()),
                    ),
            )
            .child(div().w(px(200.)).h(px(8.)).child(bar))
            // The room is kept when the button isn't there, so nothing above it moves.
            .child(div().h(px(36.)).children(skip))
    }
}
