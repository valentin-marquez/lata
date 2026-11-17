//! Adds builds that left their branch to the catalog, from manifest ids someone wrote down:
//!
//! ```sh
//! LATA_ACCOUNT="$(secret-tool lookup service lata username steam)" \
//!   cargo run -p zomboid --example backfill -- crates/zomboid/versions.json 2025-11-16 \
//!   108604=windows.txt 108603=linux.txt 108602=macos.txt
//! ```
//!
//! Steam keeps no history, so the scheduled `catalog` job only knows builds from the day it
//! started. Each list is a depot's manifests as SteamDB's manifests page shows them, copied by
//! hand (one per line: date, manifest id, branch if any); the first depot leads and the others
//! are matched to it by time, since one build uploads all its depots within minutes. A build
//! that isn't in every list is left out, so each version in the catalog installs everywhere.
//!
//! Old game manifests need an account that owns the game, so this runs on the maintainer's
//! machine. `LATA_ACCOUNT` is the account as lata saves it in the keyring (JSON with `name`,
//! `steamid`, `refresh_token`); it is read from the environment and never written anywhere.
//! Only builds that were once on the public branch can be had: Steam denies the rest for good.
//! For each build it asks Steam for the manifest (which tells whether Steam still serves it),
//! downloads only the file that holds the version, and records the build. The file is saved
//! after every build, so an interrupted run picks up where it stopped.

use std::collections::BTreeMap;
use std::path::Path;

use steam::content;
use zomboid::catalog::{self, Build};

const GAME: u32 = 108600;
/// Depots of one build are uploaded together; lists further apart than this are other builds.
const TOGETHER: i64 = 15 * 60;
/// The build line worth keeping: older ones are covered by their legacy branch.
const MAJOR: u32 = 42;

type Failure = Box<dyn std::error::Error>;

#[derive(Debug, Clone, PartialEq)]
struct Row {
    /// Seconds since the epoch.
    seen: i64,
    manifest: u64,
    branch: Option<String>,
}

#[derive(serde::Deserialize)]
struct Account {
    name: String,
    steamid: u64,
    refresh_token: String,
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, since, lists @ ..] = args.as_slice() else {
        return Err(
            "usage: backfill <versions.json> <since YYYY-MM-DD> <depot>=<list.txt>...".into(),
        );
    };
    let since = date(since).ok_or("since is not YYYY-MM-DD")?;
    let mut depots = Vec::new();
    for list in lists {
        let (depot, path) = list.split_once('=').ok_or("lists are <depot>=<file>")?;
        let rows: Vec<Row> = std::fs::read_to_string(path)?
            .lines()
            .filter_map(row)
            .filter(|r| r.seen >= since)
            .collect();
        depots.push((depot.parse::<u32>()?, rows));
    }
    let Some(((lead, rows), others)) = depots.split_first() else {
        return Err("give at least one list".into());
    };

    let account: Account = serde_json::from_str(
        &std::env::var("LATA_ACCOUNT").map_err(|_| "set LATA_ACCOUNT (see this file's top)")?,
    )?;
    let http = steam::http();
    let mut client = steam::Client::connect(&http).await?;
    client
        .user(&account.name, &account.refresh_token, account.steamid)
        .await?;
    let hosts = content::cdn_hosts(&mut client).await?;
    let key = content::depot_key(&mut client, GAME, *lead).await?;

    let path = Path::new(file);
    let mut catalog = catalog::parse(&std::fs::read_to_string(path)?)?;
    let scratch = path.with_extension(format!("backfill-{}", std::process::id()));
    let (mut added, mut gone) = (0, Vec::new());
    for row in rows {
        let mut manifests = BTreeMap::from([(*lead, row.manifest)]);
        for (depot, rows) in others {
            let Some(same) = rows
                .iter()
                .filter(|r| (r.seen - row.seen).abs() <= TOGETHER && r.branch == row.branch)
                .min_by_key(|r| (r.seen - row.seen).abs())
            else {
                continue;
            };
            // Only what Steam hands out is recorded; one already in the catalog was checked.
            if catalog.of(*depot, same.manifest).is_none() {
                let key = content::depot_key(&mut client, GAME, *depot).await?;
                let id = (GAME, *depot, same.manifest);
                if let Err(err) =
                    content::manifest(&mut client, &http, &hosts[0], id, "public", &key).await
                {
                    println!(
                        "{}  {depot}:{}  not served: {err}",
                        day(row.seen),
                        same.manifest
                    );
                    continue;
                }
                println!("{}  {depot}:{}  served", day(row.seen), same.manifest);
            }
            manifests.insert(*depot, same.manifest);
        }
        // A version has to install on every system: a build missing a depot is left out.
        if manifests.len() <= others.len() {
            println!(
                "{}  {}  not on every depot given",
                day(row.seen),
                row.manifest
            );
            continue;
        }
        // Already read on an earlier run: only its other depots may be new.
        if let Some(known) = catalog.of(*lead, row.manifest).cloned() {
            if catalog.record(Build { manifests, ..known }) {
                std::fs::write(path, catalog.to_json())?;
            }
            continue;
        }
        // Steam hands out an old manifest only if it was once on `public` (measured 2026-11-16
        // with an owning account: ones that only lived on a beta branch are denied under every
        // branch name, their own included). So rows SteamDB shows with a branch are skipped.
        if row.branch.is_some() {
            continue;
        }
        let manifest = match content::manifest(
            &mut client,
            &http,
            &hosts[0],
            (GAME, *lead, row.manifest),
            "public",
            &key,
        )
        .await
        {
            Ok(manifest) => manifest,
            Err(err) => {
                println!("{}  {}  not served: {err}", day(row.seen), row.manifest);
                gone.push(row.manifest);
                continue;
            }
        };
        let version = read(&http, &hosts, *lead, &key, &manifest, &scratch).await;
        let label = version.as_deref().unwrap_or("no version found");
        let keep = version
            .as_deref()
            .and_then(|v| v.parse::<zomboid::Version>().ok())
            .is_some_and(|v| v.major >= MAJOR);
        println!(
            "{}  {}  {label}{}",
            day(row.seen),
            row.manifest,
            if keep { "" } else { "  (skipped)" }
        );
        if !keep {
            continue;
        }
        let seen = match manifest.created {
            0 => row.seen as u64,
            created => u64::from(created),
        };
        catalog.record(Build {
            build: None,
            version,
            branches: vec!["public".into()],
            seen,
            manifests,
        });
        std::fs::write(path, catalog.to_json())?;
        added += 1;
    }
    println!("{added} builds added, {} no longer served", gone.len());
    Ok(())
}

