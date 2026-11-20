//! Steam Workshop items. A PZ mod's files live in the game's own depot (108600), under the
//! manifest named by `hcontent_file`.

use std::collections::HashSet;
use std::path::Path;

use crate::proto::{
    CPublishedFileGetDetailsRequest, CPublishedFileGetDetailsResponse, PublishedFileDetails,
};
use crate::{Client, Error, Result, content};

/// `EWorkshopFileType::Collection`.
const COLLECTION: u32 = 2;
/// Items asked for in one request.
const BATCH: usize = 50;
/// A collection with its dependencies can't go past this many items.
const MAX_ITEMS: usize = 2000;
/// `EResult::OK`.
const OK: u32 = 1;

#[derive(Debug, Clone)]
pub struct Item {
    pub id: u64,
    pub app: u32,
    pub title: String,
    /// SteamID of the author; [`authors`] gives the name.
    pub author: u64,
    /// The start of its description, in BBCode; only [`search`] asks for it.
    pub summary: String,
    /// Manifest of the current version in the app's depot.
    pub manifest: u64,
    /// Legacy items are a direct download instead of a manifest.
    pub file_url: Option<String>,
    pub size: u64,
    pub updated: Option<u32>,
    /// Workshop tags, as their author set them (`Build 42`, `Multiplayer`...).
    pub tags: Vec<String>,
    /// URL of its picture on the Workshop, when it is on one of Steam's image hosts.
    pub preview: Option<String>,
    /// The Workshop items its author marked as required (for a collection, its items).
    pub requires: Vec<u64>,
    /// How many users are subscribed to it now.
    pub subscriptions: u32,
    /// `EWorkshopFileType`: 0 for a mod; guides, art, collections and the rest have another.
    pub file_type: u32,
}

impl Item {
    /// Whether it is a regular Workshop mod, and not a guide, art or something else that has
    /// files too.
    pub fn is_mod(&self) -> bool {
        self.file_type == 0
    }
}

impl From<PublishedFileDetails> for Item {
    fn from(d: PublishedFileDetails) -> Self {
        Self {
            id: d.publishedfileid(),
            app: d.consumer_appid(),
            title: d.title().to_owned(),
            author: d.creator(),
            summary: d.short_description().to_owned(),
            manifest: d.hcontent_file(),
            size: d.file_size(),
            updated: d.time_updated,
            tags: d.tags.iter().map(|t| t.tag().to_owned()).collect(),
            preview: d.preview_url.clone().filter(|url| is_steam_image(url)),
            requires: d.children.iter().map(|c| c.publishedfileid()).collect(),
            subscriptions: d.subscriptions(),
            file_type: d.file_type(),
            // Last: it moves out of `d`.
            file_url: d.file_url.filter(|u| !u.is_empty()),
        }
    }
}

async fn query(
    client: &mut Client,
    ids: &[u64],
    children: bool,
) -> Result<Vec<PublishedFileDetails>> {
    let request = CPublishedFileGetDetailsRequest {
        publishedfileids: ids.to_vec(),
        includetags: Some(true),
        includeadditionalpreviews: None,
        includechildren: children.then_some(true),
    };
    let reply: CPublishedFileGetDetailsResponse =
        client.call("PublishedFile.GetDetails", &request).await?;
    Ok(reply.publishedfiledetails)
}

/// What the Workshop says about each of `ids`, with the items each one requires.
pub async fn details(client: &mut Client, ids: &[u64]) -> Result<Vec<Item>> {
    Ok(query(client, ids, true)
        .await?
        .into_iter()
        .map(Item::from)
        .collect())
}

/// Everything an item's page shows: [`Item`] plus its whole description and gallery.
#[derive(Debug, Clone)]
pub struct About {
    pub item: Item,
    /// The whole description, in BBCode.
    pub description: String,
    /// When it was first published, in seconds since the epoch.
    pub created: Option<u32>,
    /// How many users have it among their favorites.
    pub favorited: u32,
    /// The gallery's pictures, on Steam's image hosts; videos are left out.
    pub gallery: Vec<String>,
}

/// The page of item `id`: what [`details`] gives, its whole description and its gallery.
pub async fn about(client: &mut Client, id: u64) -> Result<About> {
    let request = CPublishedFileGetDetailsRequest {
        publishedfileids: vec![id],
        includetags: Some(true),
        includeadditionalpreviews: Some(true),
        includechildren: Some(true),
    };
    let reply: CPublishedFileGetDetailsResponse =
        client.call("PublishedFile.GetDetails", &request).await?;
    let d = reply
        .publishedfiledetails
        .into_iter()
        .find(|d| d.result() == OK)
        .ok_or(Error::Gone)?;
    let gallery = d
        .previews
        .iter()
        .filter(|p| p.youtubevideoid().is_empty())
        .filter_map(|p| p.url.clone())
        .filter(|url| is_steam_image(url))
        .collect();
    Ok(About {
        description: d.file_description().to_owned(),
        created: d.time_created,
        favorited: d.favorited(),
        gallery,
        item: Item::from(d),
    })
}

