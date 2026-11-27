//! Coop servers an instance hosts, the way the game's own Host button does it: `GameServer
//! -coop` started from the instance's build with its `zomboid/` as the cachedir, talked to over
//! stdin and stdout (the game's `CoopMaster` and `CoopSlave`). Each one is a
//! `<cachedir>/Server/<name>.ini`; its world lives in `Saves/Multiplayer/<name>/`.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::Instance;
use super::mods::Mod;
use super::worlds::{self, World};

/// A server the instance can host.
#[derive(Clone, Debug, PartialEq)]
pub struct Hosted {
    pub name: String,
    /// Its world, once it has started once; the host's own copy when that one has the
    /// screenshot.
    pub world: Option<World>,
}

/// The servers set up in `cachedir`, by name.
pub fn list(cachedir: &Path) -> Vec<Hosted> {
    let Ok(files) = std::fs::read_dir(cachedir.join("Server")) else {
        return Vec::new();
    };
    let saves = cachedir.join("Saves/Multiplayer");
    let mut hosted: Vec<Hosted> = files
        .flatten()
        .filter_map(|file| {
            let path = file.path();
            if path.extension()? != "ini" {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            // The server's world, and the copy the host's game keeps beside it
            // (`<name>_player`, which `servers::list` leaves out): one row for both.
            let world = [saves.join(&name), saves.join(format!("{name}_player"))]
                .iter()
                .filter(|dir| dir.is_dir())
                .map(|dir| worlds::read(dir, "Multiplayer"))
                .reduce(|server, player| World {
                    thumb: server.thumb.clone().or(player.thumb),
                    saved: server.saved.max(player.saved),
                    character: server.character.clone().or(player.character),
                    ..server
                });
            Some(Hosted { name, world })
        })
        .collect();
    hosted.sort_by(|a, b| a.name.cmp(&b.name));
    hosted
}

/// Whether `name` can name a new server: it becomes a file name and a command line argument,
/// and must not read as a joined server's `<host>_<port>_<hash>` folder.
pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Sets up a new server called `name`; its settings fill in when it first starts.
pub fn create(cachedir: &Path, name: &str) -> io::Result<()> {
    let dir = cachedir.join("Server");
    std::fs::create_dir_all(&dir)?;
    let ini = dir.join(format!("{name}.ini"));
    if ini.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            t!("There's already a server with that name"),
        ));
    }
    std::fs::write(ini, "")
}

/// What the server loads, from the instance's active mods in their order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub mods: Vec<String>,
    /// Workshop items, which the server downloads and tells joining players to download.
    pub items: Vec<u64>,
    pub maps: Vec<String>,
    /// Names of active mods that aren't on the Workshop: friends can't get them.
    pub local: Vec<String>,
}

/// The game's own map, which the mods' maps go in front of.
const MAP: &str = "Muldraugh, KY";

pub fn plan(instance: &Instance) -> Plan {
    let active = super::mods::active(instance);
    plan_from(&active.mods, &active.maps, &super::mods::list(instance))
}

fn plan_from(active: &[String], maps: &[String], installed: &[Mod]) -> Plan {
    let mut plan = Plan::default();
    for id in active {
        // An id left in `default.txt` after its mod went: the server would fail on it.
        let Some(found) = installed.iter().find(|m| &m.id == id) else {
            continue;
        };
        plan.mods.push(id.clone());
        match found.workshop {
            Some(item) if !plan.items.contains(&item) => plan.items.push(item),
            Some(_) => {}
            None => plan.local.push(found.name.clone()),
        }
    }
    plan.maps = maps.to_vec();
    if !plan.maps.iter().any(|map| map == MAP) {
        plan.maps.push(MAP.to_owned());
    }
    plan
}

