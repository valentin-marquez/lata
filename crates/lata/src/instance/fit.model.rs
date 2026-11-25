//! Whether a version of a Workshop item runs on an instance's game, told from that version's
//! file list and its `mod.info` files alone, without downloading the mod.
//!
//! The folders say what the game would load (Build 41 reads `mods/<folder>/mod.info`, Build 42
//! the highest version folder not above the game, or `common/`; see [`super::mods`]), and Build
//! 42's `mod.info` says which games the mod is made for (`versionMin=`, `versionMax=`). Nine
//! in ten popular Build 42 mods set `versionMin` (checked on 2026-11-17), and it is often
//! the only place a `42/` folder says it needs 42.21.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::mods::Version;

/// One `mod.info` in a version of an item: where it is (`mods/<folder>/42/mod.info`) and the
/// game versions it asks for, as written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

/// Whether a version runs on a game, and if not, what it is made for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    Yes,
    /// Made for this game version or newer.
    Needs(Version),
    /// Made for games up to this version.
    UpTo(Version),
    /// Only Build 41's layout: a Build 42 game doesn't load it.
    Build41,
    /// Only Build 42's layout: Build 41 doesn't load it.
    Build42,
}

impl Fit {
    /// What the version is made for, said in a tag; `None` when it fits.
    pub fn label(&self) -> Option<String> {
        let v = |v: &Version| format!("{}.{}", v.build, v.major);
        match self {
            Fit::Yes => None,
            Fit::Needs(min) => Some(t!("Needs {version}+", version = v(min))),
            Fit::UpTo(max) => Some(t!("Up to {version}", version = v(max))),
            Fit::Build41 => Some(t!("Build 41 only").into()),
            Fit::Build42 => Some(t!("Build 42 only").into()),
        }
    }
}

/// The game versions a `mod.info` asks for (`versionMin=42.21.0`); an empty value is none.
pub fn asks(text: &str) -> (Option<String>, Option<String>) {
    let mut found = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "versionmin" => found.0 = Some(value.to_owned()),
            "versionmax" => found.1 = Some(value.to_owned()),
            _ => {}
        }
    }
    found
}

/// Whether the version whose `mod.info` files are `infos` runs on `game`. It fits when any of
/// its mods does: an item can carry variants of which a player turns on one. A version with
/// no `mod.info` at all says nothing, so it isn't held against it.
pub fn fit(infos: &[Info], game: Version) -> Fit {
    let mut folders: Vec<&str> = infos
        .iter()
        .filter_map(|info| info.path.strip_prefix("mods/")?.split('/').next())
        .collect();
    folders.sort_unstable();
    folders.dedup();
    if folders.is_empty() {
        return Fit::Yes;
    }
    let verdicts: Vec<Fit> = folders
        .iter()
        .map(|folder| fit_folder(infos, folder, game))
        .collect();
    if verdicts.contains(&Fit::Yes) {
        return Fit::Yes;
    }
    // The nearest miss says the most: the lowest game a variant needs, else what there is.
    verdicts
        .iter()
        .filter_map(|v| match v {
            Fit::Needs(min) => Some(*min),
            _ => None,
        })
        .min()
        .map(Fit::Needs)
        .unwrap_or(verdicts[0])
}

fn fit_folder(infos: &[Info], folder: &str, game: Version) -> Fit {
    let prefix = format!("mods/{folder}/");
    // `None` is the mod's own folder (Build 41), `Some` a folder inside it.
    let inside: Vec<(Option<&str>, &Info)> = infos
        .iter()
        .filter_map(|info| {
            let rest = info.path.strip_prefix(&prefix)?;
            match rest.strip_suffix("/mod.info") {
                Some(sub) if !sub.contains('/') => Some((Some(sub), info)),
                None if rest == "mod.info" => Some((None, info)),
                _ => None,
            }
        })
        .collect();
    if game.build < 42 {
        return if inside.iter().any(|(sub, _)| sub.is_none()) {
            Fit::Yes
        } else {
            Fit::Build42
        };
    }
    let floor = Version {
        build: 42,
        major: 0,
    };
    let mut versions: Vec<(Version, &Info)> = inside
        .iter()
        .filter_map(|(sub, info)| Some((Version::parse((*sub)?)?, *info)))
        .filter(|(v, _)| *v >= floor)
        .collect();
    versions.sort_by_key(|(v, _)| *v);
    let chosen = versions
        .iter()
        .rev()
        .find(|(v, _)| *v <= game)
        .map(|(_, info)| *info)
        .or_else(|| {
            inside
                .iter()
                .find(|(sub, _)| *sub == Some("common"))
                .map(|(_, info)| *info)
        });
    let Some(info) = chosen else {
        return match versions.first() {
            Some((lowest, _)) => Fit::Needs(*lowest),
            None => Fit::Build41,
        };
    };
    let min = info.min.as_deref().and_then(Version::parse);
    let max = info.max.as_deref().and_then(Version::parse);
    match (min, max) {
        (Some(min), _) if min > game => Fit::Needs(min),
        (_, Some(max)) if max < game => Fit::UpTo(max),
        _ => Fit::Yes,
    }
}

/// Where what a version's `mod.info` files say is kept. A published version never changes,
/// so each is asked once, named by its manifest.
fn cached(manifest: u64) -> PathBuf {
    super::store::data()
        .join("cache")
        .join("workshop-fit")
        .join(format!("{manifest}.json"))
}

