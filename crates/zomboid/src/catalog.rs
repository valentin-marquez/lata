//! Every game build lata has seen, with its version and the manifest of each depot, so a build
//! can still be named and downloaded after its branch moves on. Steam keeps no history: PICS
//! only lists each branch's current build, so a build nobody recorded while it was current is
//! lost. A scheduled job records them into `versions.json`, which is embedded here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    /// Newest first.
    pub builds: Vec<Build>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Build {
    /// The game's build id (app 108600). `None` for a build added from its manifests after it
    /// left its branch: Steam only tells the build id of what is current.
    pub build: Option<u32>,
    /// "42.22.2", or `None` while unknown. Read from the dedicated server's `Core.class` on the
    /// same branch at the same time, so it can be wrong if the two didn't ship together; an
    /// installed build's own files win over it.
    pub version: Option<String>,
    /// Branches it was seen on.
    pub branches: Vec<String>,
    /// When it was first seen (or, for a build added later, when Steam built it), in seconds
    /// since the epoch.
    pub seen: u64,
    /// The manifest of each depot (108602 macOS, 108603 Linux, 108604 Windows).
    pub manifests: BTreeMap<u32, u64>,
}

/// The catalog this binary was built with.
pub fn embedded() -> Catalog {
    parse(include_str!("../versions.json")).unwrap_or_default()
}

pub fn parse(text: &str) -> serde_json::Result<Catalog> {
    serde_json::from_str(text)
}

impl Catalog {
    pub fn get(&self, build: u32) -> Option<&Build> {
        self.builds.iter().find(|b| b.build == Some(build))
    }

    /// The build that `manifest` of `depot` belongs to.
    pub fn of(&self, depot: u32, manifest: u64) -> Option<&Build> {
        self.builds
            .iter()
            .find(|b| b.manifests.get(&depot) == Some(&manifest))
    }

    /// Records `seen`: adds it, or adds the branch, a missing build id, version or depot to the
    /// one already there (the same build id, or sharing a manifest). Returns whether anything
    /// changed.
    pub fn record(&mut self, seen: Build) -> bool {
        let same = |b: &Build| {
            (b.build.is_some() && b.build == seen.build)
                || seen
                    .manifests
                    .iter()
                    .any(|(depot, manifest)| b.manifests.get(depot) == Some(manifest))
        };
        let Some(known) = self.builds.iter_mut().find(|b| same(b)) else {
            self.builds.push(seen);
            self.builds
                .sort_by(|a, b| b.seen.cmp(&a.seen).then(b.build.cmp(&a.build)));
            return true;
        };
        let mut changed = false;
        for branch in seen.branches {
            if !known.branches.contains(&branch) {
                known.branches.push(branch);
                changed = true;
            }
        }
        if known.build.is_none() && seen.build.is_some() {
            known.build = seen.build;
            changed = true;
        }
        if known.version.is_none() && seen.version.is_some() {
            known.version = seen.version;
            changed = true;
        }
        for (depot, manifest) in seen.manifests {
            if known.manifests.insert(depot, manifest) != Some(manifest) {
                changed = true;
            }
        }
        changed
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(id: u32, branch: &str, seen: u64) -> Build {
        Build {
            build: Some(id),
            version: None,
            branches: vec![branch.into()],
            seen,
            manifests: BTreeMap::from([(108603, u64::from(id))]),
        }
    }

    #[test]
    fn records_new_builds_and_what_is_new_about_known_ones() {
        let mut catalog = Catalog::default();
        assert!(catalog.record(build(1, "public", 10)));
        assert!(catalog.record(build(2, "public", 20)));
        assert_eq!(catalog.builds[0].build, Some(2), "newest first");
        assert!(!catalog.record(build(2, "public", 30)), "nothing new");
        assert!(catalog.record(build(2, "pre-release", 30)));
        let mut named = build(2, "public", 30);
        named.version = Some("42.22.2".into());
        assert!(catalog.record(named));
        let known = catalog.get(2).unwrap();
        assert_eq!(known.branches, ["public", "pre-release"]);
        assert_eq!(known.version.as_deref(), Some("42.22.2"));
        assert_eq!(known.seen, 20, "first seen stays");
        assert_eq!(parse(&catalog.to_json()).unwrap(), catalog);
    }

    #[test]
    fn a_build_without_an_id_is_known_by_its_manifests() {
        let mut catalog = Catalog::default();
        let mut old = build(0, "unstable", 5);
        old.build = None;
        old.manifests = BTreeMap::from([(108604, 77)]);
        assert!(catalog.record(old.clone()));
        assert!(
            !catalog.record(old.clone()),
            "the same manifests, nothing new"
        );
        // Its other depots and its build id arrive later and land on the same entry.
        old.build = Some(9);
        old.manifests.insert(108603, 78);
        assert!(catalog.record(old));
        assert_eq!(catalog.builds.len(), 1);
        assert_eq!(catalog.get(9).unwrap().manifests.len(), 2);
        assert_eq!(catalog.of(108603, 78).unwrap().build, Some(9));
    }

    #[test]
    fn the_embedded_catalog_parses() {
        let text = include_str!("../versions.json");
        parse(text).unwrap();
    }
}
