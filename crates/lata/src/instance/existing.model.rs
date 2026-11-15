//! The game someone already plays without lata: the folder Project Zomboid keeps in the home
//! (`~/Zomboid`), with its worlds, the mods they load and the player's settings. lata makes
//! instances out of it, so nobody starts over. Everything here only reads that folder.
//!
//! A world decides the instance it can live in: a save of Build 41 doesn't load in Build 42,
//! and it lists the mods it was played with in its own `mods.txt` (the format of
//! `mods/default.txt`). Read from the game's `GameLoadingState` and `WorldConverter`.

use std::io;
use std::path::{Path, PathBuf};

use super::Instance;
use super::mods::{self, Active};
use super::worlds::{self, World};

/// The last world version Build 41 wrote; Build 42 starts at 196 (`IsoWorld.WorldVersion_*`).
const LAST_41: u32 = 195;

/// What comes along besides worlds and mods: options, key bindings and mod options (`Lua`),
/// sandbox presets, hosted server settings and the saved server list.
// ponytail: the cached copies of servers joined (`Saves/Multiplayer`) stay behind; the game
// downloads them again on the next join. Bring them if map notes made there are missed.
const SETTINGS: [&str; 5] = ["options.ini", "Lua", "Sandbox Presets", "Server", "db"];

/// A world found in the game's folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Save {
    pub world: World,
    /// Whether Build 42 or newer saved it.
    pub modern: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Existing {
    pub dir: PathBuf,
    /// The last played first.
    pub worlds: Vec<Save>,
    /// Whether the folder was last used by Build 42 or newer: the build its settings and its
    /// list of active mods belong to.
    pub modern: bool,
    /// The mods on in the game's own mod screen, `mods/default.txt`.
    active: Active,
}

/// An instance to make: the worlds of one build line, and the mods to have on in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub modern: bool,
    pub worlds: Vec<PathBuf>,
    pub active: Active,
}

/// Where the mods a plan loads come from.
#[derive(Debug, Default, PartialEq)]
pub struct Sources {
    /// Workshop items to download, each once.
    pub workshop: Vec<u64>,
    /// Ids found neither in the game's own `mods` folder nor in Steam's Workshop folder.
    pub missing: Vec<String>,
}

/// Where the game keeps a player's things unless told otherwise.
pub fn home() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Zomboid"))
}

/// The game played from `dir`, when there is something worth bringing: a world or active mods.
pub fn find(dir: &Path) -> Option<Existing> {
    let modern = mods::game_version(dir).is_none_or(|version| version.build >= 42);
    let worlds: Vec<Save> = worlds::list(dir)
        .into_iter()
        .map(|world| Save {
            modern: world_version(&world.dir).map_or(modern, |version| version > LAST_41),
            world,
        })
        .collect();
    let active = read_active(&dir.join("mods/default.txt"));
    (!worlds.is_empty() || !active.mods.is_empty()).then(|| Existing {
        dir: dir.to_owned(),
        worlds,
        modern,
        active,
    })
}

/// The version of the game's save format a world was written in: the first four bytes of its
/// `map_ver.bin`, big-endian.
fn world_version(world: &Path) -> Option<u32> {
    use std::io::Read;
    let mut first = [0; 4];
    std::fs::File::open(world.join("map_ver.bin"))
        .ok()?
        .read_exact(&mut first)
        .ok()?;
    Some(u32::from_be_bytes(first))
}

fn read_active(file: &Path) -> Active {
    std::fs::read_to_string(file)
        .map(|text| Active::parse(&text))
        .unwrap_or_default()
}

impl Existing {
    /// The instances to make for the worlds `picked` (one flag per world): one for the build
    /// the folder was last played on, which gets the game's own active mods, and one more
    /// when picked worlds belong to the other build line. Each has on the mods its worlds list.
    pub fn plans(&self, picked: &[bool]) -> Vec<Plan> {
        [self.modern, !self.modern]
            .into_iter()
            .filter_map(|modern| {
                let worlds: Vec<&Save> = self
                    .worlds
                    .iter()
                    .zip(picked)
                    .filter(|(save, picked)| **picked && save.modern == modern)
                    .map(|(save, _)| save)
                    .collect();
                let own = modern == self.modern;
                if worlds.is_empty() && !own {
                    return None;
                }
                let mut active = if own {
                    self.active.clone()
                } else {
                    Active::default()
                };
                for save in &worlds {
                    for id in read_active(&save.world.dir.join("mods.txt")).mods {
                        if !active.mods.contains(&id) {
                            active.mods.push(id);
                        }
                    }
                }
                Some(Plan {
                    modern,
                    worlds: worlds.iter().map(|save| save.world.dir.clone()).collect(),
                    active,
                })
            })
            .collect()
    }