/// One version an item's author published.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// When it was published, in seconds since the epoch.
    pub date: u64,
    /// The author's note for it; often empty.
    pub note: String,
    pub manifest: u64,
}

/// How many versions to ask for; mods that update weekly pass a hundred in two years.
const HISTORY: u32 = 100;

/// The versions of item `id`, newest first (the first one is what [`details`] calls its
/// current manifest). Steam keeps serving the old ones: `manifest` of any of them downloads.
pub async fn history(client: &mut Client, id: u64) -> Result<Vec<Change>> {
    let request = crate::proto::CPublishedFileGetChangeHistoryRequest {
        publishedfileid: Some(id),
        startindex: None,
        count: Some(HISTORY),
    };
    let reply: crate::proto::CPublishedFileGetChangeHistoryResponse = client
        .call("PublishedFile.GetChangeHistory", &request)
        .await?;
    let mut changes: Vec<Change> = reply
        .changes
        .into_iter()
        // An entry without a manifest is a text-only edit of the page, not a version.
        .filter(|c| c.manifest_id() != 0)
        .map(|c| Change {
            date: u64::from(c.timestamp()),
            note: c.change_description().trim().to_owned(),
            manifest: c.manifest_id(),
        })
        .collect();
    changes.sort_by_key(|c| std::cmp::Reverse(c.date));
    Ok(changes)
}

/// How [`search`] ranks what it finds (`EPublishedFileQueryType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    /// What matches the search text best; without text Steam ranks by trend.
    Relevance,
    /// Gained the most over the last `days`.
    Trend {
        days: u32,
    },
    /// Most subscribed of all time.
    Popular,
    Newest,
    Updated,
}

impl Sort {
    fn query_type(self) -> u32 {
        match self {
            Sort::Newest => 1,
            Sort::Trend { .. } => 3,
            Sort::Popular => 9,
            Sort::Relevance => 12,
            Sort::Updated => 21,
        }
    }
}

/// What to look for in an app's Workshop.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub app: u32,
    /// Matched against titles and descriptions; empty for everything.
    pub text: String,
    /// Only items carrying all of these tags.
    pub tags: Vec<String>,
    /// Only items carrying none of these.
    pub excluded: Vec<String>,
    pub sort: Sort,
    /// Where the page starts: `None` for the first, then [`Page::next`].
    pub cursor: Option<String>,
    pub count: u32,
}

/// One page of what [`search`] found.
#[derive(Debug)]
pub struct Page {
    /// Mods only: collections are left out.
    pub items: Vec<Item>,
    /// How many items match in all.
    pub total: u32,
    /// The cursor of the page after this one; `None` at the end.
    pub next: Option<String>,
}

/// A page of the Workshop items that match `query`, with their tags and the start of their
/// descriptions. Works on an anonymous session.
pub async fn search(client: &mut Client, query: &Query) -> Result<Page> {
    let text = query.text.trim();
    let request = crate::proto::CPublishedFileQueryFilesRequest {
        query_type: Some(query.sort.query_type()),
        numperpage: Some(query.count),
        appid: Some(query.app),
        requiredtags: query.tags.clone(),
        excludedtags: query.excluded.clone(),
        search_text: (!text.is_empty()).then(|| text.to_owned()),
        filetype: Some(0),
        days: match query.sort {
            Sort::Trend { days } => Some(days),
            _ => None,
        },
        return_tags: Some(true),
        return_short_description: Some(true),
        cursor: Some(query.cursor.clone().unwrap_or_else(|| "*".into())),
        page: None,
    };
    let reply: crate::proto::CPublishedFileQueryFilesResponse =
        client.call("PublishedFile.QueryFiles", &request).await?;
    let items: Vec<Item> = reply
        .publishedfiledetails
        .into_iter()
        .filter(|d| d.result() == OK && d.file_type() != COLLECTION)
        .map(Item::from)
        .collect();
    // The last page hands back the cursor it was given, or none.
    let next = reply
        .next_cursor
        .filter(|next| !items.is_empty() && Some(next) != query.cursor.as_ref() && next != "*");
    Ok(Page {
        items,
        total: reply.total.unwrap_or(0),
        next,
    })
}

