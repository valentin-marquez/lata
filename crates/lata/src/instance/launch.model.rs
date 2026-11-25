//! Starting Project Zomboid from an instance's own game copy, with its own cachedir.

use std::io;
use std::path::Path;
use std::process::{Child, Command};

use super::{APP, Instance};

/// The command that starts the instance's game files with saves, mods and options in `zomboid/`.
/// With `connect` (`host:port`), the game skips its menu for its dialog to join that server,
/// filled with the account and password it saved for it.
pub fn command(instance: &Instance, connect: Option<&str>) -> io::Result<Command> {
    let game = instance
        .game()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the game isn't installed"))?;
    let missing = |what: &Path| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is missing; install the instance again", what.display()),
        )
    };

    let mut command = if cfg!(target_os = "macos") {
        let app = game.join("Project Zomboid.app");
        if !app.exists() {
            return Err(missing(&app));
        }
        // `open -W` waits for the app to quit, so the running state stays accurate.
        let mut open = Command::new("open");
        open.arg("-W").arg("-a").arg(app).arg("--args");
        open
    } else {
        let exe = game.join(if cfg!(windows) {
            "ProjectZomboid64.exe"
        } else {
            "projectzomboid.sh"
        });
        if !exe.exists() {
            return Err(missing(&exe));
        }
        Command::new(exe)
    };
    // The launcher takes Java VM options first, closed by `--`; a later `-Xmx` wins, so one
    // typed by hand overrides the memory setting. Both builds accept this on the command line
    // (checked with `jcmd VM.flags`), so the shared build's `ProjectZomboid64.json` stays as is.
    let mut jvm: Vec<String> = instance
        .memory_gb
        .map(|gb| format!("-Xmx{gb}g"))
        .into_iter()
        .collect();
    // Java mods, when the user turned them on here: the loader hooks into the VM.
    jvm.extend(super::java::arg(instance));
    jvm.extend(split_args(&instance.jvm_args));
    if !jvm.is_empty() {
        command.args(jvm).arg("--");
    }
    // Before `-cachedir`, so lata's own folder wins if the user types one too. Mods load only
    // from the instance's own `zomboid/mods`, not from what Steam has subscribed: that keeps
    // instances apart, and the same mod in both places would clash.
    command
        .args(split_args(&instance.launch_args))
        .args(["-modfolders", "mods"])
        .arg(cachedir(instance))
        .current_dir(&game);
    if let Some(address) = connect {
        command.args(["+connect", address]);
    }
    Ok(command)
}

/// The argument that points the game at the instance's `zomboid/`; it tells which instance a
/// running game belongs to.
fn cachedir(instance: &Instance) -> String {
    format!("-cachedir={}", instance.dir.join("zomboid").display())
}

/// Whether the instance's game is running, whether lata started it in this run or not.
pub fn alive(instance: &Instance) -> bool {
    games(instance, |_| ())
}

/// Ends the instance's game. On Linux that is the script and the game it started: killing
/// only the script lata spawned would leave the game open.
pub fn stop(instance: &Instance) {
    games(instance, |p| {
        p.kill();
    });
}

/// Calls `f` on each process started with the instance's cachedir; true if there was one.
fn games(instance: &Instance, mut f: impl FnMut(&sysinfo::Process)) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let arg = cachedir(instance);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
    );
    let mut found = false;
    for p in system.processes().values() {
        if p.thread_kind().is_none() && p.cmd().iter().any(|a| *a == *arg) {
            f(p);
            found = true;
        }
    }
    found
}

