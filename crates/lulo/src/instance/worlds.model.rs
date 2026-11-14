//! The worlds an instance has saved: `<cachedir>/Saves/<game mode>/<world>/`, the folder the
//! game's Load screen reads.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// Its folder: what was typed when it was made, or the game's default, the date and time.
    pub name: String,
    /// The character in it, when the game has saved one.
    pub character: Option<String>,
    /// The game mode folder it sits in (`Apocalypse`, `Sandbox`...).
    pub mode: String,
    pub dir: PathBuf,
    /// The screenshot the game takes when it saves.
    pub thumb: Option<PathBuf>,
    /// When it was last saved, in seconds since the epoch.
    pub saved: u64,
}

impl World {
    /// What the row is called: the name it was given, or its character's when it kept the
    /// game's default (a date says less than "Lucas Aaron").
    pub fn title(&self) -> &str {
        match &self.character {
            Some(character) if is_default_name(&self.name) => character,
            _ => &self.name,
        }
    }
}

/// Whether `name` is the one the game offers for a new world, `2026-11-14_14-31-22`.
fn is_default_name(name: &str) -> bool {
    let shape = "dddd-dd-dd_dd-dd-dd";
    name.len() == shape.len()
        && name.bytes().zip(shape.bytes()).all(|(c, s)| {
            if s == b'd' {
                c.is_ascii_digit()
            } else {
                c == s
            }
        })
}

/// The name of the first character in a save's `players.db`, the one Solo plays.
fn character(dir: &Path) -> Option<String> {
    use rusqlite::{Connection, OpenFlags};
    // Read-only, so a game that is running and saving is never in the way.
    let db = Connection::open_with_flags(
        dir.join("players.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.query_row(
        "SELECT name FROM localPlayers ORDER BY id LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|name| name.trim().to_owned())
    .filter(|name| !name.is_empty())
}

/// The saves in `cachedir`, the last played first.
pub fn list(cachedir: &Path) -> Vec<World> {
    let Ok(modes) = std::fs::read_dir(cachedir.join("Saves")) else {
        return Vec::new();
    };
    let mut worlds: Vec<World> = modes
        .flatten()
        .filter(|mode| mode.path().is_dir())
        // Servers' cached copies, not worlds Solo can load.
        .filter(|mode| mode.file_name() != "Multiplayer")
        .flat_map(|mode| {
            let name = mode.file_name().to_string_lossy().into_owned();
            std::fs::read_dir(mode.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|world| world.path().is_dir())
                .map(move |world| read(&world.path(), &name))
        })
        .collect();
    worlds.sort_by_key(|world| std::cmp::Reverse(world.saved));
    worlds
}

fn read(dir: &Path, mode: &str) -> World {
    // The game rewrites `map_t.bin` (the world's clock) on every save; the folder's own time
    // only moves when files come and go.
    let saved = [dir.join("map_t.bin"), dir.to_owned()]
        .iter()
        .find_map(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_secs());
    let thumb = dir.join("thumb.png");
    World {
        name: dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        character: character(dir),
        mode: mode.to_owned(),
        thumb: thumb.is_file().then_some(thumb),
        dir: dir.to_owned(),
        saved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_saves_by_mode_last_played_first() {
        let dir = std::env::temp_dir().join(format!("lulo-worlds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let world = |path: &str, thumb: bool, saved: u64| {
            let at = dir.join("Saves").join(path);
            std::fs::create_dir_all(&at).unwrap();
            let clock = std::fs::File::create(at.join("map_t.bin")).unwrap();
            clock
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(saved))
                .unwrap();
            if thumb {
                std::fs::write(at.join("thumb.png"), "png").unwrap();
            }
        };
        assert!(list(&dir).is_empty(), "no Saves folder yet");
        world("Apocalypse/2026-11-14_14-31-22", true, 1_000);
        world("Sandbox/Rosewood run", false, 2_000);
        world("Multiplayer/127.0.0.1_16261_abc", true, 3_000);

        let found = list(&dir);
        let names: Vec<(&str, &str)> = found
            .iter()
            .map(|w| (w.mode.as_str(), w.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Sandbox", "Rosewood run"),
                ("Apocalypse", "2026-11-14_14-31-22")
            ]
        );
        assert_eq!(found[0].thumb, None);
        assert_eq!(
            found[1].thumb,
            Some(dir.join("Saves/Apocalypse/2026-11-14_14-31-22/thumb.png"))
        );
        assert_eq!(found[1].saved, 1_000);
        // No `players.db`: the folder's name is all there is.
        assert_eq!(found[1].title(), "2026-11-14_14-31-22");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_world_with_the_default_name_goes_by_its_character() {
        let dir = std::env::temp_dir().join(format!("lulo-players-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The table as build 42.22 makes it, and the row it saved.
        let db = rusqlite::Connection::open(dir.join("players.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE localPlayers (id INTEGER PRIMARY KEY NOT NULL,name STRING,wx INTEGER,\
             wy INTEGER,x FLOAT,y FLOAT,z FLOAT,worldversion INTEGER,data BLOB,isDead BOOLEAN);\
             INSERT INTO localPlayers (name, x, y, isDead) VALUES ('Lucas Aaron', 3729.5, 8007.5, 0);",
        )
        .unwrap();
        drop(db);
        let mut world = read(&dir, "Apocalypse");
        assert_eq!(world.character.as_deref(), Some("Lucas Aaron"));
        world.name = "2026-11-14_14-31-22".into();
        assert_eq!(world.title(), "Lucas Aaron");
        world.name = "Rosewood run".into();
        assert_eq!(world.title(), "Rosewood run", "a name someone typed wins");
        assert!(!is_default_name("2026-11-14_14-31"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