/// `existing` (the server's ini) with `Mods=`, `WorkshopItems=` and `Map=` set to `plan` and
/// every other line kept. Build 42 wants a backslash before each mod id.
fn ini(existing: &str, plan: &Plan, modern: bool) -> String {
    let prefix = if modern { "\\" } else { "" };
    let mods: Vec<String> = plan.mods.iter().map(|id| format!("{prefix}{id}")).collect();
    let items: Vec<String> = plan.items.iter().map(u64::to_string).collect();
    let mut wanted = [
        ("Mods", mods.join(";")),
        ("WorkshopItems", items.join(";")),
        ("Map", plan.maps.join(";")),
    ]
    .map(|(key, value)| (key, Some(value)));
    let mut out = String::new();
    for line in existing.lines() {
        let key = line.split_once('=').map(|(key, _)| key);
        match wanted.iter_mut().find(|(want, _)| Some(*want) == key) {
            Some((key, value)) => {
                // A key twice: the first carries the value, the rest go.
                if let Some(value) = value.take() {
                    out.push_str(&format!("{key}={value}\n"));
                }
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    for (key, value) in wanted {
        if let Some(value) = value {
            out.push_str(&format!("{key}={value}\n"));
        }
    }
    out
}

/// Writes `plan` into the server's ini, keeping the rest of what it has.
pub fn write(cachedir: &Path, name: &str, plan: &Plan, modern: bool) -> io::Result<()> {
    let path = cachedir.join("Server").join(format!("{name}.ini"));
    // Only a missing ini starts empty: one that can't be read (not UTF-8, say, after an edit
    // in another encoding) would otherwise be replaced by the three lines, losing the rest.
    let existing = match std::fs::read_to_string(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        read => read?,
    };
    std::fs::write(&path, ini(&existing, plan, modern))
}

/// The game's launcher file, which lists its classpath and Java options.
#[derive(serde::Deserialize)]
struct Launcher {
    classpath: Vec<String>,
    #[serde(rename = "vmArgs")]
    vm_args: Vec<String>,
}

/// The command `CoopMaster.launchServer` builds: the build's own Java with its classpath and
/// options (Build 41 lists loose jars and `linux64/`, Build 42 one jar, `natives/` and module
/// flags; both come from `ProjectZomboid64.json`), run from the build's folder.
// ponytail: with Steam the server downloads `WorkshopItems` into `<build>/steamapps/workshop`,
// inside the build instances share. It is a per-id cache all of them reuse; a hardlink tree per
// instance (`cp -al`) would keep builds untouched if that ever matters.
// The memory override in Settings wins over the instance's own here too.
pub fn command(instance: &Instance, name: &str) -> io::Result<Command> {
    build(&crate::settings::overrides::current().apply(instance), name)
}

/// [`command`] for the instance as given.
fn build(instance: &Instance, name: &str) -> io::Result<Command> {
    if cfg!(target_os = "macos") {
        // ponytail: the app bundle's layout isn't mapped yet; Linux and Windows are.
        return Err(io::Error::other(t!("Hosting isn't ready on macOS yet")));
    }
    let game = instance
        .game()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, t!("the game isn't installed")))?;
    // Linux builds keep the game in a `projectzomboid` folder beside the start script.
    let root = [game.join("projectzomboid"), game.clone()]
        .into_iter()
        .find(|dir| dir.join("ProjectZomboid64.json").is_file())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, t!("the game files look broken")))?;
    let launcher: Launcher = serde_json::from_str(&std::fs::read_to_string(
        root.join("ProjectZomboid64.json"),
    )?)
    .map_err(io::Error::other)?;
    let java = root.join(if cfg!(windows) {
        "jre64/bin/java.exe"
    } else {
        "jre64/bin/java"
    });
    let separator = if cfg!(windows) { ";" } else { ":" };

    let mut command = match pdeathsig() {
        // Ends the server with lata, even when lata crashes: a Build 42 server without players
        // never notices its master is gone. TERM saves the world first.
        Some(setpriv) => {
            let mut command = Command::new(setpriv);
            command.args(["--pdeathsig", "TERM", "--"]).arg(&java);
            command
        }
        None => Command::new(&java),
    };
    command
        .args(
            launcher
                .vm_args
                .iter()
                .filter(|arg| !arg.starts_with("-Xmx") && !arg.starts_with("-Xms")),
        )
        .arg(format!("-Xmx{}g", instance.memory_gb.unwrap_or(4)))
        .arg(format!(
            "-Djava.class.path={}",
            launcher.classpath.join(separator)
        ))
        .arg(format!("-Duser.dir={}", root.display()))
        .args(["zombie.network.GameServer", "-coop", "-servername", name])
        // With Steam the admin is whoever hosts; the password has to be there and is unused.
        .args(["-adminusername", "admin", "-adminpassword", &password()])
        .arg(super::launch::cachedir(instance))
        .current_dir(&root);
    if cfg!(target_os = "linux") {
        let mut libs = vec![
            root.join("natives"),
            root.join("linux64"),
            root.clone(),
            root.join("jre64/lib"),
        ];
        // Kept after ours, entry by entry: as one path its `:` would make joining fail.
        if let Some(inherited) = std::env::var_os("LD_LIBRARY_PATH") {
            libs.extend(std::env::split_paths(&inherited));
        }
        command.env(
            "LD_LIBRARY_PATH",
            std::env::join_paths(libs).map_err(io::Error::other)?,
        );
    }
    Ok(command)
}

