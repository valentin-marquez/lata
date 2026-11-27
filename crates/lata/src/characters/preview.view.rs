//! The character in 3D, as the game would load it: its body with the skin, hair and beard of
//! a look, read from the installed build's models and textures and drawn by [`render`]. It
//! walks in place with the game's own walk, the one its character screen plays; with reduced
//! motion it stands still. Dragging sideways turns it.
//!
//! Pictures are drawn on the background, one at a time, so a slow one costs the walk a step
//! instead of stalling the app.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use figure::{Bounds, Clip, Part, model, pose, render};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use super::catalog;
use super::character::{Look, Rgb};

/// Radians per pixel dragged.
const TURN: f32 = 0.012;

/// Pictures a second while it walks: smooth enough for a walk, and half the cost of drawing at
/// the display's rate.
const FPS: f32 = 30.;

/// How fast the game's character screen plays the walk (`AnimSets/player-avatar/walk`).
const SPEED: f32 = 0.8;

/// The strip under the picture for the drag hint, so it never covers the feet.
const HINT: Pixels = px(24.);

pub struct Preview {
    media: PathBuf,
    skins: Vec<Rgb>,
    /// What the parts were loaded for, to load them again only when the look changes.
    shown: Option<String>,
    /// Counts loads, so a new look draws a new picture.
    loads: u32,
    /// Posed and drawn on the background; `None` without a look. A new look gets new parts,
    /// so a picture still drawing the old ones never holds this one up.
    parts: Option<Arc<Mutex<Vec<Part>>>>,
    /// The box the parts fill over the whole walk, so the camera holds still while it moves.
    frame: Bounds,
    walk: Arc<Clip>,
    /// Its first pose is the still one, for reduced motion.
    idle: Arc<Clip>,
    start: Instant,
    /// The wait for the next picture of the walk, while one is due.
    tick: Option<Task<()>>,
    yaw: f32,
    /// Where a drag started, and the turn then.
    drag: Option<(Pixels, f32)>,
    /// The picture, and what it was drawn for: the load, turn, size and step of the walk.
    image: Option<(Arc<RenderImage>, Drawn)>,
    /// The models of the newest look, loading.
    loading: Option<Task<()>>,
    /// The picture being drawn. Only one at a time: the next starts from where things are when
    /// this one lands, so none queue up behind a slow one.
    drawing: Option<Task<()>>,
    size: Size<Pixels>,
    /// The picture's size in device pixels when last laid out.
    pixels: Size<u32>,
}

type Drawn = (u32, f32, Size<u32>, u64);

