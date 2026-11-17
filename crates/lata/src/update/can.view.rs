//! The splash's can, alive: it settles in when the window opens, its lid rings beat while lata
//! looks for an update, the lid lifts while one comes in and lands with a bounce when it is
//! installed, and the eye blinks now and then. Designed in Figma's Motion mode (frame
//! "Updater / splash"); the timings and curves come from there.
//!
//! GPUI can scale and move images but not boxes, and doesn't play SVG animation, so the logo is
//! five SVG layers (`assets/icons/splash/`) placed and sized here every frame from the clock.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::splash::Step;

/// The mark's own coordinates: every layer box below is in these units.
const MARK: (f32, f32) = (502., 512.);

/// A layer of the can: its asset, its one color, and its box in mark units (x, y, w, h).
struct Layer {
    path: &'static str,
    color: u32,
    rect: (f32, f32, f32, f32),
}

/// The logo's two tones: body and face.
const BLUE: u32 = 0x447189;
const FACE: u32 = 0xe6edf1;

const FILL: Layer = Layer {
    path: "splash/fill.svg",
    color: FACE,
    rect: (0.135, 0., 501.729, 512.),
};
const BODY: Layer = Layer {
    path: "splash/body.svg",
    color: BLUE,
    rect: (0.135, 0., 501.729, 512.),
};
const EYE: Layer = Layer {
    path: "splash/eye.svg",
    color: FACE,
    rect: (363.008, 284.128, 110.318, 86.881),
};
const RING_OUTER: Layer = Layer {
    path: "splash/ring-outer.svg",
    color: BLUE,
    rect: (53.366, 34.493, 392.499, 212.766),
};
const RING_INNER: Layer = Layer {
    path: "splash/ring-inner.svg",
    color: BLUE,
    rect: (106.889, 63.506, 285.453, 154.739),
};

const ENTER: Duration = Duration::from_millis(250);
/// One beat of the rings while checking: inner first, outer 150 ms behind.
const BEAT: f32 = 1.2;
const LIFT: Duration = Duration::from_millis(400);
const LAND: Duration = Duration::from_millis(250);
/// The squash when the lid lands: in over 150 ms, back over 250 ms.
const SQUASH_IN: f32 = 0.15;
const SQUASH_OUT: f32 = 0.25;
/// A blink every few seconds: shut in 80 ms, open in 120 ms.
const BLINK_EVERY: f32 = 3.2;
const BLINK_SHUT: f32 = 0.08;
const BLINK_OPEN: f32 = 0.12;

/// When things started, kept between frames.
struct Clock {
    shown: Instant,
    lifted: Option<Instant>,
    landed: Option<Instant>,
}

fn secs(since: Instant, now: Instant) -> f32 {
    now.saturating_duration_since(since).as_secs_f32()
}

fn unit(t: f32, over: Duration) -> f32 {
    (t / over.as_secs_f32()).clamp(0., 1.)
}

fn quint(t: f32) -> f32 {
    crate::ui::ease_out_quint(t)
}

/// cubic-bezier(x1, y1, x2, y2) at progress `x`, solved for the curve's own parameter.
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let at = |a: f32, b: f32, t: f32| {
        let u = 1. - t;
        3. * u * u * t * a + 3. * u * t * t * b + t * t * t
    };
    // Bisection is plenty for a curve drawn at 60 frames a second.
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if at(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.)
}

/// The landing curve, (0.34, 1.45, 0.64, 1): it overshoots, so the lid dips below rest once.
fn overshoot(t: f32) -> f32 {
    bezier(0.34, 1.45, 0.64, 1., t)
}

fn inout(t: f32) -> f32 {
    bezier(0.45, 0., 0.55, 1., t)
}

/// 0 at the ends of `[from, to]` of a cycle, 1 in its middle, eased; 0 outside it.
fn bump(t: f32, from: f32, to: f32) -> f32 {
    if t <= from || t >= to {
        return 0.;
    }
    let u = (t - from) / (to - from);
    inout(if u < 0.5 { u * 2. } else { (1. - u) * 2. })
}

/// How shut the eye is, 0 open to 1 shut, for a blink that started `t` seconds ago.
fn blink(t: f32) -> f32 {
    if !(0. ..=BLINK_SHUT + BLINK_OPEN).contains(&t) {
        0.
    } else if t < BLINK_SHUT {
        inout(t / BLINK_SHUT)
    } else {
        1. - inout((t - BLINK_SHUT) / BLINK_OPEN)
    }
}

