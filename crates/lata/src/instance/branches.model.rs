//! Project Zomboid's Steam branches (the game versions an instance can pin), fetched live.
//!
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use super::build::{channel, described};
pub use steam::pics::Branch;

/// A game version the create dialog offers, and how an instance gets it.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// "42.22.2", or "Build 26486631" when the version isn't known.
    pub name: String,
    /// The channel it is listed under: "Stable", "Pre-release", "Unstable", "Build 41"...
    pub channel: String,
    /// The branch the instance follows.
    pub branch: String,
    /// Set when the version is no longer current on `branch`: the build to install instead.
    pub target: Option<super::store::Target>,
    /// The newest of its channel.
    pub latest: bool,
    /// When it came out (or was first seen), in seconds since the epoch.
    pub date: Option<u64>,
}

/// The channel a branch belongs to. Numbered branches (`42.21`, `legacy_42_12`) are Steam's
/// fallbacks to an older stable version, so they list under Stable.
fn group(branch: &str) -> String {
    match branch {
        "public" | "pre-release" | "unstable" => channel(branch),
        b if b.contains("41") => "Build 41".into(),
        b if b.chars().any(|c| c.is_ascii_digit()) => "Stable".into(),
        other => channel(other),
    }
}

/// Whether `branch` is one of Steam's numbered fallbacks to an older stable version.
pub fn fallback(branch: &str) -> bool {
    group(branch) == "Stable" && branch != "public"
}

/// Whether a choice is Build 42 or newer: by its version when known, else by its branch.
pub fn modern(choice: &Choice) -> bool {
    match choice.name.parse::<zomboid::Version>() {
        Ok(version) => version.major >= 42,
        Err(_) => super::mods::branch_is_modern(&choice.branch),
    }
}

/// Every version that can be installed, by channel (Stable first), newest first: the current
/// build of each live branch, and the builds that were once on `public`, which Steam still
/// serves. Builds that only ever lived on a beta branch that moved on are left out: Steam
/// refuses them.
pub fn choices(branches: &[Branch]) -> Vec<Choice> {
    choices_in(super::build::catalog(), branches)
}

fn choices_in(catalog: &zomboid::catalog::Catalog, branches: &[Branch]) -> Vec<Choice> {
    let mut list: Vec<(Choice, u32)> = Vec::new();
    // `public` first, so a numbered fallback on the same build doesn't take its place.
    let mut branches: Vec<&Branch> = branches.iter().collect();
    branches.sort_by_key(|b| b.name != "public");
    for branch in branches {
        let channel = group(&branch.name);
        // Two branches on one build in one channel (public and its numbered fallback): once.
        if list
            .iter()
            .any(|(c, id)| c.channel == channel && *id == branch.build_id)
        {
            continue;
        }
        list.push((
            Choice {
                name: catalog
                    .get(branch.build_id)
                    .and_then(|b| b.version.clone())
                    .unwrap_or_else(|| super::build::name(branch.build_id, None, Some(branch))),
                channel,
                branch: branch.name.clone(),
                target: None,
                latest: false,
                date: branch.updated,
            },
            branch.build_id,
        ));
    }
    for build in &catalog.builds {
        let Some(key) = super::build::key(build) else {
            continue;
        };
        if !build.branches.iter().any(|b| b == "public")
            || list
                .iter()
                .any(|(c, id)| c.channel == "Stable" && *id == key)
        {
            continue;
        }
        list.push((
            Choice {
                name: build
                    .version
                    .clone()
                    .unwrap_or_else(|| format!("Build {key}")),
                channel: "Stable".into(),
                branch: "public".into(),
                target: Some(super::store::Target {
                    build: key,
                    depots: build.manifests.clone(),
                }),
                latest: false,
                date: Some(build.seen),
            },
            key,
        ));
    }
    // A pre-release that already shipped leaves its branch on the stable build: the channel
    // has nothing of its own to offer until the next one, so it isn't listed.
    let stable: Vec<u32> = list
        .iter()
        .filter(|(c, _)| c.channel == "Stable")
        .map(|(_, id)| *id)
        .collect();
    list.retain(|(c, id)| c.channel == "Stable" || !stable.contains(id));
    let rank = |channel: &str| match channel {
        "Stable" => 0,
        "Pre-release" => 1,
        "Unstable" => 2,
        "Build 41" => 3,
        _ => 4,
    };
    let version = |c: &Choice| c.name.parse::<zomboid::Version>().ok();
    // The channel's own branch holds its newest build, even one the catalog can't name yet.
    let head =
        |c: &Choice| c.target.is_none() && group(&c.branch) == c.channel && !fallback(&c.branch);
    list.sort_by(|(a, _), (b, _)| {
        rank(&a.channel)
            .cmp(&rank(&b.channel))
            .then_with(|| a.channel.cmp(&b.channel))
            .then_with(|| head(b).cmp(&head(a)))
            // Versions newest first, then what has none ("Build 26486631").
            .then_with(|| version(a).is_none().cmp(&version(b).is_none()))
            .then_with(|| version(b).cmp(&version(a)))
            .then_with(|| b.date.cmp(&a.date))
    });
    let mut out: Vec<Choice> = list.into_iter().map(|(c, _)| c).collect();
    for ix in 0..out.len() {
        out[ix].latest = ix == 0 || out[ix - 1].channel != out[ix].channel;
    }
    out
}

