//! Making the system open `lulo://` links with lulo. On Windows lulo registers itself in the
//! current user's registry, since a zip has no installer to do it; that only happens when the
//! user turns it on in Settings. On macOS the app bundle declares the protocol (`Info.plist`)
//! and on Linux the `.desktop` file does (`MimeType`), so there is nothing to do at run time.

/// Whether lulo can register the protocol itself here, which is what Settings offers.
pub const SUPPORTED: bool = cfg!(windows);

/// Whether the system opens `lulo://` links with this very lulo.
#[cfg(windows)]
pub fn enabled() -> bool {
    std::env::current_exe().is_ok_and(|exe| {
        imp::registered_command(imp::ROOT).is_some_and(|cmd| cmd == imp::command(&exe))
    })
}

/// Registers this lulo for `lulo://` links, or removes the registration.
#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
    if on {
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        imp::register(imp::ROOT, &exe)
    } else {
        imp::unregister(imp::ROOT)
    }
}

#[cfg(not(windows))]
pub fn enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set(_: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::Command;

    /// Where Windows looks for a protocol's handler, under the current user.
    pub const ROOT: &str = r"Software\Classes\lulo";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The command Windows runs for a link: the executable and the link as one argument.
    pub fn command(exe: &Path) -> String {
        format!("\"{}\" \"%1\"", exe.display())
    }

    fn reg(args: &[&str]) -> Result<String, String> {
        let out = Command::new("reg")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|err| err.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }

    /// Writes the three values that make `root` a protocol handled by `exe`.
    pub fn register(root: &str, exe: &Path) -> Result<(), String> {
        let key = format!(r"HKCU\{root}");
        let open = format!(r"{key}\shell\open\command");
        reg(&["add", &key, "/ve", "/d", "URL:lulo", "/f"])?;
        reg(&["add", &key, "/v", "URL Protocol", "/d", "", "/f"])?;
        reg(&["add", &open, "/ve", "/d", &command(exe), "/f"])?;
        Ok(())
    }

    /// Removes the registration; one that isn't there is already as asked.
    pub fn unregister(root: &str) -> Result<(), String> {
        let key = format!(r"HKCU\{root}");
        if reg(&["query", &key]).is_err() {
            return Ok(());
        }
        reg(&["delete", &key, "/f"]).map(drop)
    }

    /// The command registered under `root`, if any. `reg query` words its labels in the
    /// system's language, so the value is found by its type, `REG_SZ`.
    pub fn registered_command(root: &str) -> Option<String> {
        let key = format!(r"HKCU\{root}\shell\open\command");
        let out = reg(&["query", &key, "/ve"]).ok()?;
        let value = out.lines().find_map(|line| line.split_once("REG_SZ"))?.1;
        Some(value.trim().to_owned())
    }

    /// The `URL Protocol` marker, which is what makes Windows treat the key as a protocol.
    #[cfg(test)]
    pub fn is_protocol(root: &str) -> bool {
        reg(&["query", &format!(r"HKCU\{root}"), "/v", "URL Protocol"]).is_ok()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;
    use std::path::Path;

    /// The same three registry writes as the real thing, under a key of its own that isn't a
    /// protocol anyone uses, removed at the end.
    #[test]
    fn registers_and_removes_a_handler() {
        let root = format!(r"Software\LuloTest-{}", std::process::id());
        let exe = Path::new(r"C:\Program Files\Lulo App\lulo.exe");
        let _ = unregister(&root);
        assert_eq!(registered_command(&root), None);
        assert!(!is_protocol(&root));

        register(&root, exe).unwrap();
        // Spaces in the path stay inside quotes, and the link is one argument.
        let expected = r#""C:\Program Files\Lulo App\lulo.exe" "%1""#;
        assert_eq!(command(exe), expected);
        assert_eq!(registered_command(&root).as_deref(), Some(expected));
        assert!(is_protocol(&root));
        // Registering again just updates it.
        register(&root, Path::new(r"D:\other\lulo.exe")).unwrap();
        assert_eq!(
            registered_command(&root).as_deref(),
            Some(r#""D:\other\lulo.exe" "%1""#)
        );

        unregister(&root).unwrap();
        assert_eq!(registered_command(&root), None);
        assert!(!is_protocol(&root));
        // Removing what isn't there is fine.
        unregister(&root).unwrap();
    }
}
