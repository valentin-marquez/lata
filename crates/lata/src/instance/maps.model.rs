//! The maps an instance's active mods bring, in the order the game loads them, and the ones
//! that fight over the same cells. A port of the game's `MapGroups` (Build 42.21), as its mod
//! screen uses it: only the mods' maps, not the game's own.
//!
//! A map is a folder with a `map.info` under a mod's `media/maps/`. Its `lots=` lines name
//! other maps it builds on, which ties them into a group; inside a group, two maps that both
//! have the same `.lotheader` file (one per cell) conflict, and the one that comes first wins
//! that cell. That order is the `maps { }` block of `mods/default.txt`.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use super::mods::Mod;

#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    /// Its folder's name, which is what `lots=` lines and the `maps` block call it.
    pub name: String,
    pub dir: PathBuf,
    /// The maps its `map.info` builds on.
    lots: Vec<String>,
    /// The other maps that have a cell this one has too.
    pub conflicts: Vec<String>,
}

/// The maps of `mods` (the active ones, in load order) as the game orders them: grouped by
/// what they build on, a map ahead of the ones it names, and then as `wanted` says, the
/// `maps` block someone already arranged. `modern` is whether the game is Build 42 or newer.
pub fn in_order(mods: &[&Mod], modern: bool, wanted: &[String]) -> Vec<Map> {
    let mut real = found(mods, modern);
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for ix in 0..real.len() {
        let mut tied = Vec::new();
        reach(&real, ix, &mut tied);
        let at = match groups
            .iter()
            .position(|group| tied.iter().any(|map| group.contains(map)))
        {
            Some(at) => at,
            None => {
                groups.push(Vec::new());
                groups.len() - 1
            }
        };
        for map in tied {
            if !groups[at]
                .iter()
                .any(|have| real[*have].name == real[map].name)
            {
                groups[at].push(map);
            }
        }
    }
    for group in &mut groups {
        for map in group.clone() {
            let named =
                |other: &usize| *other != map && real[*other].lots.contains(&real[map].name);
            if !group.iter().any(named) {
                let mut first = Vec::new();
                reach_names(&real, group, map, &mut first);
                prioritize(&real, group, &first);
            }
        }
        if !wanted.is_empty() {
            prioritize(&real, group, wanted);
        }
        // Cells two maps of the group both have, by the name of the cell's file.
        let mut cells: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for map in group.iter() {
            for cell in lotheaders(&real[*map].dir) {
                cells.entry(cell).or_default().push(*map);
            }
        }
        for sharing in cells.values().filter(|maps| maps.len() > 1) {
            for (map, other) in sharing
                .iter()
                .flat_map(|map| sharing.iter().map(move |other| (*map, *other)))
                .filter(|(map, other)| map != other)
            {
                let name = real[other].name.clone();
                if !real[map].conflicts.contains(&name) {
                    real[map].conflicts.push(name);
                }
            }
        }
    }
    groups
        .into_iter()
        .flatten()
        .map(|ix| real[ix].clone())
        .collect()
}

/// Every map folder of `mods`, mod by mod. Build 42 reads a mod's `common/` and then its
/// version folder, and skips the mod when `common/` has no maps; Build 41 has one folder.
// ponytail: `Mod::version` is the folder its `mod.info` sits in, so a Build 42 mod with
// `mod.info` in `common/` and maps only in a version folder shows none here. Read the version
// folder the game picks if such a mod turns up.
fn found(mods: &[&Mod], modern: bool) -> Vec<Map> {
    let mut real = Vec::new();
    for m in mods {
        let folders = if modern {
            let common = m.dir.join("common/media/maps");
            if !common.is_dir() {
                continue;
            }
            let version = m.version.as_ref().map(|v| m.dir.join(v).join("media/maps"));
            std::iter::once(common).chain(version).collect()
        } else {
            vec![m.dir.join("media/maps")]
        };
        for folder in folders {
            let mut names: Vec<String> = std::fs::read_dir(&folder)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !name.eq_ignore_ascii_case("challengemaps"))
                .collect();
            // The game takes them as the system lists them; sorted, the answer holds still.
            names.sort();
            for name in names {
                let dir = folder.join(&name);
                let Ok(info) = std::fs::read_to_string(dir.join("map.info")) else {
                    continue;
                };
                let lots = info
                    .lines()
                    .filter_map(|line| line.trim().strip_prefix("lots="))
                    .map(|lot| lot.trim().to_owned())
                    .collect();
                real.push(Map {
                    name,
                    dir,
                    lots,
                    conflicts: Vec::new(),
                });
            }
        }
    }
    real
}

/// `start` and every map it builds on, through `lots=`, among all the maps found.
fn reach(real: &[Map], start: usize, out: &mut Vec<usize>) {
    if out.contains(&start) {
        return;
    }
    out.push(start);
    for lot in &real[start].lots {
        if let Some(next) = real.iter().position(|map| map.name == *lot) {
            reach(real, next, out);
        }
    }
}

