//! One lulo at a time. The running lulo listens on a loopback port and keeps the port and a
//! secret in `lulo.lock`; a second one reads that file, hands over what it was started with (the
//! links, or "show yourself") and leaves, instead of opening another window.
//!
//! Any program on the computer can reach a loopback port, a web page in a browser too, so the
//! first thing a connection sends must be the secret from the lock file, which a page can't
//! read. Everything read is bounded in size, count and time. Even then a link only gets to the
//! install dialog, which always asks.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

use super::Incoming;

const HELLO: &str = "lulo-1";
const CONNECT: Duration = Duration::from_millis(500);
const TIMEOUT: Duration = Duration::from_secs(2);
/// What one connection may send, and how many messages of it count.
const MAX_BYTES: u64 = 8 * 1024;
const MAX_MESSAGES: usize = 20;

/// The messages a second lulo sends: its links, or a request to be shown if it has none.
pub fn messages(links: &[String]) -> Vec<String> {
    if links.is_empty() {
        vec!["show".to_owned()]
    } else {
        links.iter().map(|link| format!("link {link}")).collect()
    }
}

/// Hands `messages` to the lulo that is running, and says whether it took them. `false` when
/// there is none: no lock file, or one left by a lulo that has since closed.
pub fn forward(lock: &Path, messages: &[String]) -> bool {
    let Some((port, token)) = read_lock(lock) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let mut payload = format!("{HELLO} {token}\n");
    for message in messages {
        // One message per line: a line break inside one would be taken for two.
        if !message.chars().any(char::is_control) {
            payload.push_str(message);
            payload.push('\n');
        }
    }
    if stream.write_all(payload.as_bytes()).is_err() || stream.shutdown(Shutdown::Write).is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

/// Becomes the running lulo: listens on a loopback port, writes the lock file and passes what
/// arrives to `deliver`, on a thread of its own.
pub fn serve(lock: &Path, deliver: impl Fn(Incoming) + Send + 'static) -> io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let token = token();
    let port = listener.local_addr()?.port();
    crate::account::write_private(lock, &format!("{port}\n{token}\n"))?;
    std::thread::spawn(move || {
        for connection in listener.incoming().flatten() {
            read(connection, &token, &deliver);
        }
    });
    Ok(())
}

fn read_lock(lock: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(lock).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse().ok()?;
    let token = lines.next()?.trim().to_owned();
    (!token.is_empty()).then_some((port, token))
}

/// One connection: the secret first, then one message per line until it closes.
fn read(stream: TcpStream, token: &str, deliver: &impl Fn(Incoming)) {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let Ok(mut reply) = stream.try_clone() else {
        return;
    };
    let mut lines = BufReader::new(stream.take(MAX_BYTES)).lines();
    if lines.next().and_then(Result::ok).as_deref() != Some(&format!("{HELLO} {token}")) {
        return;
    }
    for line in lines.map_while(Result::ok).take(MAX_MESSAGES) {
        if let Some(link) = line.strip_prefix("link ") {
            deliver(Incoming::Link(link.to_owned()));
        } else if line == "show" {
            deliver(Incoming::Show);
        }
    }
    let _ = reply.write_all(b"ok\n");
}

/// A secret for this run. The standard library seeds its hashers from the system's random
/// source, so hashing with two of them gives 128 bits nobody can guess.
// shortcut: not a cryptographic generator; fine against a web page or another user's program
// that can't read the lock file, and not meant to hold against a hostile program running as
// this user, which could read the lock file anyway.
fn token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let part = || RandomState::new().hash_one(std::process::id());
    format!("{:016x}{:016x}", part(), part())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lulo-single-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn running(lock: &Path) -> mpsc::Receiver<Incoming> {
        let (send, got) = mpsc::channel();
        let send = std::sync::Mutex::new(send);
        serve(lock, move |message| {
            let _ = send.lock().unwrap().send(message);
        })
        .unwrap();
        got
    }

    fn next(got: &mpsc::Receiver<Incoming>) -> Option<Incoming> {
        got.recv_timeout(Duration::from_secs(3)).ok()
    }

    #[test]
    fn the_second_lulo_hands_its_links_to_the_first() {
        let dir = temp("hand");
        let lock = dir.join("lulo.lock");
        assert!(!forward(&lock, &messages(&[])), "nothing is running yet");

        let got = running(&lock);
        let links = [
            "lulo://install-mod?id=1".to_owned(),
            "lulo://install-mod?id=2".to_owned(),
        ];
        assert!(forward(&lock, &messages(&links)));
        assert_eq!(next(&got), Some(Incoming::Link(links[0].clone())));
        assert_eq!(next(&got), Some(Incoming::Link(links[1].clone())));
        // Started without a link, a second lulo only asks the first to show itself.
        assert!(forward(&lock, &messages(&[])));
        assert_eq!(next(&got), Some(Incoming::Show));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_left_by_a_lulo_that_closed_forwards_nothing() {
        let dir = temp("stale");
        let lock = dir.join("lulo.lock");
        // A port that was free a moment ago and has nobody listening.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(&lock, format!("{port}\nsecret\n")).unwrap();
        assert!(!forward(&lock, &messages(&[])));
        for broken in ["", "nonsense", "70000\nsecret\n", "80\n\n"] {
            std::fs::write(&lock, broken).unwrap();
            assert!(!forward(&lock, &messages(&[])), "{broken:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_does_not_know_the_secret_gets_nothing() {
        let dir = temp("secret");
        let lock = dir.join("lulo.lock");
        let got = running(&lock);
        let (port, token) = read_lock(&lock).unwrap();
        assert_eq!(token.len(), 32);

        let send = |first: &str| {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let _ = stream.set_read_timeout(Some(TIMEOUT));
            let _ = write!(stream, "{first}\nlink lulo://install-mod?id=9\nshow\n");
            let _ = stream.shutdown(Shutdown::Write);
            let mut answer = String::new();
            let _ = BufReader::new(stream).read_line(&mut answer);
            answer
        };
        // A web page's request, a wrong secret, no secret, and the right word with a wrong one.
        for first in [
            "POST / HTTP/1.1",
            "lulo-1 wrong",
            "link lulo://install-mod?id=9",
            "lulo-2 x",
        ] {
            assert_eq!(send(first), "", "{first}");
        }
        assert_eq!(got.recv_timeout(Duration::from_millis(300)).ok(), None);
        // With the secret it works.
        assert_eq!(send(&format!("{HELLO} {token}")), "ok\n");
        assert_eq!(
            next(&got),
            Some(Incoming::Link("lulo://install-mod?id=9".into()))
        );
        assert_eq!(next(&got), Some(Incoming::Show));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_connection_may_send_is_bounded() {
        let dir = temp("bounds");
        let lock = dir.join("lulo.lock");
        let got = running(&lock);
        let (port, token) = read_lock(&lock).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let _ = stream.set_read_timeout(Some(TIMEOUT));
        // Far more messages than are taken, and then a flood of bytes.
        let mut payload = format!("{HELLO} {token}\n");
        for _ in 0..(MAX_MESSAGES * 3) {
            payload.push_str("show\n");
        }
        payload.push_str(&"x".repeat(100_000));
        let _ = stream.write_all(payload.as_bytes());
        let _ = stream.shutdown(Shutdown::Write);
        let mut answer = String::new();
        let _ = BufReader::new(stream).read_line(&mut answer);
        assert_eq!(answer, "ok\n");
        let mut shown = 0;
        while got.recv_timeout(Duration::from_millis(300)).is_ok() {
            shown += 1;
        }
        assert_eq!(shown, MAX_MESSAGES);
        // A line break inside a message can't smuggle in a second one.
        assert!(forward(
            &lock,
            &["link a\nshow".to_owned(), "show".to_owned()]
        ));
        assert_eq!(next(&got), Some(Incoming::Show));
        assert_eq!(got.recv_timeout(Duration::from_millis(300)).ok(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_secret_changes_every_time() {
        assert_ne!(token(), token());
    }
}
