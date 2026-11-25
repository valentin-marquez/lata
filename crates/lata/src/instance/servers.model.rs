//! The servers an instance has joined. The game keeps a copy of each one's world in
//! `<cachedir>/Saves/Multiplayer/<host>_<port>_<md5 of the username>/` and, in its server list,
//! the name it was saved under and the character played there.

use std::io;
use std::net::{ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::Duration;

use super::worlds::{self, World};

#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    /// The cached copy: its folder, screenshot, last save, and the character when the folder
    /// keeps a `players.db` or the server list knows them.
    pub world: World,
    /// What was typed to join it: an IP or a host name.
    pub host: String,
    pub port: u16,
    /// The name it has in the game's server list.
    pub name: Option<String>,
}

impl Server {
    /// What `+connect` takes.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn title(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.address())
    }
}

/// The servers joined from `cachedir`, the last played first.
pub fn list(cachedir: &Path) -> Vec<Server> {
    let Ok(dirs) = std::fs::read_dir(cachedir.join("Saves/Multiplayer")) else {
        return Vec::new();
    };
    let saved = entries(cachedir);
    let mut servers: Vec<Server> = dirs
        .flatten()
        .filter(|dir| dir.path().is_dir())
        .filter_map(|dir| {
            let folder = dir.file_name().to_string_lossy().into_owned();
            let (host, port, user) = parse(&folder)?;
            let mut world = worlds::read(&dir.path(), "Multiplayer");
            // Several accounts can sit on one server: the folder names whose it is.
            let on_server = || saved.iter().filter(|e| e.host == host && e.port == port);
            let entry = on_server()
                .find(|e| e.user.as_deref().is_some_and(|u| md5(u) == user))
                .or_else(|| on_server().next());
            if world.character.is_none() {
                world.character = entry.and_then(|e| e.character.clone());
            }
            Some(Server {
                world,
                host: host.to_owned(),
                port,
                name: entry.map(|e| e.name.clone()),
            })
        })
        .collect();
    servers.sort_by_key(|server| std::cmp::Reverse(server.world.saved));
    servers
}

/// `147.185.221.214_45360_0f0d...` into its host, port and username hash. A host name can
/// hold underscores, so the port and hash are taken from the end.
fn parse(folder: &str) -> Option<(&str, u16, &str)> {
    let mut parts = folder.rsplitn(3, '_');
    let user = parts.next()?;
    let port = parts.next()?.parse().ok()?;
    // A host name or an IP and nothing else: it goes to the game's command line after
    // `+connect`, where a leading `-` would read as an option.
    let host = parts.next().filter(|host| {
        !host.is_empty()
            && !host.starts_with('-')
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    })?;
    Some((host, port, user))
}