/// The `count` items of `app`'s Workshop that gained the most over the last `days`, most
/// first. Collections are left out.
pub async fn trending(client: &mut Client, app: u32, days: u32, count: u32) -> Result<Vec<Item>> {
    let query = Query {
        app,
        text: String::new(),
        tags: Vec::new(),
        excluded: Vec::new(),
        sort: Sort::Trend { days },
        cursor: None,
        count,
    };
    Ok(search(client, &query).await?.items)
}

/// The names the authors in `ids` go by on their profiles. Steam denies this to anonymous
/// sessions, so `client` has to be logged on as a user; ids it says nothing about are left out.
pub async fn authors(
    client: &mut Client,
    ids: &[u64],
) -> Result<std::collections::HashMap<u64, String>> {
    let request = crate::proto::CPlayerGetPlayerLinkDetailsRequest {
        steamids: ids.to_vec(),
    };
    let reply: crate::proto::CPlayerGetPlayerLinkDetailsResponse =
        client.call("Player.GetPlayerLinkDetails", &request).await?;
    Ok(reply
        .accounts
        .into_iter()
        .filter_map(|account| {
            let data = account.public_data?;
            Some((data.steamid?, data.persona_name.filter(|n| !n.is_empty())?))
        })
        .collect())
}

/// A collection's mods, with the ones they require.
#[derive(Debug)]
pub struct Collection {
    pub id: u64,
    pub title: String,
    /// URL of its picture on the Workshop, as [`Item::preview`].
    pub preview: Option<String>,
    /// Every mod once, in the collection's order and then the order dependencies were found.
    /// Items that are gone or private are left out.
    pub mods: Vec<Item>,
}

/// The mods of collection `id`: the ones it lists, the collections inside it, and what each
/// mod requires (`children`), each once.
pub async fn collection(client: &mut Client, id: u64) -> Result<Collection> {
    let root = query(client, &[id], true)
        .await?
        .into_iter()
        .next()
        .filter(|d| d.result() == OK && d.file_type() == COLLECTION)
        .ok_or(Error::NotCollection)?;
    let mut seen = HashSet::from([id]);
    let mut queue = new_children(&root, &mut seen);
    let mut mods = Vec::new();
    while !queue.is_empty() {
        let take = queue.len().min(BATCH);
        let batch: Vec<u64> = queue.drain(..take).collect();
        for item in query(client, &batch, true).await? {
            queue.extend(new_children(&item, &mut seen));
            if item.result() == OK && item.file_type() != COLLECTION {
                mods.push(Item::from(item));
            }
        }
        if seen.len() > MAX_ITEMS {
            return Err(Error::TooLarge("collection"));
        }
    }
    Ok(Collection {
        id,
        title: root.title().to_owned(),
        preview: root.preview_url.clone().filter(|url| is_steam_image(url)),
        mods,
    })
}

/// The ids `item` lists that haven't been seen yet, by their sort order; marks them seen.
fn new_children(item: &PublishedFileDetails, seen: &mut HashSet<u64>) -> Vec<u64> {
    let mut children: Vec<_> = item.children.iter().collect();
    children.sort_by_key(|child| child.sortorder());
    children
        .into_iter()
        .map(|child| child.publishedfileid())
        .filter(|id| seen.insert(*id))
        .collect()
}

/// A Workshop picture is at most this big; they are a few hundred KB.
const PREVIEW_CAP: usize = 8 << 20;

/// An `https` URL on one of the hosts Steam serves Workshop pictures from. The URL comes from
/// what an item's author uploaded, so it is checked before lata fetches it.
pub fn is_steam_image(url: &str) -> bool {
    let Some(host) = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
    else {
        return false;
    };
    [".steamusercontent.com", ".steamstatic.com", ".akamaihd.net"]
        .iter()
        .any(|suffix| host.ends_with(suffix))
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// The picture at `url`, one of [`Item::preview`].
pub async fn preview(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let reply = http
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    crate::read_capped(reply, PREVIEW_CAP, "preview").await
}

/// The item id in what a user pastes: a Workshop or Steam link, or the number itself.
pub fn parse_id(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Ok(id) = text.parse() {
        return Some(id);
    }
    if let Some(id) = text.strip_prefix("steam://url/CommunityFilePage/") {
        return id.trim_end_matches('/').parse().ok();
    }
    let (host, path) = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))?
        .split_once('/')?;
    let (path, query) = path.split_once('?')?;
    let page = path.trim_end_matches('/');
    let ours = matches!(host, "steamcommunity.com" | "www.steamcommunity.com")
        && matches!(page, "sharedfiles/filedetails" | "workshop/filedetails");
    if !ours {
        return None;
    }
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("id=")?.parse().ok())
}

