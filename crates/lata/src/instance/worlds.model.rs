//! The worlds an instance has saved: `<cachedir>/Saves/<game mode>/<world>/`, the folder the
//! game's Load screen reads.

use std::io;
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

/// Where the game keeps solo saves unless told otherwise: `~/Zomboid/Saves`.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Worlds tab's import action (#36) will call it"
    )
)]
pub fn game_saves_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Zomboid/Saves"))
}

/// Copies the save at `source` (`.../Saves/<game mode>/<world>`) into `cachedir` as a new
/// world, and returns it. The source is only read. A name already taken gets a number
/// (`Rosewood run 2`), nothing is ever overwritten, and the copy is made out of the way and
/// moved into place, so the game never sees a half-copied world. `progress` gets the bytes
/// copied so far and the total, after each file.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Worlds tab's import action (#36) will call it"
    )
)]
pub fn import(
    source: &Path,
    cachedir: &Path,
    mut progress: impl FnMut(u64, u64),
) -> io::Result<World> {
    let (mode, name) = identify(source)?;
    if cachedir.starts_with(source) {
        return Err(invalid("that folder holds the instance itself"));
    }
    let total = size(source)?;
    let staging = cachedir.join(format!(".lata-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let modes = cachedir.join("Saves").join(&mode);
    let placed = std::fs::create_dir_all(&modes)
        .and_then(|()| copy(source, &staging, &mut 0, total, &mut progress))
        .and_then(|()| place(&staging, &modes, &name));
    match placed {
        Ok(dir) => Ok(read(&dir, &mode)),
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(err)
        }
    }
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_owned())
}

/// The game mode and world name of a save folder, or why it isn't one.
fn identify(source: &Path) -> io::Result<(String, String)> {
    if !source.is_dir() {
        return Err(invalid("that isn't a folder"));
    }
    // `map_t.bin` is the world's clock and `players.db` its characters: every save has them.
    if !source.join("map_t.bin").is_file() && !source.join("players.db").is_file() {
        return Err(invalid("that folder isn't a Project Zomboid save"));
    }
    let name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    };
    let (Some(world), Some(mode)) = (name(source), source.parent().and_then(name)) else {
        return Err(invalid("a save sits in a game mode folder"));
    };
    if mode == "Multiplayer" {
        return Err(invalid(
            "that is a server's cached copy, not a world you can load",
        ));
    }
    Ok((mode, world))
}

/// Bytes in the files under `dir`; links are not followed.
fn size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            total += size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Copies the folders and files of `from` to `to`; links are left behind.
fn copy(
    from: &Path,
    to: &Path,
    done: &mut u64,
    total: u64,
    progress: &mut impl FnMut(u64, u64),
) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy(&entry.path(), &target, done, total, progress)?;
        } else if kind.is_file() {
            *done += std::fs::copy(entry.path(), target)?;
            progress(*done, total);
        }
    }
    Ok(())
}