/// Opening an instance asks again, so the cache only spares repeated fetches within minutes.
const TTL: Duration = Duration::from_secs(600);
static CACHE: Mutex<Option<(Instant, Vec<Branch>)>> = Mutex::new(None);

/// Branches that can be installed (password-protected ones are left out).
pub async fn fetch() -> Result<Vec<Branch>, String> {
    if let Some((at, cached)) = CACHE.lock().expect("cache lock").clone()
        && at.elapsed() < TTL
    {
        return Ok(cached);
    }
    let branches = crate::runtime::spawn(async {
        let http = steam::http();
        let mut client = steam::Client::anonymous(&http).await?;
        let info = steam::pics::app_info(&mut client, &http, super::APP).await?;
        Ok::<_, steam::Error>(steam::pics::branches(&info))
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())?;
    let usable: Vec<_> = branches
        .into_iter()
        .filter(|b| !b.password_required)
        .collect();
    *CACHE.lock().expect("cache lock") = Some((Instant::now(), usable.clone()));
    Ok(usable)
}

/// "Nov 6, 2026" for a Unix time, in UTC.
pub fn date(unix: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Days to civil date (Howard Hinnant's algorithm).
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use zomboid::catalog::{Build, Catalog};

    fn branch(name: &str, build_id: u32) -> Branch {
        Branch {
            name: name.into(),
            build_id,
            description: None,
            password_required: false,
            updated: Some(u64::from(build_id)),
        }
    }

    fn build(id: Option<u32>, version: &str, branches: &[&str], manifest: u64) -> Build {
        Build {
            build: id,
            version: Some(version.into()),
            branches: branches.iter().map(|b| b.to_string()).collect(),
            seen: 1,
            manifests: BTreeMap::from([(108603, manifest)]),
        }
    }

    #[test]
    fn lists_versions_by_channel_newest_first() {
        let catalog = Catalog {
            builds: vec![
                build(Some(30), "42.22.2", &["public", "pre-release"], 3),
                build(Some(20), "42.21.0", &["42.21"], 2),
                build(Some(10), "41.78.21", &["legacy41"], 1),
                build(None, "42.20.4", &["public"], 5),
                // Only ever on a beta branch that moved on: Steam won't serve it.
                build(None, "42.22.0", &["unstable"], 6),
            ],
        };
        let live = [
            branch("public", 30),
            branch("pre-release", 30),
            branch("unstable", 31),
            branch("42.21", 20),
            branch("legacy41", 10),
        ];
        let list = choices_in(&catalog, &live);
        let seen: Vec<(&str, &str, &str, bool)> = list
            .iter()
            .map(|c| {
                (
                    c.channel.as_str(),
                    c.name.as_str(),
                    c.branch.as_str(),
                    c.latest,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("Stable", "42.22.2", "public", true),
                ("Stable", "42.21.0", "42.21", false),
                ("Stable", "42.20.4", "public", false),
                // Pre-release sits on the stable build; Unstable has one of its own.
                ("Unstable", "Build 31", "unstable", true),
                ("Build 41", "41.78.21", "legacy41", true),
            ]
        );
        // Only the build that left its branch is installed by its manifests.
        assert!(list[0].target.is_none() && list[1].target.is_none());
        let old = list[2].target.as_ref().unwrap();
        assert_eq!(old.depots, BTreeMap::from([(108603, 5)]));
        assert!(old.build & 0x8000_0000 != 0, "a key outside Steam's ids");
    }

    #[test]
    fn formats_branch_dates() {
        assert_eq!(date(0), "Jan 1, 1970");
        assert_eq!(date(1_793_974_760), "Nov 6, 2026");
        assert_eq!(date(1_787_663_102), "Aug 25, 2026");
        assert_eq!(date(951_782_400), "Feb 29, 2000");
    }
}
