//! What a game build is called. Players say 42.22.2; Steam only has build ids. Each complete
//! build folder keeps a `build.json` (the catalog's `Build` shape) with the version read from its
//! own files and the manifest of every OS depot, so the build can be named and found again.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use steam::pics::Branch;
use zomboid::catalog::{self, Build, Catalog};

const FILE: &str = "build.json";

static CATALOG: LazyLock<Catalog> = LazyLock::new(catalog::embedded);
/// Version by build id, so a render never opens a jar or walks a folder twice. A build whose
/// files had no version is kept as `None` too; one with no folder to look in isn't, since it
/// may be installed later.
static NAMES: Mutex<Option<HashMap<u32, Option<String>>>> = Mutex::new(None);

pub fn read(game: &Path) -> Option<Build> {
    serde_json::from_slice(&std::fs::read(game.join(FILE)).ok()?).ok()
}

/// Writes the record and remembers its version for this run.
pub fn record(game: &Path, build: &Build) -> io::Result<()> {
    let text = serde_json::to_string_pretty(build).map_err(io::Error::other)?;
    std::fs::write(game.join(FILE), text + "\n")?;
    if let Some(version) = &build.version {
        remember(build.build, Some(version.clone()));
    }
    Ok(())
}

fn remember(build: u32, version: Option<String>) {
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    names
        .get_or_insert_with(Default::default)
        .insert(build, version);
}

/// The version of the game files under `game`: the jar, or a loose `Core.class`, in the root or
/// a folder or two down (Linux nests it, the server keeps it under `java/`).
pub fn read_files(game: &Path) -> Option<String> {
    let mut dirs = vec![game.to_owned()];
    let mut next = Vec::new();
    for _ in 0..3 {
        for dir in &dirs {
            for source in ["projectzomboid.jar", "zombie/core/Core.class"] {
                if let Some(version) = zomboid::version::read(&dir.join(source)) {
                    return Some(version.to_string());
                }
            }
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                // media/ is the bulk of the game and never holds the jar.
                if entry.path().is_dir() && entry.file_name() != "media" {
                    next.push(entry.path());
                }
            }
        }
        dirs = std::mem::take(&mut next);
    }
    None
}

/// The version of a build: remembered, else its record, else its files (written to the record
/// then, for builds installed before records existed), else the catalog.
pub fn version(build: u32, game: Option<&Path>) -> Option<String> {
    let cached = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = cached.as_ref().and_then(|n| n.get(&build)) {
        return known
            .clone()
            .or_else(|| CATALOG.get(build)?.version.clone());
    }
    drop(cached);
    if let Some(game) = game.filter(|g| g.is_dir()) {
        let mut found = read(game);
        if found.as_ref().is_none_or(|b| b.version.is_none())
            && let Some(version) = read_files(game)
        {
            let mut rec = found.take().unwrap_or(Build {
                build,
                version: None,
                branches: Vec::new(),
                seen: super::store::now(),
                manifests: BTreeMap::new(),
            });
            rec.version = Some(version);
            let _ = record(game, &rec);
            found = Some(rec);
        }
        let version = found.and_then(|b| b.version);
        remember(build, version.clone());
        if version.is_some() {
            return version;
        }
    }
    CATALOG.get(build)?.version.clone()
}

/// "Build 42.19.2" in a branch description is the version; other descriptions aren't.
pub fn described(description: &str) -> Option<String> {
    let text = description.trim().strip_prefix("Build ")?;
    text.parse::<zomboid::Version>().ok().map(|v| v.to_string())
}

/// What to call a build: its version, else what the branch description says when `branch` is
/// where it is current, else the build id.
pub fn name(build: u32, game: Option<&Path>, branch: Option<&Branch>) -> String {
    version(build, game)
        .or_else(|| {
            let branch = branch.filter(|b| b.build_id == build)?;
            described(branch.description.as_deref()?)
        })
        .unwrap_or_else(|| format!("Build {build}"))
}

/// The update channel a branch is: the part of its name players care about.
pub fn channel(branch: &str) -> String {
    match branch {
        "public" => "Stable".into(),
        "pre-release" => "Pre-release".into(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(build_id: u32, description: &str) -> Branch {
        Branch {
            name: "x".into(),
            build_id,
            description: Some(description.into()),
            password_required: false,
            updated: None,
        }
    }

    #[test]
    fn writes_and_reads_the_record() {
        let dir = std::env::temp_dir().join(format!("lata-build-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let build = Build {
            build: 7,
            version: Some("42.22.2".into()),
            branches: vec!["public".into()],
            seen: 1,
            manifests: BTreeMap::from([(108603, 9)]),
        };
        record(&dir, &build).unwrap();
        assert_eq!(read(&dir), Some(build));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_an_old_install_from_its_files_once_and_records_it() {
        let dir = std::env::temp_dir().join(format!("lata-old-{}", std::process::id()));
        let class = dir.join("projectzomboid/zombie/core");
        std::fs::create_dir_all(&class).unwrap();
        std::fs::write(
            class.join("Core.class"),
            include_bytes!("../../../zomboid/src/testdata/core-42.22.2.class"),
        )
        .unwrap();
        assert_eq!(version(424242, Some(&dir)).as_deref(), Some("42.22.2"));
        assert_eq!(read(&dir).unwrap().version.as_deref(), Some("42.22.2"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn looks_in_a_folder_without_a_version_only_once() {
        let dir = std::env::temp_dir().join(format!("lata-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(version(434343, Some(&dir)), None);
        // Found later only through a record (an install), never by walking the folder again.
        std::fs::write(
            dir.join("projectzomboid.jar"),
            include_bytes!("../../../zomboid/src/testdata/core-42.22.2.class"),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("zombie/core")).unwrap();
        std::fs::write(
            dir.join("zombie/core/Core.class"),
            include_bytes!("../../../zomboid/src/testdata/core-42.22.2.class"),
        )
        .unwrap();
        assert_eq!(version(434343, Some(&dir)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_a_build_by_version_then_description_then_id() {
        let now = branch(5, "Build 41.78.21");
        assert_eq!(name(5, None, Some(&now)), "41.78.21");
        // The description is about the branch's current build only.
        assert_eq!(name(4, None, Some(&now)), "Build 4");
        assert_eq!(name(5, None, Some(&branch(5, "Stable"))), "Build 5");
        assert_eq!(name(6, None, None), "Build 6");
        remember(8, Some("42.1.0".into()));
        assert_eq!(name(8, None, Some(&now)), "42.1.0");
    }

    #[test]
    fn channels_read_plainly() {
        assert_eq!(channel("public"), "Stable");
        assert_eq!(channel("pre-release"), "Pre-release");
        assert_eq!(channel("legacy41"), "legacy41");
    }
}