fn md5(text: &str) -> String {
    use md5::{Digest, Md5};
    Md5::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A server saved in the game's list, with one of its accounts.
#[derive(Debug, PartialEq)]
struct Entry {
    name: String,
    host: String,
    port: u16,
    user: Option<String>,
    character: Option<String>,
}

/// The game's saved servers: Build 42 keeps them in `db/ServerListSteam.db` (`ServerList.db`
/// without Steam), Build 41 in `Lua/ServerListSteam.txt` (`ServerList.txt`).
fn entries(cachedir: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    for steam in ["ServerListSteam", "ServerList"] {
        entries.extend(from_db(&cachedir.join(format!("db/{steam}.db"))).unwrap_or_default());
        if let Ok(text) = std::fs::read_to_string(cachedir.join(format!("Lua/{steam}.txt"))) {
            entries.extend(from_txt(&text));
        }
    }
    entries
}

fn from_db(path: &Path) -> rusqlite::Result<Vec<Entry>> {
    use rusqlite::{Connection, OpenFlags};
    if !path.is_file() {
        return Ok(Vec::new());
    }
    // Read-only, so a game that is running and saving is never in the way. Passwords stay
    // where they are: only these columns are read.
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut rows = db.prepare(
        "SELECT s.name, s.ip, s.port, a.username, a.playerFirstAndLastName \
         FROM server s LEFT JOIN account a ON a.serverId = s.id",
    )?;
    let found = rows
        .query_map([], |row| {
            Ok(Entry {
                name: row.get(0)?,
                host: row.get(1)?,
                port: row.get(2)?,
                user: row.get(3)?,
                character: row
                    .get::<_, Option<String>>(4)?
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty()),
            })
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(found)
}

/// Build 41's list: `key=value` lines, each server starting at its `name=`.
fn from_txt(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if key == "name" {
            entries.push(Entry {
                name: value.to_owned(),
                host: String::new(),
                port: 0,
                user: None,
                character: None,
            });
            continue;
        }
        let Some(entry) = entries.last_mut() else {
            continue;
        };
        match key {
            "ip" => entry.host = value.to_owned(),
            "port" => entry.port = value.parse().unwrap_or(0),
            "user" => entry.user = Some(value.to_owned()).filter(|u| !u.is_empty()),
            _ => {}
        }
    }
    entries
}

/// Where asking a server for its players stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Count {
    Asking,
    Online(Players),
    /// It didn't answer: down, or behind a firewall that drops the query.
    Silent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Players {
    pub online: u8,
    pub max: u8,
}

/// How many play on the server now, asked of the server itself. Its Steam side answers
/// A2S_INFO on the game port (the port `+connect` takes), as public trackers list it too.
/// Blocks for up to a few seconds.
pub fn players(host: &str, port: u16) -> io::Result<Players> {
    let addr = (host, port)
        .to_socket_addrs()?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no IPv4 address"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.connect(addr)?;
    let mut query = b"\xff\xff\xff\xffTSource Engine Query\0".to_vec();
    let mut reply = [0u8; 1400];
    // Newer servers answer the first query with a challenge to send back.
    for _ in 0..2 {
        socket.send(&query)?;
        let n = socket.recv(&mut reply)?;
        match &reply[..n] {
            [0xff, 0xff, 0xff, 0xff, b'A', challenge @ ..] if challenge.len() >= 4 => {
                query.truncate(25);
                query.extend_from_slice(&challenge[..4]);
            }
            answer => {
                return info(answer).ok_or_else(|| io::Error::other("not an A2S_INFO answer"));
            }
        }
    }
    Err(io::Error::other("the server kept asking for a challenge"))
}

/// The player count in an A2S_INFO answer: header, protocol, four strings (name, map, folder,
/// game), the app id, then players and max players.
fn info(answer: &[u8]) -> Option<Players> {
    let rest = answer.strip_prefix(b"\xff\xff\xff\xffI")?.get(1..)?;
    let mut at = 0;
    for _ in 0..4 {
        at += rest.get(at..)?.iter().position(|&b| b == 0)? + 1;
    }
    let (online, max) = (*rest.get(at + 2)?, *rest.get(at + 3)?);
    Some(Players { online, max })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A2S_INFO answers captured from public servers on 2026-11-24: a Build 42.22 server with
    /// 86 of 150 playing and a Build 41.78 one with 7 of 100.
    const B42: &str = "ffffffff49115b50565056455d20204166746572207468652046616c6c207c20534541534f4e203130207c204c6f6e672057697065730042424c5f6234323b477265656e6c6561663b436f6e73746f776e2c204b593b007a6f6d626f69640050726f6a656374205a6f6d626f6964000000569600646c0001312e302e302e3000b1853f036ca376b0cd40013b6d6f646465643b7076703b56455253494f4e3a34322e32320038a8010000000000";
    const B41: &str = "ffffffff4911444541444c494e453a204368617074657220332053746f72792d64726976656e20525000444c5f4453455f455850323b444c5f4453455f4558503b4d4320436f6e6e65007a6f6d626f69640050726f6a656374205a6f6d626f6964000000076400646c0101312e302e302e3000b1853f08544522adcd40010038a8010000000000";

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn reads_the_player_count_of_real_answers() {
        assert_eq!(
            info(&bytes(B42)),
            Some(Players {
                online: 86,
                max: 150
            })
        );
        assert_eq!(
            info(&bytes(B41)),
            Some(Players {
                online: 7,
                max: 100
            })
        );
        let cut = bytes(B42);
        assert_eq!(info(&cut[..40]), None, "cut short");
        assert_eq!(info(b"\xff\xff\xff\xffA1234"), None, "a challenge");
    }

    #[test]
    fn splits_a_cache_folder_name() {
        assert_eq!(
            parse("147.185.221.214_45360_0f0d60d8213039534b142b3f8bb8694f"),
            Some(("147.185.221.214", 45360, "0f0d60d8213039534b142b3f8bb8694f"))
        );
        assert_eq!(
            parse("my_host.net_16261_abc"),
            Some(("my_host.net", 16261, "abc"))
        );
        // A coop host's own copy, `<server name>_player`.
        assert_eq!(parse("servertest_player"), None);
        assert_eq!(parse("_16261_abc"), None);
        assert_eq!(parse("-debug_16261_abc"), None, "an option, not a host");
        assert_eq!(parse("a b;c_16261_abc"), None);
        assert_eq!(md5("bob"), "9f9d51bc70ef21ca5c14f307980a29d8");
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-servers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cache(cachedir: &Path, folder: &str, saved: u64) {
        let at = cachedir.join("Saves/Multiplayer").join(folder);
        std::fs::create_dir_all(&at).unwrap();
        let clock = std::fs::File::create(at.join("map_t.bin")).unwrap();
        clock
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(saved))
            .unwrap();
    }

    #[test]
    fn names_build_42_servers_from_its_list() {
        let cachedir = temp("b42");
        // The tables as build 42.22 makes them (`AccountDBHelper`), two accounts on one server.
        std::fs::create_dir_all(cachedir.join("db")).unwrap();
        let db = rusqlite::Connection::open(cachedir.join("db/ServerListSteam.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE server (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
              ip TEXT NOT NULL, port INTEGER NOT NULL, serverPassword TEXT, description TEXT,
              mods TEXT, icon BLOB, banner BLOB, panelBackground BLOB, screenBackground BLOB,
              lastOnline TEXT, lastDataUpdate TEXT);
             CREATE TABLE account (id INTEGER PRIMARY KEY AUTOINCREMENT,
              serverId INTEGER NOT NULL, playerFirstAndLastName TEXT, username TEXT NOT NULL,
              password TEXT, isSavePassword INTEGER DEFAULT 0, isUseSteamRelay INTEGER DEFAULT 0,
              authType INTEGER DEFAULT 1, icon BLOB, timePlayed INTEGER DEFAULT 0,
              lastLogon TEXT, FOREIGN KEY (serverId) REFERENCES server (id));
             INSERT INTO server (name, ip, port, serverPassword, lastOnline)
              VALUES ('teton server', '147.185.221.214', 45360, '', '2026-10-09 23:14:16');
             INSERT INTO account (serverId, playerFirstAndLastName, username, password, isSavePassword)
              VALUES (1, 'Kate Smith', 'alice', 'x', 1), (1, 'Comandante Martinez', 'bob', 'y', 1);",
        )
        .unwrap();
        drop(db);
        cache(
            &cachedir,
            "147.185.221.214_45360_9f9d51bc70ef21ca5c14f307980a29d8",
            2_000,
        );
        // A server that isn't in the list any more.
        cache(
            &cachedir,
            "10.0.0.2_16261_9f9d51bc70ef21ca5c14f307980a29d8",
            1_000,
        );
        cache(&cachedir, "servertest_player", 3_000);

        let found = list(&cachedir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title(), "teton server");
        assert_eq!(found[0].address(), "147.185.221.214:45360");
        assert_eq!(
            found[0].world.character.as_deref(),
            Some("Comandante Martinez"),
            "bob's account, the one the folder names"
        );
        assert_eq!(found[1].title(), "10.0.0.2:16261");
        assert_eq!(found[1].world.character, None);
        // Solo worlds still leave these out.
        assert!(worlds::list(&cachedir).is_empty());
        std::fs::remove_dir_all(&cachedir).unwrap();
    }

    #[test]
    fn names_build_41_servers_and_reads_a_cached_character() {
        let cachedir = temp("b41");
        std::fs::create_dir_all(cachedir.join("Lua")).unwrap();
        // As build 41.78 writes it: one block per server, each from its `name=`.
        std::fs::write(
            cachedir.join("Lua/ServerListSteam.txt"),
            "name=Friends\nip=play.example.net\nlocalip=\nport=16261\ndescription=\n\
             user=bob\npassword=secret\nserverpassword=\nusesteamrelay=false\n\
             name=Other\nip=1.2.3.4\nport=27015\nuser=bob\n",
        )
        .unwrap();
        let folder = "play.example.net_16261_9f9d51bc70ef21ca5c14f307980a29d8";
        cache(&cachedir, folder, 1_000);
        let db = rusqlite::Connection::open(
            cachedir
                .join("Saves/Multiplayer")
                .join(folder)
                .join("players.db"),
        )
        .unwrap();
        db.execute_batch(
            "CREATE TABLE localPlayers (id INTEGER PRIMARY KEY NOT NULL,name STRING,wx INTEGER,\
             wy INTEGER,x FLOAT,y FLOAT,z FLOAT,worldversion INTEGER,data BLOB,isDead BOOLEAN);\
             INSERT INTO localPlayers (name, isDead) VALUES ('Lucas Aaron', 0);",
        )
        .unwrap();
        drop(db);

        let found = list(&cachedir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title(), "Friends");
        assert_eq!(found[0].address(), "play.example.net:16261");
        assert_eq!(found[0].world.character.as_deref(), Some("Lucas Aaron"));
        assert_eq!(found[0].world.saved, 1_000);
        std::fs::remove_dir_all(&cachedir).unwrap();
    }

    #[test]
    fn no_multiplayer_folder_means_no_servers() {
        let cachedir = temp("none");
        assert!(list(&cachedir).is_empty());
        std::fs::remove_dir_all(&cachedir).unwrap();
    }

    /// Asks a public server for its players: `cargo test -p lata -- --ignored asks_a_server`.
    #[test]
    #[ignore]
    fn asks_a_server() {
        let players = players("40.160.20.9", 16261).unwrap();
        assert!(players.max > 0, "{players:?}");
    }
}