/// Downloads only the file of `manifest` that holds the version, into `scratch`, and reads it.
async fn read(
    http: &steam::reqwest::Client,
    hosts: &[String],
    depot: u32,
    key: &content::Key,
    manifest: &content::Manifest,
    scratch: &Path,
) -> Option<String> {
    let mut only = manifest.clone();
    only.files
        .retain(|f| !f.is_dir && zomboid::version::is_source(&f.path));
    let source = only.files.first()?.path.clone();
    let done = content::install(http, hosts, (depot, key), &only, scratch, None, |_| {}).await;
    let version = done
        .ok()
        .and_then(|_| zomboid::version::read(&scratch.join(&source)))
        .map(|v| v.to_string());
    let _ = std::fs::remove_dir_all(scratch);
    version
}

/// One line of SteamDB's manifests table: "6 November 2026 – 14:41:06 UTC  10 days ago
/// 2378094061158458897 pre-release". Anything else (the header, blanks) is `None`.
fn row(line: &str) -> Option<Row> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let [d, month, y, _, time, ..] = words.as_slice() else {
        return None;
    };
    let month = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .iter()
    .position(|m| m == month)? as i64
        + 1;
    let mut clock = time.split(':').map(|n| n.parse::<i64>().ok());
    let secs = clock.next()?? * 3600 + clock.next()?? * 60 + clock.next()??;
    let at = words
        .iter()
        .position(|w| w.len() >= 12 && w.bytes().all(|b| b.is_ascii_digit()))?;
    Some(Row {
        seen: days(y.parse().ok()?, month, d.parse().ok()?) * 86_400 + secs,
        manifest: words[at].parse().ok()?,
        branch: words.get(at + 1).map(|b| b.to_string()),
    })
}

/// "2025-11-16" as seconds since the epoch.
fn date(text: &str) -> Option<i64> {
    let mut parts = text.split('-').map(|n| n.parse::<i64>().ok());
    Some(days(parts.next()??, parts.next()??, parts.next()??) * 86_400)
}

/// Days since the epoch of a civil date (Howard Hinnant's algorithm), to avoid a date crate.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Seconds since the epoch as "2026-11-06".
fn day(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{:04}-{m:02}-{:02}",
        yoe + era * 400 + i64::from(m <= 2),
        doy - (153 * mp + 2) / 5 + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_table_as_it_is_copied() {
        let labelled = row(
            "4 November 2026 – 10:07:01 UTC     13 days ago     665160979465878368 pre-release",
        )
        .unwrap();
        assert_eq!(labelled.manifest, 665160979465878368);
        assert_eq!(labelled.branch.as_deref(), Some("pre-release"));
        assert_eq!(day(labelled.seen), "2026-11-04");
        let plain =
            row("5 August 2026 – 10:07:05 UTC     3 months ago     4032122953183820411     ")
                .unwrap();
        assert_eq!((plain.branch, plain.seen % 86_400), (None, 36_425));
        assert_eq!(row("Seen Date     Relative Date     ManifestID"), None);
        assert_eq!(row(""), None);
        assert_eq!(date("2026-11-04").map(day).as_deref(), Some("2026-11-04"));
        assert!(date("2026-11-04").unwrap() < labelled.seen);
    }
}