/// What every Workshop download of an app needs: its depot key and CDN hosts. They are the
/// same for all of the app's items, so a batch asks for them once.
#[derive(Debug, Clone)]
pub struct Source {
    app: u32,
    key: content::Key,
    hosts: Vec<String>,
}

/// The [`Source`] for `app`'s Workshop items (PZ's live in the game's own depot).
pub async fn source(client: &mut Client, app: u32) -> Result<Source> {
    Ok(Source {
        app,
        key: content::depot_key(client, app, app).await?,
        hosts: content::cdn_hosts(client).await?,
    })
}

/// The file list of `item`'s current version. This is the step that needs the Steam client,
/// so a batch takes them one by one and downloads with [`fetch`] in parallel.
pub async fn manifest(
    client: &mut Client,
    http: &reqwest::Client,
    source: &Source,
    item: &Item,
) -> Result<content::Manifest> {
    content::manifest(
        client,
        http,
        &source.hosts[0],
        (source.app, source.app, item.manifest),
        "public",
        &source.key,
    )
    .await
}

/// Writes the files of `manifest` into `dir`. `progress` receives the bytes done so far.
// ponytail: legacy `file_url` items aren't handled; none of the PZ mods tested use them.
pub async fn fetch(
    http: &reqwest::Client,
    source: &Source,
    manifest: &content::Manifest,
    dir: &Path,
    progress: impl FnMut(u64),
) -> Result<()> {
    content::install(
        http,
        &source.hosts,
        (source.app, &source.key),
        manifest,
        dir,
        None,
        progress,
    )
    .await
    .map(drop)
}

/// Downloads `item`'s current version into `dir`. `progress` receives the bytes done so far.
pub async fn download(
    client: &mut Client,
    http: &reqwest::Client,
    item: &Item,
    dir: &Path,
    progress: impl FnMut(u64),
) -> Result<()> {
    let source = source(client, item.app).await?;
    let manifest = manifest(client, http, &source, item).await?;
    fetch(http, &source, &manifest, dir, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::PublishedFileChild;

    #[test]
    fn finds_the_id_in_what_people_paste() {
        let id = Some(3832274571);
        assert_eq!(parse_id("3832274571"), id);
        assert_eq!(parse_id("  3832274571\n"), id);
        for link in [
            "https://steamcommunity.com/sharedfiles/filedetails/?id=3832274571",
            "https://steamcommunity.com/sharedfiles/filedetails?id=3832274571",
            "https://www.steamcommunity.com/workshop/filedetails/?id=3832274571&searchtext=x",
            "http://steamcommunity.com/sharedfiles/filedetails/?l=spanish&id=3832274571",
            "steam://url/CommunityFilePage/3832274571",
        ] {
            assert_eq!(parse_id(link), id, "{link}");
        }
        for not in [
            "",
            "mods",
            "https://evil.example/sharedfiles/filedetails/?id=3832274571",
            "https://steamcommunity.com/id/someone/",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=abc",
            "https://steamcommunity.com/sharedfiles/filedetails/",
        ] {
            assert_eq!(parse_id(not), None, "{not}");
        }
    }

    #[test]
    fn only_steam_hosts_serve_previews() {
        for ok in [
            "https://images.steamusercontent.com/ugc/123/ABC/",
            "https://shared.fastly.steamstatic.com/a.jpg",
            "https://steamuserimages-a.akamaihd.net/ugc/1/2/",
        ] {
            assert!(is_steam_image(ok), "{ok}");
        }
        for bad in [
            "http://images.steamusercontent.com/ugc/1/",
            "https://evil.example/images.steamusercontent.com/a",
            "https://steamusercontent.com.evil.example/a",
            "https://user:pw@images.steamusercontent.com/a",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!is_steam_image(bad), "{bad}");
        }
    }

    #[test]
    fn children_come_in_sort_order_and_only_once() {
        let child = |id, sortorder| PublishedFileChild {
            publishedfileid: Some(id),
            sortorder: Some(sortorder),
        };
        let item = PublishedFileDetails {
            children: vec![child(30, 2), child(10, 0), child(20, 1), child(10, 3)],
            ..Default::default()
        };
        let mut seen = HashSet::from([20]);
        assert_eq!(new_children(&item, &mut seen), [10, 30]);
        // A second pass over the same item finds nothing new, so shared dependencies don't loop.
        assert!(new_children(&item, &mut seen).is_empty());
    }
}