/// `setpriv`, when the system has it (util-linux): it sets the parent-death signal and runs
/// the server, which lata can't do itself without `unsafe`. Windows does it with a job (see
/// [`start`]).
// ponytail: macOS has neither pdeathsig nor jobs; there a server outlives a crashed lata until
// its host leaves (B42 ignores stdin's EOF). A tiny watcher process polling `getppid()` would
// do it, if hosting on macOS ever lands.
fn pdeathsig() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("setpriv"))
        .find(|path| path.is_file())
}

fn password() -> String {
    use std::hash::{BuildHasher, Hasher};
    let random = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{:016x}{:016x}", random(), random())
}

/// What the server says on stdout that lata cares about.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Where it is in starting up: a key like `UI_ServerStatus_LoadingWorld`, or a text the
    /// game already translated (`Descargados 1 de 1`).
    Status(String),
    /// `ip:port`; the port is what `+connect` takes after the Steam ID.
    Address(String),
    /// The server's Steam ID; new on every start.
    SteamId(u64),
}

pub const STARTED: &str = "UI_ServerStatus_Started";
pub const TERMINATED: &str = "UI_ServerStatus_Terminated";

/// Splits a line the way `CoopSlave` does, `^([\-\w]+)(\[(\d+)\])?@(.*)$`, into its tag and
/// value. Anything else on stdout is Steam's own noise.
fn tagged(line: &str) -> Option<(&str, &str)> {
    let (head, value) = line.split_once('@')?;
    let tag = match head.split_once('[') {
        Some((tag, cookie)) => {
            let digits = cookie.strip_suffix(']')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            tag
        }
        None => head,
    };
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    (!tag.is_empty() && tag.chars().all(word)).then_some((tag, value))
}

fn parse(line: &str) -> Option<Event> {
    match tagged(line)? {
        ("status", status) => Some(Event::Status(status.to_owned())),
        ("server-address", address) => Some(Event::Address(address.to_owned())),
        ("steam-id", id) => id.parse().ok().map(Event::SteamId),
        _ => None,
    }
}

/// A status as the row shows it.
pub fn friendly(status: &str) -> String {
    let Some(key) = status.strip_prefix("UI_ServerStatus_") else {
        return status.to_owned();
    };
    match key {
        "Initialising" => t!("Getting things ready"),
        "Downloading_Workshop_Items" => t!("Downloading mods"),
        "Requesting_Workshop_Item_Details" => t!("Asking Steam about the mods"),
        "CreateBackup" => t!("Backing up the world"),
        "Loading_Assets" => t!("Loading the game's files"),
        "Initing_Checksum" => t!("Checking the files"),
        "Loading_Sandbox_Vars" => t!("Reading the world settings"),
        "LoadingWorld" => t!("Loading the world"),
        "Started" => t!("Up and running"),
        "Terminated" => t!("Shutting down"),
        _ => t!("Getting things ready"),
    }
    .to_owned()
}

