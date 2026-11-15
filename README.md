![lata](assets/banner.jpg)

[![Discord](https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white)](https://discord.gg/jeGPaMkDbU)
[![CI](https://img.shields.io/github/actions/workflow/status/valentin-marquez/lata/ci.yml?label=CI)](https://github.com/valentin-marquez/lata/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

Instance launcher for Project Zomboid, rewritten from scratch in Rust.

> Work in progress. Not usable yet.

Each instance is its own install: game version (Steam branch), mods and saves. lata talks to Steam
natively, no SteamCMD.

![Library](assets/screenshots/library-dark.png)

## What it does

- Several installs side by side, each on its own Steam branch.
- Mods per instance: add, turn on or off, remove.
- Workshop mods and whole collections installed from a link, with the
  [lata link](extension) browser extension.
- Worlds and live game logs for each instance.
- Light and dark themes.

| | |
|---|---|
| ![Mods of an instance](assets/screenshots/mods-dark.png) | ![Installing a collection](assets/screenshots/collection-dark.png) |
| ![Installing a mod, light theme](assets/screenshots/mod-light.png) | |

## lata link

The browser extension adds an "Install in lata" button to Workshop pages.

[![Chrome Web Store](https://img.shields.io/chrome-web-store/v/fopkfokkcmmhgjjdlbjjmdgkffddjbpm?label=Chrome&logo=googlechrome&logoColor=white)](https://chromewebstore.google.com/detail/fopkfokkcmmhgjjdlbjjmdgkffddjbpm)
[![Firefox Add-ons](https://img.shields.io/amo/v/lata-link?label=Firefox&logo=firefoxbrowser&logoColor=white)](https://addons.mozilla.org/firefox/addon/lata-link/)

## Community

Questions, feedback and news are on [Discord](https://discord.gg/jeGPaMkDbU).

## Build

```sh
cargo run -p lata
```

## Credits

Icons: [Solar](https://www.figma.com/community/file/1166831539721848736) by 480 Design,
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

Banner background: a screenshot of [Project Zomboid](https://projectzomboid.com) by The Indie Stone,
from its Steam store page. lata is a fan project, not affiliated with The Indie Stone.

## License

The code is under the [GNU General Public License v3](LICENSE). The lata name and logo, and the
Project Zomboid material, are not: see [COPYING.md](COPYING.md).
