//! Records every current game branch into the catalog:
//! `cargo run -p zomboid --example catalog -- crates/zomboid/versions.json`.
//!
//! The game's depots can't be downloaded anonymously, so the version comes from the dedicated
//! server (same branch names), whose depot keys are free. Run on a schedule, with no login.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use zomboid::catalog::Build;

const GAME: u32 = 108600;
const SERVER: u32 = 380870;
/// The server's common depot: `projectzomboid.jar`, or a loose `Core.class` on `legacy41`.
const SERVER_COMMON: u32 = 380871;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> Result<()> {
    let file = std::env::args()
        .nth(1)
        .ok_or("usage: catalog <versions.json>")?;
    let mut catalog = zomboid::catalog::parse(&std::fs::read_to_string(&file)?)?;
    let before = catalog.to_json();

    let http = steam::http();
    let mut client = steam::Client::anonymous(&http).await?;
    let game = steam::pics::app_info(&mut client, &http, GAME).await?;
    let server = steam::pics::app_info(&mut client, &http, SERVER).await?;
    let hosts = steam::content::cdn_hosts(&mut client).await?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    // One jar per server manifest: public and pre-release usually share it.
    let mut by_manifest: HashMap<u64, Option<String>> = HashMap::new();
    for branch in steam::pics::branches(&game) {
        if branch.password_required {
            continue;
        }
        let mut manifests = BTreeMap::new();
        for os in ["linux", "windows", "macos"] {
            for depot in steam::pics::depots(&game, &branch.name, os) {
                manifests.insert(depot.id, depot.manifest);
            }
        }
        let mut version = catalog.get(branch.build_id).and_then(|b| b.version.clone());
        if version.is_none() {
            let gid = steam::pics::depots(&server, &branch.name, "linux")
                .into_iter()
                .find(|d| d.id == SERVER_COMMON)
                .map(|d| d.manifest);
            match gid {
                Some(gid) => {
                    if let Entry::Vacant(slot) = by_manifest.entry(gid) {
                        // Not fatal: the manifests are what can't be recorded later.
                        let read = read_version(&mut client, &http, &hosts, &branch.name, gid)
                            .await
                            .inspect_err(|e| eprintln!("{}: no version: {e}", branch.name))
                            .ok()
                            .flatten();
                        slot.insert(read);
                    }
                    version = by_manifest[&gid].clone();
                }
                None => eprintln!("{}: the server has no such branch", branch.name),
            }
        }
        let changed = catalog.record(Build {
            build: branch.build_id,
            version: version.clone(),
            branches: vec![branch.name.clone()],
            seen: now,
            manifests,
        });
        println!(
            "{:<12} build {} {}{}",
            branch.name,
            branch.build_id,
            version.as_deref().unwrap_or("(no version)"),
            if changed { "  changed" } else { "" }
        );
    }

    if catalog.to_json() != before {
        std::fs::write(&file, catalog.to_json())?;
        println!("{file} updated");
    }
    Ok(())
}

/// Downloads only the file that holds the version from the server's common depot.
async fn read_version(
    client: &mut steam::Client,
    http: &reqwest::Client,
    hosts: &[String],
    branch: &str,
    gid: u64,
) -> steam::Result<Option<String>> {
    let key = steam::content::depot_key(client, SERVER, SERVER_COMMON).await?;
    let mut manifest = steam::content::manifest(
        client,
        http,
        &hosts[0],
        (SERVER, SERVER_COMMON, gid),
        branch,
        &key,
    )
    .await?;
    manifest
        .files
        .retain(|f| zomboid::version::is_source(&f.path));
    let dir = std::env::temp_dir().join(format!("lata-catalog-{gid}"));
    steam::content::install(
        http,
        hosts,
        (SERVER_COMMON, &key),
        &manifest,
        &dir,
        None,
        |_| {},
    )
    .await?;
    let version = manifest
        .files
        .iter()
        .find_map(|f| zomboid::version::read(&dir.join(&f.path)))
        .map(|v| v.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(version)
}
