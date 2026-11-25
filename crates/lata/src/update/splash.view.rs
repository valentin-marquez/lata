//! The small window lata opens before its main one while it updates itself, as Discord does:
//! the logo, one line saying what is going on, and a bar. Only what it looks like lives
//! here; [`super::start`] decides the step.

use std::rc::Rc;
use std::time::{Duration, Instant};

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
/// How long each friendly line stays before the next.
const QUIP_EVERY: Duration = Duration::from_secs(8);
/// How long a download may sit still before the window offers to skip it. A moving bar says
/// enough on its own; the button is for one that is stuck or crawling.
const STUCK: Duration = Duration::from_secs(3);
/// Said under the status while lata works, so the window isn't bare: a bit of the game's
/// humor, and things about lata worth knowing.
const QUIPS: [&str; 13] = [
    n!("Pack light. Bring a can opener."),
    n!("Check the doors before checking the fridge."),
    n!("Knox County rewards a quiet entrance."),
    n!("Save the last can for tomorrow."),
    n!("Every safehouse starts with a bad idea."),
    n!("Your next great plan needs an escape route."),
    n!("Lock the door. Then lock it again."),
    n!("Each instance keeps its own mods."),
    n!("Mods stay on the version you installed."),
    n!("A mod update broke your save? Roll it back."),
    n!("Keep a spare instance to try mod updates."),
    n!("A spare instance beats a broken evening."),
    n!("Different instances, different game versions."),
];

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
            Step::Checking => t!("Checking for updates"),
            Step::Downloading { .. } => t!("Downloading update"),
            Step::Installing => t!("Installing update"),
            Step::Failed(_) => t!("That update didn't make it"),
            Step::Launching => t!("Starting lata"),
        }
    }

    /// The small print under it, when there is something to add.
    fn detail(&self) -> Option<SharedString> {
        match self {
            Step::Downloading { done, total } => Some(
                t!(
                    "{done} of {total}",
                    done = install::size(*done),
                    total = install::size(*total)
                )
                .into(),
            ),
            // Errors come from many places, some starting in lowercase.
            Step::Failed(why) => {
                let mut chars = why.chars();
                let first = chars.next()?;
                Some(format!("{}{}", first.to_uppercase(), chars.as_str()).into())
            }
            _ => None,
        }
    }

    /// The line under the status: a friendly one while lata works, what happens next after a
    /// failure, nothing while it starts.
    fn aside(&self, quip: usize) -> Option<&'static str> {
        match self {
            Step::Checking | Step::Downloading { .. } | Step::Installing => {
                Some(crate::i18n::tr(QUIPS[quip % QUIPS.len()]))
            }
            Step::Failed(_) => Some(t!("Opening lata as it is")),
            Step::Launching => None,
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
        "failed" => Step::Failed(t!("the download doesn't match the release's checksum").into()),
        "launching" => Step::Launching,
        _ => return None,
    })
}

pub struct Splash {
    pub step: Step,
    /// When the window opened and which friendly line it began with; the line moves on every
    /// [`QUIP_EVERY`] from there.
    shown: Instant,
    first_quip: usize,
    /// Set while the update can still be called off: the window then offers to open lata
    /// as it is, and this is what the button does.
    pub on_skip: Option<Rc<dyn Fn()>>,
    /// The bytes the download last reported and when that number last changed.
    progress: (u64, Instant),
    /// A press on the window that may turn into dragging it (Linux and macOS; Windows drags
    /// by the area's hit test).
    pressed: bool,
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
        // A different first line each time, without a random number crate for it.
        let first_quip = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos() as usize);
        cx.new(|_| Splash {
            step,
            on_skip,
            shown: Instant::now(),
            first_quip,
            progress: (0, Instant::now()),
            pressed: false,
        })
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
        // The can asks for a frame on every step that has a quip, so time alone moves it on.
        let turn = (self.shown.elapsed().as_secs_f32() / QUIP_EVERY.as_secs_f32()) as usize;
        let aside = self.step.aside(self.first_quip + turn).map(|text| {
            div().relative().child(text).with_animation(
                SharedString::from(text),
                Animation::new(crate::ui::ENTER).with_easing(crate::ui::ease_out_quint),
                |line, t| line.top(px(3. * (1. - t))).opacity(t),
            )
        });
        if let Step::Downloading { done, .. } = self.step
            && done != self.progress.0
        {
            self.progress = (done, Instant::now());
        }
        let stuck = self.progress.1.elapsed() >= STUCK;
        let skip = self.on_skip.clone().filter(|_| stuck).map(|on_skip| {
            crate::ui::secondary("skip-update", t!("Skip for now"))
                .text_color(theme.muted_foreground)
                .on_click(move |_, _, _| on_skip())
                .with_animation(
                    "skip-update-in",
                    Animation::new(crate::ui::ENTER).with_easing(crate::ui::ease_out_quint),
                    |button, t| button.opacity(t),
                )
        });
        div()
            .size_full()
            // It has no title bar, so the whole card moves the window.
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.pressed = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.pressed = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.pressed {
                    this.pressed = false;
                    window.start_window_move();
                }
            }))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
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
                    // Always two lines tall, so nothing moves when the detail comes, goes or
                    // wraps (a failure's reason often does).
                    .child(
                        div()
                            .h(px(32.))
                            .text_xs()
                            .text_center()
                            .text_color(theme.muted_foreground)
                            .children(self.step.detail()),
                    ),
            )
            .child(div().w(px(200.)).h(px(8.)).child(bar))
            // Room for two lines whether or not there is one.
            .child(
                div()
                    .w_full()
                    .h(px(32.))
                    .text_xs()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .children(aside),
            )
            // The room is kept when the button isn't there, so nothing above it moves.
            .child(div().h(px(36.)).children(skip))
    }
}