/// The `mod.info` files of version `manifest` of Workshop item `item`, from the cache or
/// Steam: its file list, then only those few small files.
pub async fn infos(
    client: &mut steam::Client,
    http: &steam::reqwest::Client,
    source: &steam::workshop::Source,
    item: &steam::workshop::Item,
    manifest: u64,
) -> Result<Vec<Info>, steam::Error> {
    let path = cached(manifest);
    if let Some(found) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        return Ok(found);
    }
    let one = steam::workshop::Item {
        manifest,
        ..item.clone()
    };
    let mut files = steam::workshop::manifest(client, http, source, &one).await?;
    files.files.retain(|file| {
        !file.is_dir
            && file
                .path
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with("/mod.info")
    });
    let dir = std::env::temp_dir().join(format!("lata-fit-{manifest}"));
    steam::workshop::fetch(http, source, &files, &dir, |_| ()).await?;
    let found: Vec<Info> = files
        .files
        .iter()
        .map(|file| {
            let text = std::fs::read_to_string(dir.join(&file.path)).unwrap_or_default();
            let (min, max) = asks(&text);
            Info {
                path: file.path.replace('\\', "/"),
                min,
                max,
            }
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_ok()
        && let Ok(text) = serde_json::to_string(&found)
    {
        let _ = std::fs::write(&path, text);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, min: Option<&str>, max: Option<&str>) -> Info {
        Info {
            path: path.into(),
            min: min.map(Into::into),
            max: max.map(Into::into),
        }
    }

    fn game(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn reads_what_a_mod_info_asks_for() {
        let text = "name=Sandbox Control\nmodversion=1.5.5\nversionMin=42.21.0\nversionMax=\n";
        assert_eq!(asks(text), (Some("42.21.0".into()), None));
        assert_eq!(asks("id=x\n"), (None, None));
    }

    #[test]
    fn a_42_folder_asking_for_a_newer_game_needs_it() {
        // Sandbox Control: one `42/` folder that any 42.x loads, and versionMin says 42.21.
        let infos = [at("mods/SandboxControl/42/mod.info", Some("42.21.0"), None)];
        assert_eq!(fit(&infos, game("42.22.2")), Fit::Yes);
        assert_eq!(fit(&infos, game("42.13")), Fit::Needs(game("42.21")));
        assert_eq!(fit(&infos, game("41.78")), Fit::Build42);
    }

    #[test]
    fn the_folder_the_game_picks_is_the_one_that_counts() {
        let infos = [
            at("mods/M/42.0/mod.info", None, None),
            at("mods/M/42.21/mod.info", Some("42.21"), None),
        ];
        assert_eq!(fit(&infos, game("42.13")), Fit::Yes);
        assert_eq!(fit(&infos, game("42.22")), Fit::Yes);
        let only_new = [at("mods/M/42.21/mod.info", None, None)];
        assert_eq!(fit(&only_new, game("42.13")), Fit::Needs(game("42.21")));
    }

    #[test]
    fn common_counts_and_a_max_can_rule_a_version_out() {
        let infos = [at("mods/M/common/mod.info", Some("42.0"), Some("42.13"))];
        assert_eq!(fit(&infos, game("42.13")), Fit::Yes);
        assert_eq!(fit(&infos, game("42.22")), Fit::UpTo(game("42.13")));
    }

    #[test]
    fn build_41_and_42_layouts_split_the_builds() {
        let old = [at("mods/M/mod.info", None, None)];
        assert_eq!(fit(&old, game("41.78")), Fit::Yes);
        assert_eq!(fit(&old, game("42.22")), Fit::Build41);
        // SpiffBoy ships both layouts, so both builds load it.
        let both = [
            at("mods/S/mod.info", Some("42.0"), None),
            at("mods/S/common/mod.info", Some("42.0"), None),
        ];
        assert_eq!(fit(&both, game("41.78")), Fit::Yes);
        assert_eq!(fit(&both, game("42.22")), Fit::Yes);
    }

    #[test]
    fn one_variant_that_runs_is_enough_and_nothing_known_is_no_verdict() {
        let infos = [
            at("mods/Old/mod.info", None, None),
            at("mods/New/42/mod.info", Some("42.20"), None),
        ];
        assert_eq!(fit(&infos, game("42.22")), Fit::Yes);
        assert_eq!(fit(&infos, game("42.13")), Fit::Needs(game("42.20")));
        assert_eq!(fit(&[], game("42.22")), Fit::Yes);
    }

    /// Asks Steam about the newest version of Sandbox Control (3825312974), whose `42/` folder
    /// says `versionMin=42.21.0`. Writes to lata's cache, so point `LATA_HOME` at a scratch
    /// folder: `LATA_HOME=/tmp/x cargo test -p lata -- --ignored fit`.
    #[tokio::test]
    #[ignore = "needs network access to Steam"]
    async fn tells_from_steam_which_game_a_version_needs() {
        let http = steam::http();
        let mut client = steam::Client::anonymous(&http).await.unwrap();
        let item = steam::workshop::details(&mut client, &[3_825_312_974])
            .await
            .unwrap()
            .remove(0);
        let source = steam::workshop::source(&mut client, super::super::APP)
            .await
            .unwrap();
        let found = infos(&mut client, &http, &source, &item, item.manifest)
            .await
            .unwrap();
        assert!(found.iter().any(|info| info.path.ends_with("/42/mod.info")));
        assert_eq!(fit(&found, game("42.13")), Fit::Needs(game("42.21")));
        assert_eq!(fit(&found, game("42.22")), Fit::Yes);
    }

    #[test]
    fn labels_say_what_a_version_is_made_for() {
        assert_eq!(
            Fit::Needs(game("42.21")).label().as_deref(),
            Some("Needs 42.21+")
        );
        assert_eq!(
            Fit::UpTo(game("42.13")).label().as_deref(),
            Some("Up to 42.13")
        );
        assert_eq!(Fit::Build41.label().as_deref(), Some("Build 41 only"));
        assert_eq!(Fit::Yes.label(), None);
    }
}