/// Splits arguments on spaces; quotes group words and are dropped, so a path can hold spaces.
// shortcut: no backslash escapes, so Windows paths stay as typed; add them if a flag ever needs
// a literal quote.
fn split_args(text: &str) -> Vec<String> {
    let (mut args, mut word) = (Vec::new(), String::new());
    let (mut quote, mut started) = (None, false);
    for c in text.chars() {
        match (c, quote) {
            ('"' | '\'', None) => {
                quote = Some(c);
                started = true;
            }
            (c, Some(open)) if c == open => quote = None,
            (c, None) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (c, _) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(word);
    }
    args
}

/// Starts the game, joining the server at `connect` when there is one (see [`command`]).
// ponytail: the game's output is dropped; the Logs tab (#16) will read it.
pub fn start(instance: &Instance, connect: Option<&str>) -> io::Result<Child> {
    // The game ships with `-Dzomboid.steam=1` and quits silently when SteamAPI_Init fails.
    if !steam_running() {
        return Err(io::Error::other(
            "Steam isn't open. Open Steam and press Play again",
        ));
    }
    super::java::prepare(instance)?;
    // Sharing characters is a nicety: a preset that didn't copy shouldn't keep the game shut.
    let _ = crate::characters::push(&instance.dir);
    command(instance, connect)?
        // Lets the Steam API find the app when the game isn't started by Steam itself.
        .env("SteamAppId", APP.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

/// Whether the Steam client is open.
pub fn steam_running() -> bool {
    running(STEAM)
}

const STEAM: &str = if cfg!(windows) {
    "steam.exe"
} else if cfg!(target_os = "macos") {
    "steam_osx"
} else {
    "steam"
};

/// Whether a process with this name is running.
#[cfg(target_os = "linux")]
fn running(name: &str) -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs.flatten().any(|p| {
        std::fs::read_to_string(p.path().join("comm")).is_ok_and(|comm| comm.trim_end() == name)
    })
}

#[cfg(target_os = "macos")]
fn running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(windows)]
fn running(name: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains(name)
        })
}

#[cfg(test)]
mod split_tests {
    use super::split_args;

    #[test]
    fn splits_on_spaces_and_groups_quotes() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("  -debug   -nosteam "), ["-debug", "-nosteam"]);
        assert_eq!(
            split_args(r#"-pzexeconfig "C:\My Games\pz.json" -x"#),
            ["-pzexeconfig", r"C:\My Games\pz.json", "-x"]
        );
        assert_eq!(split_args("-a='b c' d"), ["-a=b c", "d"]);
        assert_eq!(split_args(r#"-a "" b"#), ["-a", "", "b"]);
        assert_eq!(split_args(r#"-a "open"#), ["-a", "open"]);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn runs_the_instance_copy_with_its_cachedir() {
        let dir = std::env::temp_dir().join(format!("lata-launch-{}", std::process::id()));
        let instance = Instance {
            id: "a".into(),
            dir: dir.clone(),
            name: "A".into(),
            branch: "public".into(),
            build: Some(1),
            last_played: None,
            icon: None,
            memory_gb: Some(8),
            jvm_args: "-XX:+AlwaysPreTouch".into(),
            launch_args: "-debug".into(),
        };
        assert!(command(&instance, None).is_err(), "no game files yet");

        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::write(dir.join("game/projectzomboid.sh"), "#!/bin/sh\n").unwrap();
        let cmd = command(&instance, None).unwrap();
        assert_eq!(
            cmd.get_program(),
            dir.join("game/projectzomboid.sh").as_os_str()
        );
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-Xmx8g".to_owned(),
                "-XX:+AlwaysPreTouch".to_owned(),
                "--".to_owned(),
                "-debug".to_owned(),
                "-modfolders".to_owned(),
                "mods".to_owned(),
                format!("-cachedir={}", dir.join("zomboid").display())
            ]
        );
        assert_eq!(cmd.get_current_dir(), Some(dir.join("game").as_path()));
        // Joining a server adds what `MainScreenState` reads from the command line.
        let join = command(&instance, Some("40.160.20.9:16261")).unwrap();
        let args: Vec<_> = join.get_args().collect();
        assert_eq!(args[args.len() - 2..], ["+connect", "40.160.20.9:16261"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_and_stops_a_game_by_its_cachedir() {
        let dir = std::env::temp_dir().join(format!("lata-alive-{}", std::process::id()));
        let instance = Instance {
            id: "a".into(),
            dir,
            name: "A".into(),
            branch: "public".into(),
            build: None,
            last_played: None,
            icon: None,
            memory_gb: None,
            jvm_args: String::new(),
            launch_args: String::new(),
        };
        assert!(!alive(&instance));
        // A stand-in for the game: any process carrying the argument.
        let mut game = Command::new("sh")
            .args([
                "-c",
                "while :; do sleep 1; done",
                "sh",
                &cachedir(&instance),
            ])
            .spawn()
            .unwrap();
        assert!(alive(&instance));
        stop(&instance);
        game.wait().unwrap();
        assert!(!alive(&instance));
    }

    #[test]
    fn finds_running_processes_by_name() {
        let me = std::fs::read_to_string("/proc/self/comm").unwrap();
        assert!(running(me.trim_end()));
        assert!(!running("no-such-process"));
    }
}