/// Moves `staging` into `modes` as `name`, or `name 2`, `name 3`... when that is taken.
fn place(staging: &Path, modes: &Path, name: &str) -> io::Result<PathBuf> {
    for n in 1.. {
        let target = modes.join(if n == 1 {
            name.to_owned()
        } else {
            format!("{name} {n}")
        });
        if target.exists() {
            continue;
        }
        match std::fs::rename(staging, &target) {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    unreachable!("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_saves_by_mode_last_played_first() {
        let dir = std::env::temp_dir().join(format!("lata-worlds-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("lata-players-{}", std::process::id()));
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

    /// A save with a clock, a screenshot and a file one folder down.
    fn save(at: &Path, clock: &str) {
        std::fs::create_dir_all(at.join("chunks")).unwrap();
        std::fs::write(at.join("map_t.bin"), clock).unwrap();
        std::fs::write(at.join("thumb.png"), "png").unwrap();
        std::fs::write(at.join("chunks/map_1_1.bin"), "chunk").unwrap();
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn imports_a_save_without_touching_the_source() {
        let base = temp("copy");
        let source = base.join("Zomboid/Saves/Sandbox/Rosewood run");
        let cachedir = base.join("instance/zomboid");
        save(&source, "clock");

        let mut seen = Vec::new();
        let world = import(&source, &cachedir, |done, total| seen.push((done, total))).unwrap();
        assert_eq!(
            (world.mode.as_str(), world.name.as_str()),
            ("Sandbox", "Rosewood run")
        );
        assert_eq!(world.dir, cachedir.join("Saves/Sandbox/Rosewood run"));
        let clock = std::fs::read_to_string(world.dir.join("map_t.bin")).unwrap();
        assert_eq!(clock, "clock");
        let chunk = std::fs::read_to_string(world.dir.join("chunks/map_1_1.bin")).unwrap();
        assert_eq!(chunk, "chunk");
        assert!(world.thumb.is_some());
        // 5 + 3 + 5 bytes, reported after each file, ending at the total.
        assert_eq!(seen.len(), 3);
        assert_eq!(seen.last(), Some(&(13, 13)));
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
        // The source is intact and nothing is left behind in the instance.
        assert!(source.join("map_t.bin").is_file());
        assert_eq!(files_in(&cachedir), ["Saves"]);
        assert_eq!(list(&cachedir).len(), 1);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_taken_name_gets_a_number_and_nothing_is_overwritten() {
        let base = temp("clash");
        let source = base.join("Saves/Sandbox/Run");
        let cachedir = base.join("zomboid");
        save(&source, "first");
        let first = import(&source, &cachedir, |_, _| {}).unwrap();
        std::fs::write(source.join("map_t.bin"), "second").unwrap();
        let second = import(&source, &cachedir, |_, _| {}).unwrap();
        let third = import(&source, &cachedir, |_, _| {}).unwrap();

        assert_eq!(
            [&first.name, &second.name, &third.name],
            ["Run", "Run 2", "Run 3"]
        );
        let clock = |w: &World| std::fs::read_to_string(w.dir.join("map_t.bin")).unwrap();
        assert_eq!(clock(&first), "first");
        assert_eq!(clock(&second), "second");
        assert_eq!(clock(&third), "second");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn refuses_what_is_not_a_solo_save() {
        let base = temp("refuse");
        let cachedir = base.join("zomboid");
        let plain = base.join("Saves/Sandbox/NotASave");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("notes.txt"), "x").unwrap();
        let server = base.join("Saves/Multiplayer/127.0.0.1_16261_abc");
        save(&server, "clock");
        // A save that holds the instance, so importing it would copy the instance too.
        let around = base.join("Saves/Sandbox/Around");
        save(&around, "clock");

        for (source, cachedir) in [
            (base.join("missing"), cachedir.clone()),
            (around.join("map_t.bin"), cachedir.clone()),
            (plain, cachedir.clone()),
            (server, cachedir.clone()),
            (around.clone(), around.join("zomboid")),
        ] {
            let err = import(&source, &cachedir, |_, _| panic!("copied")).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{source:?}");
        }
        assert!(!cachedir.exists(), "a refused import leaves nothing");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_failed_copy_leaves_no_partial_world() {
        let base = temp("fail");
        let source = base.join("Saves/Sandbox/Run");
        save(&source, "clock");
        // A file where the instance's Saves folder should go makes the copy fail.
        let cachedir = base.join("zomboid");
        std::fs::create_dir_all(&cachedir).unwrap();
        std::fs::write(cachedir.join("Saves"), "in the way").unwrap();

        assert!(import(&source, &cachedir, |_, _| {}).is_err());
        assert_eq!(files_in(&cachedir), ["Saves"], "no staging folder is left");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_games_own_saves_folder() {
        assert!(game_saves_dir().is_none_or(|dir| dir.ends_with("Zomboid/Saves")));
    }
}
