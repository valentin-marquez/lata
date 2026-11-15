//! Which browser opens links on this computer, to send people to the store that has lata link
//! for it: Firefox and its forks (Zen, LibreWolf, Floorp, Waterfox) install it from Mozilla's
//! add-ons site, Chrome and the browsers built on Chromium (Brave, Edge, Opera, Vivaldi) from
//! the Chrome Web Store.

pub const FIREFOX: &str = "https://addons.mozilla.org/firefox/addon/lata-link/";
pub const CHROME: &str =
    "https://chromewebstore.google.com/detail/fopkfokkcmmhgjjdlbjjmdgkffddjbpm";

/// Names that only browsers made from Firefox carry in their desktop file, Windows ProgId or
/// macOS bundle id.
const MOZILLA: [&str; 7] = [
    "firefox",
    "zen",
    "librewolf",
    "floorp",
    "waterfox",
    "mozilla",
    "mullvad",
];

/// lata link's page in the store for the default browser. The Chrome Web Store when the
/// browser can't be told: most browsers are built on Chromium.
// ponytail: Safari and GNOME Web have no lata link; they get the Chrome page too.
pub fn store() -> &'static str {
    store_for(&default().unwrap_or_default())
}

fn store_for(browser: &str) -> &'static str {
    let browser = browser.to_ascii_lowercase();
    if MOZILLA.iter().any(|name| browser.contains(name)) {
        FIREFOX
    } else {
        CHROME
    }
}

/// The default browser as the system names it: `zen.desktop` on Linux, a ProgId like
/// `FirefoxURL-308046B0AF4A39CB` on Windows, a bundle id like `com.brave.Browser` on macOS.
#[cfg(all(unix, not(target_os = "macos")))]
fn default() -> Option<String> {
    let out = std::process::Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|name| !name.is_empty());
    // Without xdg-utils, the file it reads.
    out.or_else(|| {
        let file = dirs::config_dir()?.join("mimeapps.list");
        handler(&std::fs::read_to_string(file).ok()?)
    })
}

/// The handler of `https` links in a `mimeapps.list`.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn handler(text: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("x-scheme-handler/https="))?;
    let first = value.split(';').next()?.trim();
    (!first.is_empty()).then(|| first.to_owned())
}

#[cfg(windows)]
fn default() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v",
            "ProgId",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    prog_id(&String::from_utf8_lossy(&out.stdout))
}

/// The value of `ProgId` in the output of `reg query`.
#[cfg(any(windows, test))]
fn prog_id(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("ProgId"))?;
    let value = line.split_once("REG_SZ")?.1.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(target_os = "macos")]
fn default() -> Option<String> {
    let plist = dirs::home_dir()?
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let out = std::process::Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    bundle_id(&String::from_utf8_lossy(&out.stdout))
}

/// The app that handles `https` in LaunchServices' list, converted to JSON.
#[cfg(any(target_os = "macos", test))]
fn bundle_id(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value["LSHandlers"]
        .as_array()?
        .iter()
        .find(|handler| handler["LSHandlerURLScheme"] == "https")?["LSHandlerRoleAll"]
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_each_browser_to_its_store() {
        // Linux desktop files (Flatpak ones included), Windows ProgIds, macOS bundle ids.
        for firefox in [
            "zen.desktop",
            "app.zen_browser.zen.desktop",
            "firefox.desktop",
            "org.mozilla.firefox.desktop",
            "librewolf.desktop",
            "FirefoxURL-308046B0AF4A39CB",
            "app.zen-browser.zen",
            "org.mozilla.firefox",
        ] {
            assert_eq!(store_for(firefox), FIREFOX, "{firefox}");
        }
        for chrome in [
            "google-chrome.desktop",
            "brave-browser.desktop",
            "com.brave.Browser.desktop",
            "microsoft-edge.desktop",
            "vivaldi-stable.desktop",
            "ChromeHTML",
            "BraveHTML",
            "MSEdgeHTM",
            "com.google.Chrome",
            "",
        ] {
            assert_eq!(store_for(chrome), CHROME, "{chrome}");
        }
    }

    #[test]
    fn reads_the_default_browser_where_each_system_keeps_it() {
        let mimeapps = "[Default Applications]\nx-scheme-handler/http=zen.desktop;\nx-scheme-handler/https=zen.desktop;\n";
        assert_eq!(handler(mimeapps).as_deref(), Some("zen.desktop"));
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice\r\n    ProgId    REG_SZ    BraveHTML\r\n\r\n";
        assert_eq!(prog_id(reg).as_deref(), Some("BraveHTML"));
        let plist = r#"{"LSHandlers":[{"LSHandlerURLScheme":"mailto","LSHandlerRoleAll":"com.apple.mail"},{"LSHandlerURLScheme":"https","LSHandlerRoleAll":"app.zen-browser.zen"}]}"#;
        assert_eq!(bundle_id(plist).as_deref(), Some("app.zen-browser.zen"));
    }
}