/// The can at `size` px wide for `step`.
pub fn can(step: &Step, size: f32, window: &mut Window, cx: &mut App) -> Div {
    let now = Instant::now();
    let reduce = cx.reduce_motion();
    let clock = window.use_keyed_state("splash-can", cx, |_, _| Clock {
        shown: now,
        lifted: None,
        landed: None,
    });
    let lifted = matches!(step, Step::Downloading { .. } | Step::Installing);
    // Note when the lid goes up, and when it comes down after an install.
    let (shown, lifted_at, landed_at) = clock.update(cx, |clock, _| {
        match (lifted, clock.lifted) {
            (true, None) => clock.lifted = Some(now),
            (false, Some(_)) => {
                clock.lifted = None;
                if *step == Step::Launching {
                    clock.landed = Some(now);
                }
            }
            _ => {}
        }
        (clock.shown, clock.lifted, clock.landed)
    });
    let age = secs(shown, now);
    let enter = if reduce { 1. } else { quint(unit(age, ENTER)) };

    // The rings beat while lata waits on GitHub.
    let (beat_inner, beat_outer) = if *step == Step::Checking && !reduce {
        let t = (age % BEAT) / BEAT;
        (
            1. + 0.10 * bump(t, 0., 0.5),
            1. + 0.06 * bump(t, 0.125, 0.625),
        )
    } else {
        (1., 1.)
    };
    // The lid: up while an update comes in, then down with a dip below rest.
    let since_land = landed_at.map(|at| secs(at, now));
    let lift = if reduce {
        0.
    } else if let Some(at) = lifted_at {
        quint(unit(secs(at, now), LIFT))
    } else if let Some(t) = since_land {
        1. - overshoot(unit(t, LAND))
    } else {
        0.
    };
    // The squash as the lid lands, anchored at the can's bottom.
    let (squash_x, squash_y) = match since_land.filter(|_| !reduce) {
        Some(t) if t < SQUASH_IN => {
            let e = inout(t / SQUASH_IN);
            (1. + 0.03 * e, 1. - 0.04 * e)
        }
        Some(t) if t < SQUASH_IN + SQUASH_OUT => {
            let e = overshoot((t - SQUASH_IN) / SQUASH_OUT);
            (1.03 - 0.03 * e, 0.96 + 0.04 * e)
        }
        _ => (1., 1.),
    };
    // A blink every few seconds while working, and a happy one 150 ms after landing.
    let shut = if reduce {
        0.
    } else if let Some(t) = since_land {
        blink(t - 0.15)
    } else if !matches!(step, Step::Failed(_)) {
        blink((age % BLINK_EVERY) - (BLINK_EVERY - 0.4))
    } else {
        0.
    };

    let k = size / MARK.0;
    let (w, h) = (size, MARK.1 * k);
    let enter_scale = 0.92 + 0.08 * enter;
    // Each layer: scaled about its own center, lifted, then squashed and settled about the can.
    let place = |layer: &Layer, sx: f32, sy: f32, dy: f32| {
        let (x, y, lw, lh) = layer.rect;
        let (cx0, cy0) = ((x + lw / 2.) * k, (y + lh / 2.) * k);
        let (lw, lh) = (lw * k * sx, lh * k * sy);
        let (mut left, mut top) = (cx0 - lw / 2., cy0 - lh / 2. + dy);
        let (mut lw, mut lh) = (lw, lh);
        // Squash about the bottom center.
        left = w / 2. + (left - w / 2.) * squash_x;
        top = h + (top - h) * squash_y;
        lw *= squash_x;
        lh *= squash_y;
        // Settle in about the center.
        left = w / 2. + (left - w / 2.) * enter_scale;
        top = h / 2. + (top - h / 2.) * enter_scale;
        lw *= enter_scale;
        lh *= enter_scale;
        // `svg` and not `img`: an image is rasterized once at the file's own 502 px and then
        // shrunk by the GPU without mipmaps, which leaves jagged edges; an svg is rasterized
        // at the size it is drawn. Each layer is one color, which is all `svg` can paint.
        svg()
            .path(layer.path)
            .text_color(rgb(layer.color))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(lw))
            .h(px(lh))
    };
    // Something moves on every step but Failed (the blink at least), so keep drawing.
    if !reduce && (!matches!(step, Step::Failed(_)) || age < ENTER.as_secs_f32()) {
        window.request_animation_frame();
    }
    // Lid travel in the mark's 76 px design: inner 7 px, outer 4 px.
    let unit_px = size / 76.;
    div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .opacity(enter)
        // The fill only has to show through the body's holes (lid, cross), all well inside.
        // At full size it shares the body's outline, and the two smoothed edges stacked let
        // a light rim show around the can; a little smaller, it never reaches the edge.
        .child(place(&FILL, 0.97, 0.97, 0.))
        .child(place(&BODY, 1., 1., 0.))
        .child(place(&EYE, 1., 1. - 0.9 * shut, 0.))
        .child(place(
            &RING_OUTER,
            beat_outer,
            beat_outer,
            -4. * unit_px * lift,
        ))
        .child(place(
            &RING_INNER,
            beat_inner,
            beat_inner,
            -7. * unit_px * lift,
        ))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's own `test` attribute would come along and shadow the std one.
    use super::{BLINK_OPEN, BLINK_SHUT, blink, bump, overshoot};

    #[test]
    fn curves_start_and_end_where_they_should() {
        assert!(overshoot(0.).abs() < 1e-3 && (overshoot(1.) - 1.).abs() < 1e-3);
        assert!(
            (30..90).any(|i| overshoot(i as f32 / 100.) > 1.),
            "the landing dips past rest"
        );
        assert!((bump(0.25, 0., 0.5) - 1.).abs() < 1e-3);
        assert_eq!(bump(0.75, 0., 0.5), 0.);
        assert_eq!(blink(-1.), 0.);
        assert!((blink(BLINK_SHUT) - 1.).abs() < 1e-3, "shut at the turn");
        assert_eq!(blink(BLINK_SHUT + BLINK_OPEN + 0.01), 0.);
    }
}