    /// Where to get the mods `plan` loads. The ones in the game's own `mods` folder are copied
    /// with it; the others are looked up in `workshop`, Steam's folders of downloaded items,
    /// to learn which Workshop item each one came from.
    pub fn sources(&self, plan: &Plan, workshop: &[PathBuf]) -> Sources {
        let folders = |dir: &Path| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
        };
        let local: Vec<String> = folders(&self.dir.join("mods"))
            .flat_map(|folder| mods::ids_in(&folder))
            .collect();
        let mut wanted: Vec<&String> = plan
            .active
            .mods
            .iter()
            .filter(|id| !local.contains(id))
            .collect();
        let mut sources = Sources::default();
        for item in workshop.iter().flat_map(|dir| folders(dir)) {
            if wanted.is_empty() {
                break;
            }
            let Some(id) = item
                .file_name()
                .and_then(|name| name.to_string_lossy().parse::<u64>().ok())
            else {
                continue;
            };
            let has: Vec<String> = folders(&item.join("mods"))
                .flat_map(|folder| mods::ids_in(&folder))
                .collect();
            let before = wanted.len();
            wanted.retain(|id| !has.contains(id));
            if wanted.len() < before && !sources.workshop.contains(&id) {
                sources.workshop.push(id);
            }
        }
        sources.missing = wanted.into_iter().cloned().collect();
        sources
    }

    /// Copies what `plan` takes into `instance`: the settings, the mods kept in the game's
    /// own folder and the worlds, then turns the plan's mods on. The source is only read and
    /// nothing in the instance is overwritten by a world. `progress` gets the bytes copied
    /// and the total.
    pub fn copy(
        &self,
        plan: &Plan,
        instance: &Instance,
        mut progress: impl FnMut(u64, u64),
    ) -> io::Result<()> {
        let cachedir = instance.dir.join("zomboid");
        // Settings belong to the build that wrote them; the other one starts with its own.
        let settings: Vec<PathBuf> = SETTINGS
            .iter()
            .filter(|_| plan.modern == self.modern)
            .map(|name| self.dir.join(name))
            .filter(|path| path.exists())
            .collect();
        let local: Vec<PathBuf> = std::fs::read_dir(self.dir.join("mods"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            // The game's own template, which every folder has.
            .filter(|path| path.is_dir() && !path.ends_with("examplemod"))
            .collect();
        let bytes = |path: &PathBuf| {
            if path.is_dir() {
                worlds::size(path)
            } else {
                path.metadata().map(|file| file.len())
            }
        };
        let mut total = 0;
        for path in settings.iter().chain(&local).chain(&plan.worlds) {
            total += bytes(path)?;
        }
        let mut done = 0;
        std::fs::create_dir_all(cachedir.join("mods"))?;
        for (path, into) in settings
            .iter()
            .map(|path| (path, cachedir.clone()))
            .chain(local.iter().map(|path| (path, cachedir.join("mods"))))
        {
            let Some(name) = path.file_name() else {
                continue;
            };
            if path.is_dir() {
                worlds::copy(path, &into.join(name), &mut done, total, &mut progress)?;
            } else {
                done += std::fs::copy(path, into.join(name))?;
                progress(done, total);
            }
        }
        for world in &plan.worlds {
            let before = done;
            worlds::import(world, &cachedir, |copied, _| {
                progress(before + copied, total)
            })?;
            done += worlds::size(world)?;
        }
        mods::activate(instance, &plan.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-existing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf, text: impl AsRef<[u8]>) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// `default.txt` or a save's `mods.txt` with these ids, as Build 42 writes them.
    fn listing(ids: &[&str]) -> String {
        let lines: String = ids
            .iter()
            .map(|id| format!("    mod = \\{id},\n"))
            .collect();
        format!("VERSION = 1,\n\nmods\n{{\n{lines}}}\n\nmaps\n{{\n}}\n")
    }

    /// A save in `home` written at world version `version`, loading `mods`.
    fn world(home: &Path, path: &str, version: u32, mods: &[&str]) {
        let at = home.join("Saves").join(path);
        write(at.join("map_t.bin"), "clock");
        write(at.join("map_ver.bin"), version.to_be_bytes());
        write(at.join("mods.txt"), listing(mods));
    }

    fn instance(root: &Path, name: &str, branch: &str) -> Instance {
        super::super::store::create(root, name, branch).unwrap()
    }

    /// A player on Build 42 with one world of each build line, a mod of their own and two
    /// from the Workshop.
    fn player(home: &Path) -> Existing {
        write(home.join("version.txt"), "42.21.0 4a0e9546ec");
        write(home.join("options.ini"), "zoom=true");
        write(home.join("Lua/keysB42.ini"), "Forward=17");
        write(home.join("console.txt"), "a log, which stays behind");
        write(home.join("mods/default.txt"), listing(&["Mine"]));
        write(home.join("mods/Mine/42/mod.info"), "id=Mine\nname=Mine\n");
        write(home.join("mods/examplemod/mod.info"), "id=examplemod\n");
        world(home, "Apocalypse/Rosewood", 251, &["Mine", "Brita", "Gone"]);
        world(home, "Sandbox/Old days", 195, &["Hydro"]);
        find(home).unwrap()
    }

    /// Steam's Workshop folder with the items of `Brita` (Build 42) and `Hydro` (Build 41).
    fn workshop(root: &Path) -> Vec<PathBuf> {
        let dir = root.join("content/108600");
        write(
            dir.join("2200148440/mods/Brita/42/mod.info"),
            "id=Brita\nname=Brita's Weapons\n",
        );
        write(
            dir.join("2392709985/mods/Hydrocraft/mod.info"),
            "id=Hydro\nname=Hydrocraft\n",
        );
        write(dir.join("not-an-item/mods/x/mod.info"), "id=Gone\n");
        vec![dir]
    }

    #[test]
    fn nothing_to_bring_is_no_game() {
        let home = temp("empty");
        assert_eq!(find(&home), None, "no folder contents at all");
        write(home.join("mods/default.txt"), listing(&[]));
        write(
            home.join("Saves/Multiplayer/1.2.3.4_16261_abc/map_t.bin"),
            "",
        );
        assert_eq!(find(&home), None, "a server's cached copy is not a world");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_world_goes_with_its_build_and_its_mods() {
        let home = temp("plans");
        let game = player(&home);
        assert!(game.modern);
        let builds: Vec<(&str, bool)> = game
            .worlds
            .iter()
            .map(|save| (save.world.name.as_str(), save.modern))
            .collect();
        assert!(builds.contains(&("Rosewood", true)) && builds.contains(&("Old days", false)));

        let plans = game.plans(&[true, true]);
        assert_eq!(plans.len(), 2, "one instance per build line");
        // The game's own list first, then what the world adds, each once.
        assert_eq!(plans[0].active.mods, ["Mine", "Brita", "Gone"]);
        assert_eq!(plans[0].worlds, [home.join("Saves/Apocalypse/Rosewood")]);
        assert!(!plans[1].modern);
        assert_eq!(plans[1].active.mods, ["Hydro"]);

        // Nothing picked still brings the settings and the game's own mods.
        let bare = game.plans(&[false, false]);
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].active.mods, ["Mine"]);
        assert!(bare[0].worlds.is_empty());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn mods_are_traced_to_their_workshop_items() {
        let home = temp("sources");
        let game = player(&home);
        let items = workshop(&home.join("steam"));
        let plans = game.plans(&[true, true]);
        assert_eq!(
            game.sources(&plans[0], &items),
            Sources {
                workshop: vec![2200148440],
                missing: vec!["Gone".into()],
            },
            "Mine is local, Brita is an item, Gone is nowhere"
        );
        assert_eq!(game.sources(&plans[1], &items).workshop, [2392709985]);
        assert_eq!(game.sources(&plans[1], &[]).missing, ["Hydro"]);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn copies_settings_local_mods_and_worlds() {
        let home = temp("copy");
        let game = player(&home);
        let root = home.join("lata");
        let plans = game.plans(&[true, true]);
        let (new, old) = (
            instance(&root, "Mine", "public"),
            instance(&root, "Old", "legacy41"),
        );
        let mut seen = Vec::new();
        game.copy(&plans[0], &new, |done, total| seen.push((done, total)))
            .unwrap();
        game.copy(&plans[1], &old, |_, _| {}).unwrap();

        let at = new.dir.join("zomboid");
        assert_eq!(
            std::fs::read_to_string(at.join("options.ini")).unwrap(),
            "zoom=true"
        );
        assert!(at.join("Lua/keysB42.ini").is_file());
        assert!(at.join("mods/Mine/42/mod.info").is_file());
        assert!(at.join("Saves/Apocalypse/Rosewood/map_ver.bin").is_file());
        assert!(!at.join("console.txt").exists(), "logs stay behind");
        assert!(!at.join("mods/examplemod").exists());
        assert!(
            !at.join("Saves/Sandbox").exists(),
            "the other build's world"
        );
        assert_eq!(
            std::fs::read_to_string(at.join("mods/default.txt")).unwrap(),
            listing(&["Mine", "Brita", "Gone"])
        );
        let (done, total) = *seen.last().unwrap();
        assert!(total > 0 && done == total, "progress ends at the total");
        assert!(seen.windows(2).all(|pair| pair[0].0 <= pair[1].0));

        // Build 41 gets its world and its list in its own format, not Build 42's settings.
        let at = old.dir.join("zomboid");
        assert!(at.join("Saves/Sandbox/Old days/map_t.bin").is_file());
        assert!(!at.join("options.ini").exists());
        let list = std::fs::read_to_string(at.join("mods/default.txt")).unwrap();
        assert!(list.contains("mod = Hydro,"), "{list}");
        assert!(at.join("mods/reset-mods-41_51.txt").is_file());
        // The source is only read.
        assert!(home.join("Saves/Apocalypse/Rosewood/mods.txt").is_file());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