/// The 64-bit Steam ID in what someone pasted: the number, or a profile link
/// (`steamcommunity.com/profiles/<id>`).
pub fn steam_id(text: &str) -> Option<u64> {
    let text = text.trim().trim_end_matches('/');
    let id = match text.split_once("/profiles/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next()?,
        None => text,
    };
    let id: u64 = id.parse().ok()?;
    // A person's account: universe 1 (public), type 1 (individual), instance 1.
    (id >> 32 == 0x0110_0001).then_some(id)
}

/// A running server lata started.
pub struct Server {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    /// When Stop was pressed.
    stopping: Option<Instant>,
    /// When it said it was shutting down by itself.
    terminated: Option<Instant>,
    termed: bool,
    /// The job the server runs in on Windows: closing it, as lata ending does, kills the server.
    #[cfg(windows)]
    job: Option<win32job::Job>,
}

/// How long a stop waits for `quit` before TERM, and then for TERM before killing. Build 42
/// can take minutes to read stdin after starting; TERM saves the world just the same.
const QUIT_WAIT: Duration = Duration::from_secs(15);
const TERM_WAIT: Duration = Duration::from_secs(60);
/// A server that failed to load can hang after saying it shuts down.
const HANG_WAIT: Duration = Duration::from_secs(30);

/// Starts `command` and hands each [`Event`] to `events` from a thread of its own, which also
/// answers the server's pings.
pub fn start(mut command: Command, events: impl Fn(Event) + Send + 'static) -> io::Result<Server> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    #[cfg(windows)]
    let job = bind(&child);
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("piped")));
    let stdout = child.stdout.take().expect("piped");
    let answer = stdin.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        while reader.read_until(b'\n', &mut bytes).is_ok_and(|n| n > 0) {
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end();
            if matches!(tagged(line), Some(("ping", _))) {
                let _ = send(&answer, "ping@pong");
            } else if let Some(event) = parse(line) {
                events(event);
            }
            bytes.clear();
        }
    });
    Ok(Server {
        child,
        stdin,
        stopping: None,
        terminated: None,
        termed: false,
        #[cfg(windows)]
        job,
    })
}

/// Puts the server in a job that kills it once the job's last handle closes, which is when
/// lata ends, even by crashing: a Build 42 server without players never notices on its own.
#[cfg(windows)]
fn bind(child: &Child) -> Option<win32job::Job> {
    use std::os::windows::io::AsRawHandle;
    let mut info = win32job::ExtendedLimitInfo::new();
    info.limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&info).ok()?;
    job.assign_process(child.as_raw_handle() as isize).ok()?;
    Some(job)
}

fn send(stdin: &Mutex<ChildStdin>, line: &str) -> io::Result<()> {
    let mut stdin = stdin.lock().map_err(|_| io::Error::other("poisoned"))?;
    writeln!(stdin, "{line}")?;
    stdin.flush()
}

impl Server {
    /// A line for the server: a `tag@value` message, or else an admin command.
    pub fn send(&self, line: &str) -> io::Result<()> {
        send(&self.stdin, line)
    }

    /// Asks it to save and quit; [`Server::tick`] gets firmer if it doesn't.
    pub fn stop(&mut self) {
        let _ = self.send("quit");
        self.stopping.get_or_insert_with(Instant::now);
    }

    /// TERM: the server's shutdown hook saves the world. On Windows that hook doesn't run,
    /// so there `quit` is the way and this waits for the kill; it lets go of the job instead,
    /// so a `quit` sent as lata quits can still save before the server exits.
    pub fn terminate(&mut self) {
        self.termed = true;
        #[cfg(windows)]
        if let Some(job) = &self.job {
            let _ = job.set_extended_limit_info(&win32job::ExtendedLimitInfo::new());
        }
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    /// It said it was shutting down.
    pub fn terminated(&mut self) {
        self.terminated.get_or_insert_with(Instant::now);
    }

    /// Call it every second or so: how it exited once it has, after stepping up a stop that
    /// takes too long or ending a server that hung.
    pub fn tick(&mut self) -> Option<ExitStatus> {
        if let Ok(Some(status)) = self.child.try_wait() {
            return Some(status);
        }
        let waited = |since: Option<Instant>, wait| since.is_some_and(|at| at.elapsed() >= wait);
        if waited(self.stopping, QUIT_WAIT) && !self.termed {
            self.terminate();
        }
        if waited(self.stopping, QUIT_WAIT + TERM_WAIT) || waited(self.terminated, HANG_WAIT) {
            let _ = self.child.kill();
        }
        None
    }
}

/// Where a hosted server stands, as its row shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// With what it last said it was doing.
    Starting(String),
    Running,
    Stopping,
    Stopped,
    /// It ended before it was up, with what it was doing then.
    Failed(String),
}