impl Preview {
    pub fn new(
        media: PathBuf,
        skins: Vec<Rgb>,
        size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Self {
        // GPUI keeps a drawn picture until told to drop it.
        cx.on_release(|this, cx| {
            if let Some((image, _)) = this.image.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let clip = |name: &str| {
            find(&media.join("anims_X").join("Bob"), &format!("{name}.x"))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|text| model::clip(&text))
                .unwrap_or_default()
        };
        Self {
            walk: Arc::new(clip("Bob_Walk")),
            idle: Arc::new(clip("Bob_Idle")),
            media,
            skins,
            shown: None,
            loads: 0,
            parts: None,
            frame: ([0.; 3], [0.; 3]),
            start: Instant::now(),
            tick: None,
            yaw: 0.,
            drag: None,
            image: None,
            loading: None,
            drawing: None,
            size,
            pixels: Size::default(),
        }
    }

    /// Shows `look`, loading its models on the background if they changed. The last look
    /// stays on screen until they land; a load overtaken by a newer look is dropped.
    pub fn show(&mut self, look: Option<&Look>, cx: &mut Context<Self>) {
        let key = look.map(|l| format!("{}{:?}{:?}{}", l.female(), l.skin(), l.hair(), l.beard()));
        if key == self.shown {
            return;
        }
        self.shown = key;
        let Some(look) = look.cloned() else {
            self.loading = None;
            self.parts = None;
            self.loads += 1;
            return;
        };
        let (media, skins) = (self.media.clone(), self.skins.clone());
        let (walk, idle) = (self.walk.clone(), self.idle.clone());
        let work = cx.background_spawn(async move {
            let mut parts = parts(&media, &look, &skins);
            let frame = frame(&mut parts, &walk, &idle);
            (parts, frame)
        });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let (parts, frame) = work.await;
            this.update(cx, |this, cx| {
                this.loading = None;
                this.frame = frame;
                this.parts = (!parts.is_empty()).then(|| Arc::new(Mutex::new(parts)));
                this.loads += 1;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Preview {
    fn release(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// Starts drawing `parts` for `drawn` on the background; the picture replaces the shown
    /// one when it lands, unless the look changed meanwhile.
    fn draw(
        &mut self,
        parts: Arc<Mutex<Vec<Part>>>,
        walking: bool,
        drawn: Drawn,
        cx: &mut Context<Self>,
    ) {
        let (_, yaw, pixels, step) = drawn;
        let (clip, time) = if walking {
            let time = (step as f32 / FPS * SPEED) % self.walk.length;
            (self.walk.clone(), time)
        } else {
            (self.idle.clone(), 0.)
        };
        let frame = self.frame;
        let work = cx.background_spawn(async move {
            let mut parts = parts.lock().unwrap_or_else(PoisonError::into_inner);
            for part in parts.iter_mut() {
                pose(&mut part.mesh, &clip, time);
            }
            let mut picture = render::render(&parts, frame, yaw, pixels.width, pixels.height, true);
            // GPUI keeps images in BGRA.
            for p in picture.pixels_mut() {
                p.0.swap(0, 2);
            }
            Arc::new(RenderImage::new(vec![image::Frame::new(picture)]))
        });
        self.drawing = Some(cx.spawn(async move |this, cx| {
            let image = work.await;
            this.update(cx, |this, cx| {
                this.drawing = None;
                // A picture of a look no longer shown never reached GPUI: nothing to drop.
                if drawn.0 == this.loads
                    && let Some((old, _)) = this.image.replace((image, drawn))
                {
                    cx.drop_image(old, None);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Starts the picture for how things are now, unless one is drawing or the shown one is
    /// it. Returns the step of the walk, while it walks.
    fn refresh(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) -> Option<u64> {
        let walking = self.walking(cx);
        let step = walking.then(|| (self.start.elapsed().as_secs_f32() * FPS) as u64);
        let drawn = (self.loads, self.yaw, self.pixels, step.unwrap_or(0));
        match self.parts.clone() {
            Some(parts) => {
                if self.drawing.is_none() && self.image.as_ref().is_none_or(|(_, at)| *at != drawn)
                {
                    self.draw(parts, walking, drawn, cx);
                }
            }
            // Gone at once, without waiting for a picture still drawing. The window being
            // drawn is out of GPUI's list: it has to be named to drop the picture from it.
            None => {
                if let Some((old, _)) = self.image.take() {
                    cx.drop_image(old, window);
                }
            }
        }
        step
    }

    fn walking(&self, cx: &App) -> bool {
        !cx.reduce_motion() && self.walk.length > 0. && self.parts.is_some()
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        let area = size(self.size.width, self.size.height - HINT);
        let pixels = size(
            (f32::from(area.width) * scale) as u32,
            (f32::from(area.height) * scale) as u32,
        );
        self.pixels = pixels;
        let step = self.refresh(Some(window), cx);
        // The next step is asked for only while this is drawn, so the walk stops when the
        // preview is hidden or closed.
        if let Some(step) = step
            && self.tick.is_none()
        {
            let next = Duration::from_secs_f32((step + 1) as f32 / FPS);
            let wait = next.saturating_sub(self.start.elapsed());
            self.tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| {
                    this.tick = None;
                    // Without redrawing the window: the picture does that when it lands.
                    this.refresh(None, cx);
                    if this.drawing.is_none() {
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        let muted = cx.theme().muted_foreground;
        div()
            .id("character-preview")
            .flex()
            .flex_col()
            .w(self.size.width)
            .h(self.size.height)
            .flex_none()
            .cursor_grab()
            .child(
                div()
                    .w(area.width)
                    .h(area.height)
                    .children(self.image.as_ref().map(|(image, _)| {
                        img(ImageSource::Render(image.clone()))
                            .w(area.width)
                            .h(area.height)
                    })),
            )
            .child(
                div()
                    .h(HINT)
                    .w_full()
                    .text_center()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("Drag to rotate")),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, _| {
                    this.drag = Some((e.position.x, this.yaw));
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some((x, yaw)) = this.drag {
                    if e.pressed_button != Some(MouseButton::Left) {
                        this.drag = None;
                    } else {
                        this.yaw = yaw + f32::from(e.position.x - x) * TURN;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::release))
            // Let go over the rest of the dialog.
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::release))
    }
}

/// The box `parts` fill over the walk and the still pose, sampled often enough that a frame
/// between samples doesn't stick out.
fn frame(parts: &mut [Part], walk: &Clip, idle: &Clip) -> Bounds {
    let steps = 16;
    let poses = (0..steps)
        .map(|i| (walk, walk.length * i as f32 / steps as f32))
        .chain([(idle, 0.)]);
    let mut sum: Option<Bounds> = None;
    for (clip, time) in poses {
        for part in parts.iter_mut() {
            pose(&mut part.mesh, clip, time);
        }
        let (min, max) = render::bounds(parts);
        sum = Some(sum.map_or((min, max), |(a, b)| {
            (
                std::array::from_fn(|i| a[i].min(min[i])),
                std::array::from_fn(|i| b[i].max(max[i])),
            )
        }));
    }
    sum.unwrap_or_default()
}

/// The body, hair and beard of `look`, with the textures the game would give them. A part
/// whose files are missing is left out.
///
/// The skin texture is the game's for the listed skin color nearest the saved one: the game
/// keeps the index the color picker had, the preset only the color.
// ponytail: the plain `MaleBody0N` texture; the game also has hairier `0Na` ones (chest hair).
fn parts(media: &Path, look: &Look, skins: &[Rgb]) -> Vec<Part> {
    let female = look.female();
    let body = if female { "FemaleBody" } else { "MaleBody" };
    let skin = look.skin().unwrap_or([1., 0.91, 0.72]);
    let distance = |c: &Rgb| {
        c.iter()
            .zip(skin)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
    };
    let index = (0..skins.len())
        .min_by(|a, b| distance(&skins[*a]).total_cmp(&distance(&skins[*b])))
        .unwrap_or(0);
    let mut parts = Vec::new();
    let mut add = |model: &str, texture: &str, tint: Rgb| {
        let Some(text) = find(&media.join("models_X"), &format!("{model}.x"))
            .and_then(|p| std::fs::read(p).ok())
        else {
            return;
        };
        let open = |file: &str| {
            find(&media.join("textures"), file)
                .and_then(|p| image::open(p).ok())
                .map(|i| i.into_rgba8())
        };
        let texture = open(&format!("{texture}.png"));
        for mesh in model::parse(&String::from_utf8_lossy(&text)) {
            // The one the model names, when the game's pick isn't there.
            let texture = texture
                .clone()
                .or_else(|| mesh.texture.as_deref().and_then(open));
            parts.push(Part {
                mesh,
                texture,
                tint,
            });
        }
    };
    add(
        &format!("Skinned/{body}"),
        &format!("Body/{body}0{}", index + 1),
        [1.; 3],
    );
    let read = |file: &str| std::fs::read_to_string(media.join("hairStyles").join(file));
    let (hair, color) = look.hair();
    let color = color.unwrap_or([0.2, 0.15, 0.1]);
    let tag = if female { "female" } else { "male" };
    let mut styles = vec![(read("hairStyles.xml"), tag, hair)];
    if !female && !look.beard().is_empty() {
        styles.push((read("beardStyles.xml"), "style", look.beard().to_owned()));
    }
    for (xml, tag, name) in styles {
        if let Some((model, texture)) = xml.ok().and_then(|x| catalog::style(&x, tag, &name))
            && !model.is_empty()
        {
            add(&model, &texture, color);
        }
    }
    parts
}

/// `dir/rel`, matching each part of `rel` in any case: the styles files write model paths in
/// lower case (with either slash), and Linux file names aren't.
fn find(dir: &Path, rel: &str) -> Option<PathBuf> {
    rel.split(['/', '\\'])
        .try_fold(dir.to_path_buf(), |at, part| {
            let exact = at.join(part);
            if exact.exists() {
                return Some(exact);
            }
            std::fs::read_dir(&at)
                .ok()?
                .flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                .map(|e| e.path())
        })
}

#[cfg(test)]
mod tests {
    use super::{Look, Path, PathBuf, find, frame, model, parts, pose, render};

    /// Against the B42 build in the demo library, when it is here; `LATA_PREVIEW_OUT` names a
    /// folder to write the pictures to.
    #[test]
    #[ignore]
    fn renders_the_installed_build() {
        let media = PathBuf::from(
            "/mnt/datos/dev/lata/research/demo/home/builds/2978601190/projectzomboid/media",
        );
        let skins = vec![
            [1., 0.91, 0.72],
            [0.98, 0.79, 0.49],
            [0.8, 0.65, 0.45],
            [0.54, 0.38, 0.25],
            [0.36, 0.25, 0.14],
        ];
        let clip = |name: &str| {
            let path = find(&media.join("anims_X").join("Bob"), name).unwrap();
            model::clip(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let (walk, idle) = (clip("Bob_Walk.x"), clip("Bob_Idle.x"));
        let looks = [
            (
                "male",
                "gender=2;skincolor=0.8,0.65,0.45;hair=Messy|0.3,0.2,0.1;beard=Full;",
            ),
            (
                "female",
                "gender=1;skincolor=1,0.91,0.72;hair=Long|0.6,0.4,0.2;",
            ),
        ];
        for (name, look) in looks {
            let mut parts = parts(&media, &Look::parse(look), &skins);
            assert!(parts.len() >= 2, "{name}: body and hair");
            let bounds = frame(&mut parts, &walk, &idle);
            let triangles: usize = parts.iter().map(|p| p.mesh.triangles.len()).sum();
            for (view, yaw) in [("front", 0.), ("three-quarter", 0.8)] {
                for phase in 0..4 {
                    let start = std::time::Instant::now();
                    for part in &mut parts {
                        pose(&mut part.mesh, &walk, walk.length * phase as f32 / 4.);
                    }
                    let posed = start.elapsed();
                    let image = render::render(&parts, bounds, yaw, 400, 600, true);
                    eprintln!(
                        "{name} {view} {phase}: {triangles} triangles, pose {posed:?}, total {:?}",
                        start.elapsed()
                    );
                    if let Ok(out) = std::env::var("LATA_PREVIEW_OUT") {
                        image
                            .save(Path::new(&out).join(format!("{name}-{view}-{phase}.png")))
                            .unwrap();
                    }
                }
            }
        }
    }
}