/// The names of `start` and what it builds on inside `group`, itself first.
fn reach_names(real: &[Map], group: &[usize], start: usize, out: &mut Vec<String>) {
    if out.contains(&real[start].name) {
        return;
    }
    out.push(real[start].name.clone());
    for lot in &real[start].lots {
        if let Some(next) = group.iter().find(|map| real[**map].name == *lot) {
            reach_names(real, group, *next, out);
        }
    }
}

/// Puts the maps of `group` that `names` lists in that order, in the places they held; the
/// others stay where they are.
fn prioritize(real: &[Map], group: &mut [usize], names: &[String]) {
    let mut sorted: VecDeque<usize> = VecDeque::new();
    for name in names {
        if let Some(map) = group.iter().find(|map| real[**map].name == *name)
            && !sorted.contains(map)
        {
            sorted.push_back(*map);
        }
    }
    for place in group.iter_mut() {
        if names.contains(&real[*place].name)
            && let Some(next) = sorted.pop_front()
        {
            *place = next;
        }
    }
}

fn lotheaders(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".lotheader"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-maps-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Build 42 mod in `root` with these maps: name, what it builds on, and its cells.
    fn map_mod(root: &Path, id: &str, maps: &[(&str, &[&str], &[&str])]) -> Mod {
        let dir = root.join(id);
        for (name, lots, cells) in maps {
            let at = dir.join("common/media/maps").join(name);
            std::fs::create_dir_all(&at).unwrap();
            let info: String = lots.iter().map(|lot| format!("lots={lot}\n")).collect();
            std::fs::write(at.join("map.info"), format!("title={name}\n{info}")).unwrap();
            for cell in *cells {
                std::fs::write(at.join(format!("{cell}.lotheader")), "").unwrap();
            }
        }
        Mod {
            id: id.into(),
            name: id.into(),
            author: String::new(),
            description: String::new(),
            folder: id.into(),
            dir,
            version: None,
            workshop: None,
            manifest: None,
            held: false,
            java: false,
            icon: None,
            requires: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            enabled: true,
        }
    }

    fn names(maps: &[Map]) -> Vec<&str> {
        maps.iter().map(|map| map.name.as_str()).collect()
    }

    #[test]
    fn maps_that_build_on_each_other_conflict_over_shared_cells() {
        let root = temp("conflict");
        let town = map_mod(
            &root,
            "town",
            &[("Town", &["Muldraugh, KY"], &["35_30", "35_31"])],
        );
        let patch = map_mod(
            &root,
            "patch",
            &[("TownPatch", &["Town"], &["35_31", "36_31"])],
        );
        let far = map_mod(&root, "far", &[("Island", &[], &["35_31"])]);
        std::fs::create_dir_all(root.join("far/common/media/maps/challengemaps")).unwrap();
        std::fs::create_dir_all(root.join("far/common/media/maps/NoInfo")).unwrap();

        let maps = in_order(&[&town, &patch, &far], true, &[]);
        // The patch names the town, so it goes ahead of it and wins the cell they share.
        assert_eq!(names(&maps), ["TownPatch", "Town", "Island"]);
        assert_eq!(maps[0].conflicts, ["Town"]);
        assert_eq!(maps[1].conflicts, ["TownPatch"]);
        // The island has that cell too, but nothing ties it to them: the game doesn't compare.
        assert!(maps[2].conflicts.is_empty());

        // An order someone arranged holds, and maps it doesn't name stay put.
        let wanted = ["Town".to_owned(), "TownPatch".to_owned(), "Gone".to_owned()];
        assert_eq!(
            names(&in_order(&[&town, &patch, &far], true, &wanted)),
            ["Town", "TownPatch", "Island"]
        );
        assert!(in_order(&[], true, &[]).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_maps_where_each_build_keeps_them() {
        let root = temp("layout");
        // Build 42: `common/` and the version folder; nothing without maps in `common/`.
        let mut both = map_mod(&root, "both", &[("Base", &[], &[])]);
        both.version = Some("42".into());
        let extra = root.join("both/42/media/maps/Extra");
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::write(extra.join("map.info"), "lots=Base\n").unwrap();
        let bare = root.join("bare/42/media/maps/Lonely");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("map.info"), "").unwrap();
        let mut lonely = both.clone();
        lonely.dir = root.join("bare");
        assert_eq!(
            names(&in_order(&[&both, &lonely], true, &[])),
            ["Extra", "Base"]
        );

        // Build 41: the mod's own `media/maps`.
        let old = root.join("old/media/maps/Bedford");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("map.info"), "lots=Muldraugh, KY\n").unwrap();
        let mut legacy = both.clone();
        legacy.dir = root.join("old");
        assert_eq!(names(&in_order(&[&legacy], false, &[])), ["Bedford"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