/// The server lata hosts: one at a time, since two would fight over the game's port.
pub struct Hosting {
    pub instance: gpui_kit::SharedString,
    pub name: String,
    pub state: State,
    pub steam_id: Option<u64>,
    pub port: Option<u16>,
    /// Friends invited this time; the server forgets them when it stops.
    pub invited: Vec<u64>,
    /// `None` once it has exited.
    pub server: Option<Server>,
}

impl Hosting {
    pub fn new(instance: gpui_kit::SharedString, name: String, server: Server) -> Self {
        Self {
            instance,
            name,
            state: State::Starting(friendly("UI_ServerStatus_Initialising")),
            steam_id: None,
            port: None,
            invited: Vec::new(),
            server: Some(server),
        }
    }

    /// Takes in what the server said; true when that means it just came up.
    pub fn heard(&mut self, event: Event) -> bool {
        match event {
            Event::SteamId(id) => self.steam_id = Some(id),
            Event::Address(address) => {
                self.port = address.rsplit_once(':').and_then(|(_, p)| p.parse().ok());
            }
            Event::Status(status) if status == STARTED => {
                if matches!(self.state, State::Starting(_)) {
                    self.state = State::Running;
                    return true;
                }
            }
            Event::Status(status) if status == TERMINATED => {
                if let Some(server) = &mut self.server {
                    server.terminated();
                }
            }
            Event::Status(status) => {
                if let State::Starting(_) = self.state {
                    self.state = State::Starting(friendly(&status));
                }
            }
        }
        false
    }

    /// The server's process ended.
    pub fn exited(&mut self) {
        self.server = None;
        self.state = match std::mem::replace(&mut self.state, State::Stopped) {
            State::Starting(doing) => State::Failed(doing),
            _ => State::Stopped,
        };
    }

    /// Whether its process is still there.
    pub fn alive(&self) -> bool {
        self.server.is_some()
    }

