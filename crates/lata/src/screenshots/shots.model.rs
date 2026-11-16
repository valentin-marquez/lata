//! The screenshots the game took in each instance (`zomboid/Screenshots/*.png`, named by when
//! they were taken), and the small copies the grid shows.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use gpui_kit::SharedString;

use crate::instance::Instance;

#[derive(Clone, PartialEq, Debug)]
pub struct Shot {
    pub path: PathBuf,
    /// The instance's id and name.
    pub instance: SharedString,
    pub name: SharedString,
    pub taken: NaiveDateTime,
    /// Width and height, read from the file's header: the viewer fits the picture by them.
    pub size: (u32, u32),
}

/// Every screenshot of `instances`, newest first.
pub fn list(instances: &[Instance]) -> Vec<Shot> {
    let mut shots: Vec<Shot> = instances
        .iter()
        .flat_map(|instance| {
            std::fs::read_dir(instance.dir.join("zomboid/Screenshots"))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(move |entry| {
                    let path = entry.path();
                    if !path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
                    {
                        return None;
                    }
                    let taken = taken(path.file_stem()?.to_str()?).or_else(|| modified(&entry))?;
                    // One still being written has no header yet: 16:9 until the next look.
                    let size = image::image_dimensions(&path).unwrap_or((16, 9));
                    Some(Shot {
                        size,
                        path,
                        instance: instance.id.clone(),
                        name: instance.name.clone(),
                        taken,
                    })
                })
        })
        .collect();
    // By name the order would be by day of the month; ties go by path so it never shuffles.
    shots.sort_by(|a, b| b.taken.cmp(&a.taken).then_with(|| b.path.cmp(&a.path)));
    shots
}

/// `screenshot_09-10-2026_23-36-58`: day, month and year, then the time.
fn taken(stem: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(stem.strip_prefix("screenshot_")?, "%d-%m-%Y_%H-%M-%S").ok()
}

/// A file named some other way goes by when it was written.
fn modified(entry: &std::fs::DirEntry) -> Option<NaiveDateTime> {
    let time = entry.metadata().ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Local>::from(time).naive_local())
}

/// The longest side of a grid picture: twice a tile, so it stays sharp on HiDPI screens.
const THUMB: u32 = 480;

/// A small copy of `shot` for the grid, made once into `cache`: hundreds of full screenshots
/// decoded for a grid would take gigabytes. Fails on a picture still being written; the next
/// look tries again.
pub fn thumb(shot: &Path, cache: &Path) -> io::Result<PathBuf> {
    let meta = std::fs::metadata(shot)?;
    let mut hash = DefaultHasher::new();
    (shot, meta.len(), meta.modified().ok()).hash(&mut hash);
    let out = cache.join(format!("{:016x}.png", hash.finish()));
    if out.is_file() {
        return Ok(out);
    }
    let small = image::open(shot)
        .map_err(io::Error::other)?
        .thumbnail(THUMB, THUMB);
    std::fs::create_dir_all(cache)?;
    // Written aside and renamed, so a half-made copy is never taken for a finished one.
    let tmp = out.with_extension("tmp");
    small
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(io::Error::other)?;
    std::fs::rename(tmp, &out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_when_the_game_took_it() {
        let at = taken("screenshot_09-10-2026_23-36-58").unwrap();
        assert_eq!(at.to_string(), "2026-10-09 23:36:58");
        assert_eq!(taken("holiday"), None);
    }

    #[test]
    fn lists_newest_first_and_makes_small_copies() {
        let base = std::env::temp_dir().join(format!("lata-shots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let shots = base.join("i/zomboid/Screenshots");
        std::fs::create_dir_all(&shots).unwrap();
        let picture = image::RgbaImage::new(1920, 1080);
        // Older by date but later by name: the date wins.
        picture
            .save(shots.join("screenshot_30-01-2025_10-00-00.png"))
            .unwrap();
        picture
            .save(shots.join("screenshot_02-02-2026_10-00-00.png"))
            .unwrap();
        std::fs::write(shots.join("notes.txt"), b"not a picture").unwrap();
        let instance = Instance {
            id: "i".into(),
            dir: base.join("i"),
            name: "I".into(),
            branch: "public".into(),
            build: None,
            last_played: None,
            icon: None,
            memory_gb: None,
            jvm_args: String::new(),
            launch_args: String::new(),
        };
        let found = list(&[instance]);
        assert_eq!(found.len(), 2);
        assert!(
            found[0]
                .path
                .ends_with("screenshot_02-02-2026_10-00-00.png")
        );

        let small = thumb(&found[0].path, &base.join("cache")).unwrap();
        let size = image::image_dimensions(&small).unwrap();
        assert_eq!(size, (480, 270), "keeps the shape");
        assert_eq!(thumb(&found[0].path, &base.join("cache")).unwrap(), small);
        std::fs::remove_dir_all(base).unwrap();
    }
}