    /// What the game's `+connect` takes to join it: the Steam ID and a port, as the game's
    /// own invites send it (a bare Steam ID doesn't parse). Once it is up.
    pub fn connect(&self) -> Option<String> {
        let id = self.steam_id.filter(|_| self.state == State::Running)?;
        Some(format!("{id}:{}", self.port.unwrap_or(16261)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_a_server_from_start_to_stop() {
        // A stand-in that stays up, so the state machine runs on a real process.
        let mut sleep = Command::new("sleep");
        sleep.arg("30");
        let mut hosting = Hosting::new("a".into(), "coop".into(), start(sleep, |_| {}).unwrap());
        assert!(!hosting.heard(Event::Status("UI_ServerStatus_LoadingWorld".into())));
        assert_eq!(hosting.state, State::Starting("Loading the world".into()));
        hosting.heard(Event::Address("201.188.193.153:16261".into()));
        hosting.heard(Event::SteamId(90298236957966342));
        assert_eq!(hosting.connect(), None, "not up yet");
        assert!(hosting.heard(Event::Status(STARTED.into())));
        assert_eq!(
            hosting.connect().as_deref(),
            Some("90298236957966342:16261")
        );
        // The host left and it shut down by itself.
        hosting.heard(Event::Status(TERMINATED.into()));
        hosting.server.as_mut().unwrap().child.kill().unwrap();
        hosting.server.as_mut().unwrap().child.wait().unwrap();
        assert!(hosting.server.as_mut().unwrap().tick().is_some());
        hosting.exited();
        assert_eq!(hosting.state, State::Stopped);
        assert!(!hosting.alive());
    }

    #[test]
    fn a_server_that_ends_while_starting_failed() {
        let mut hosting = Hosting::new(
            "a".into(),
            "coop".into(),
            start(Command::new("true"), |_| {}).unwrap(),
        );
        hosting.heard(Event::Status("UI_ServerStatus_Loading_Assets".into()));
        hosting.exited();
        assert_eq!(
            hosting.state,
            State::Failed("Loading the game's files".into())
        );
    }

    #[test]
    fn answers_pings_and_passes_on_events() {
        // A stand-in server: says it's up, pings, and echoes what comes back.
        let mut fake = Command::new("sh");
        fake.args([
            "-c",
            "echo noise; echo steam-id@90298236957966342; echo status@UI_ServerStatus_Started; \
             echo ping@ping; read answer; echo \"status@$answer\"",
        ]);
        let (to, from) = std::sync::mpsc::channel();
        let mut server = start(fake, move |event| to.send(event).unwrap()).unwrap();
        let heard: Vec<Event> = from.iter().take(3).collect();
        assert_eq!(
            heard,
            [
                Event::SteamId(90298236957966342),
                Event::Status(STARTED.into()),
                Event::Status("ping@pong".into()),
            ]
        );
        while server.tick().is_none() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn reads_what_real_servers_said() {
        // Lines from Build 42.22 and 41.78 coop servers started the way the game does.
        assert_eq!(parse("info@coop mode enabled"), None);
        assert_eq!(
            parse("status@UI_ServerStatus_Initialising"),
            Some(Event::Status("UI_ServerStatus_Initialising".into()))
        );
        assert_eq!(
            parse("status@Cantidad de elementos del Workshop descargados: 1"),
            Some(Event::Status(
                "Cantidad de elementos del Workshop descargados: 1".into()
            ))
        );
        assert_eq!(
            parse("server-address@201.188.193.153:16261"),
            Some(Event::Address("201.188.193.153:16261".into()))
        );
        assert_eq!(
            parse("steam-id@90298236957966342"),
            Some(Event::SteamId(90298236957966342))
        );
        assert_eq!(
            tagged("get-parameter[1000001]@90298236957966342"),
            Some(("get-parameter", "90298236957966342"))
        );
        assert_eq!(tagged("ping@ping"), Some(("ping", "ping")));
        // Steam's noise, which shares stdout.
        for noise in [
            "dlopen failed trying to load:",
            "steamclient.so: no se puede abrir el fichero del objeto compartido",
            "[S_API] SteamAPI_Init(): Loaded '/home/v/.local/share/Steam/linux64/steamclient.so' OK.",
            "11/25 00:09:50 minidumps folder is set to /tmp/dumps",
            "SteamInternal_SetMinidumpSteamID:  Caching Steam ID:  76561197960265728 [API loaded no]",
            "[S_API FAIL] Tried to access Steam interface SteamNetworkingUtils004 before SteamAPI_Init succeeded.",
            "user name@example.com",
            "tag[x]@1",
        ] {
            assert_eq!(parse(noise), None, "{noise}");
        }
        assert_eq!(
            friendly("UI_ServerStatus_LoadingWorld"),
            "Loading the world"
        );
        assert_eq!(friendly("Descargados 1 de 1"), "Descargados 1 de 1");
    }

    #[test]
    fn reads_a_friends_steam_id() {
        let id = Some(76561197960287930);
        assert_eq!(steam_id("76561197960287930"), id);
        assert_eq!(
            steam_id(" https://steamcommunity.com/profiles/76561197960287930/ "),
            id
        );
        assert_eq!(
            steam_id("steamcommunity.com/profiles/76561197960287930/?xml=1"),
            id
        );
        // A server's own ID, a vanity link, and junk.
        assert_eq!(steam_id("90298236957966342"), None);
        assert_eq!(
            steam_id("https://steamcommunity.com/id/gabelogannewell"),
            None
        );
        assert_eq!(steam_id("hello"), None);
    }

    #[test]
    fn checks_new_server_names() {
        assert!(valid_name("coop"));
        assert!(valid_name("Friday-Night2"));
        assert!(!valid_name(""));
        assert!(!valid_name("a_16261_b"), "reads as a joined server");
        assert!(!valid_name("../x"));
        assert!(!valid_name("two words"));
        assert!(!valid_name(&"x".repeat(33)));
    }

    fn installed(id: &str, name: &str, workshop: Option<u64>) -> Mod {
        Mod {
            id: id.into(),
            name: name.into(),
            author: String::new(),
            description: String::new(),
            folder: id.into(),
            dir: PathBuf::new(),
            version: None,
            workshop,
            manifest: None,
            held: false,
            java: false,
            icon: None,
            requires: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            enabled: true,
            added: None,
            updated: None,
        }
    }

    #[test]
    fn plans_from_the_active_mods_in_order() {
        let mods = [
            installed("SartekWhatAThrill_SMB", "Ladder SMB", Some(3837899636)),
            installed("SartekWhatAThrill", "Ladder", Some(3837899636)),
            installed("Mine", "My own mod", None),
            installed("Off", "Not active", Some(1)),
        ];
        let active =
            ["SartekWhatAThrill", "Gone", "SartekWhatAThrill_SMB", "Mine"].map(String::from);
        let plan = plan_from(&active, &["Bedford Falls".into()], &mods);
        assert_eq!(
            plan,
            Plan {
                mods: ["SartekWhatAThrill", "SartekWhatAThrill_SMB", "Mine"]
                    .map(String::from)
                    .into(),
                items: vec![3837899636],
                maps: vec!["Bedford Falls".into(), MAP.into()],
                local: vec!["My own mod".into()],
            }
        );
    }

    #[test]
    fn writes_only_the_mod_lines_of_an_ini() {
        let plan = Plan {
            mods: vec!["FarmingZone".into(), "LataCoopTest".into()],
            items: vec![3838784593],
            maps: vec![MAP.into()],
            local: Vec::new(),
        };
        // An excerpt of the ini a Build 42.22 coop server wrote.
        let existing = "# Players can hurt and kill other players\nPVP=true\nDefaultPort=16261\n\
                        Mods=\nMap=Muldraugh, KY\nPublic=false\nWorkshopItems=\nMods=old\n";
        assert_eq!(
            ini(existing, &plan, true),
            "# Players can hurt and kill other players\nPVP=true\nDefaultPort=16261\n\
             Mods=\\FarmingZone;\\LataCoopTest\nMap=Muldraugh, KY\nPublic=false\n\
             WorkshopItems=3838784593\n"
        );
        // A new server's empty ini gets the three lines; Build 41 has no backslashes.
        assert_eq!(
            ini("", &plan, false),
            "Mods=FarmingZone;LataCoopTest\nWorkshopItems=3838784593\nMap=Muldraugh, KY\n"
        );
    }

    #[test]
    fn leaves_an_ini_it_cant_read_alone() {
        let dir = std::env::temp_dir().join(format!("lata-host-ini-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir, "coop").unwrap();
        // Saved as Latin-1 by an editor: not UTF-8.
        let latin1 = b"PublicName=Caf\xe9\nPVP=false\n";
        let ini = dir.join("Server/coop.ini");
        std::fs::write(&ini, latin1).unwrap();
        assert!(write(&dir, "coop", &Plan::default(), true).is_err());
        assert_eq!(std::fs::read(&ini).unwrap(), latin1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_servers_once_with_the_hosts_copy() {
        let dir = std::env::temp_dir().join(format!("lata-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir, "coop").unwrap();
        assert!(create(&dir, "coop").is_err(), "taken");
        std::fs::write(dir.join("Server/coop_SandboxVars.lua"), "").unwrap();
        std::fs::create_dir_all(dir.join("Saves/Multiplayer/coop")).unwrap();
        std::fs::create_dir_all(dir.join("Saves/Multiplayer/coop_player")).unwrap();
        std::fs::write(dir.join("Saves/Multiplayer/coop_player/thumb.png"), "").unwrap();
        create(&dir, "fresh").unwrap();

        let found = list(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "coop");
        let world = found[0].world.as_ref().unwrap();
        assert_eq!(world.name, "coop");
        assert!(
            world
                .thumb
                .as_ref()
                .unwrap()
                .ends_with("coop_player/thumb.png")
        );
        assert_eq!(found[1].world, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The whole flow on a real server, like the shell drives it: `LATA_COOP_INSTANCE` is an
    /// instance folder whose build is installed, `LATA_COOP_HOST` the host's SteamID64.
    /// Takes up to half an hour on Build 42.
    #[test]
    #[ignore]
    fn hosts_for_real() {
        let dir = std::path::PathBuf::from(std::env::var("LATA_COOP_INSTANCE").unwrap());
        let host: u64 = std::env::var("LATA_COOP_HOST").unwrap().parse().unwrap();
        let root = dir.parent().unwrap();
        let instance = crate::instance::store::list(root)
            .into_iter()
            .find(|i| i.dir == dir)
            .expect("an instance");
        let cachedir = dir.join("zomboid");
        let name = "e2e";
        if !list(&cachedir).iter().any(|h| h.name == name) {
            create(&cachedir, name).unwrap();
        }
        let modern = crate::instance::mods::is_modern(&instance);
        write(&cachedir, name, &plan(&instance), modern).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = start(command(&instance, name).unwrap(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let mut hosting = Hosting::new(instance.id.clone(), name.into(), server);
        let began = Instant::now();
        let mut stop_at = None;
        let mut game: Option<std::process::Child> = None;
        loop {
            while let Ok(event) = rx.try_recv() {
                eprintln!("[{:>4}s] {event:?}", began.elapsed().as_secs());
                if hosting.heard(event) {
                    let server = hosting.server.as_ref().unwrap();
                    server.send("set-host-user@lata").unwrap();
                    server.send(&format!("set-host-steamid@{host}")).unwrap();
                    server.send("invite-add@76561197960287930").unwrap();
                    server.send("players").unwrap();
                    let connect = hosting.connect().unwrap();
                    eprintln!("connect: {connect}");
                    // With `LATA_COOP_PLAY`, the host's game joins like Play does, and gets
                    // time to load before the stop.
                    let play = std::env::var_os("LATA_COOP_PLAY").is_some();
                    if play {
                        game = Some(
                            crate::instance::launch::start(&instance, Some(&connect)).unwrap(),
                        );
                    }
                    // Room for B42 to read stdin, which lags minutes behind Started.
                    let wait = if play { 900 } else { 360 };
                    stop_at = Some(Instant::now() + Duration::from_secs(wait));
                }
            }
            // The host leaving closes the server by itself; `quit` only if it doesn't.
            if stop_at.is_some_and(|at| Instant::now() >= at)
                && let Some(mut game) = game.take()
            {
                eprintln!("[{:>4}s] host leaves", began.elapsed().as_secs());
                let _ = game.kill();
                crate::instance::launch::stop(&instance);
                stop_at = Some(Instant::now() + Duration::from_secs(180));
            }
            if stop_at.is_some_and(|at| Instant::now() >= at) && hosting.state == State::Running {
                eprintln!("[{:>4}s] stop", began.elapsed().as_secs());
                hosting.state = State::Stopping;
                hosting.server.as_mut().unwrap().stop();
            }
            if let Some(status) = hosting.server.as_mut().and_then(Server::tick) {
                eprintln!("[{:>4}s] exited {status}", began.elapsed().as_secs());
                hosting.exited();
                break;
            }
            assert!(began.elapsed() < Duration::from_secs(45 * 60), "too slow");
            std::thread::sleep(Duration::from_secs(1));
        }
        if let Some(mut game) = game {
            let _ = game.kill();
        }
        assert_eq!(hosting.state, State::Stopped, "came up and went down");
        let console = std::fs::read_to_string(cachedir.join("coop-console.txt")).unwrap();
        eprintln!("{}", &console[console.len().saturating_sub(3000)..]);
    }
}
