# The `.lata` pack format

A `.lata` file is an instance in a can: what someone needs to play the same game with the
same mods and settings. It is a zip shaped like Modrinth's `.mrpack` (an index plus an
`overrides/` folder), with its own index, since `.mrpack` requires `game: "minecraft"` and
download URLs on Modrinth's domains.

## Layout

```
pack.lata               (zip)
├── lata.index.json
├── icon.png            only when the instance uses an uploaded picture
└── overrides/          extracted into the new instance's zomboid/ folder
    ├── Server/         servertest.ini, *_SandboxVars.lua
    ├── Lua/            mod options
    ├── Sandbox Presets/
    └── mods/<folder>/  mods that are not on the Workshop
```

## `lata.index.json`

```json
{
  "format": 1,
  "game": "projectzomboid",
  "name": "Server with friends",
  "version": "1.0.0",
  "summary": "",
  "build": {
    "branch": "unstable",
    "id": 19876543,
    "depots": [
      { "id": 108603, "manifest": 1234567890123456789 },
      { "id": 108604, "manifest": 2345678901234567890 }
    ]
  },
  "workshop": [
    { "id": 2392709985, "manifest": 4123456789012, "title": "Brita's Weapon Pack", "folders": ["Brita"] }
  ],
  "mods": ["Brita", "Local"],
  "maps": ["Muldraugh, KY"],
  "settings": { "memory_gb": 8 },
  "icon": { "kind": "item", "item": "Item_Axe", "color": 14234211 }
}
```

| Field | Meaning |
|---|---|
| `format` | Version of this format. A lata refuses a pack with a higher number than it knows. |
| `game` | Always `projectzomboid`. |
| `name`, `version`, `summary` | As typed when exporting. |
| `build` | Steam branch, the build the pack was made on, and that build's manifest for every OS depot (`depots`). With `depots` the pack keeps the game at that build; without them (the exporter unchecked "Keep the game at", or lata didn't know the manifests) it follows the branch. |
| `workshop` | Workshop items with the manifest each was downloaded at: an exact version, like the hashes and CDN URLs of `.mrpack`. `folders` are the folders the item puts under `mods/`. |
| `mods`, `maps` | The active mod ids and maps, in load order. |
| `settings` | Memory for the game, in GB, or `null` for the game's default. |
| `icon` | An item icon (`kind: "item"`), or an uploaded picture (`kind: "image"`) stored at the top of the zip as `icon.<ext>`. |

Unknown fields are ignored, so a newer lata can add fields without bumping `format`.

## Importing

1. Every path in the zip is checked before anything is written: no absolute paths, no `..`
   that climbs out, and only `overrides/` and the icon are extracted.
2. A new instance is made on `build.branch` with the pack's name, memory and icon, the
   overrides go into its `zomboid/`, and `mods`/`maps` become its active list.
3. The game installs. With `depots`, the first install asks Steam for that build's manifests
   (on the pack's branch, then on `public`, where Steam still serves builds that have left it).
   When Steam refuses (a build that only ever lived on a beta branch), the branch's current
   build comes instead, and lata says so. Without `depots`, the branch's current build comes,
   and lata says so when the pack was made on an older one.
4. The Workshop items download at their manifest. When Steam no longer serves that version,
   the current one comes instead, and lata says so.

## What never goes in a pack

- Java and game arguments: one like `-javaagent` would run whatever the pack bundles as soon
  as someone presses Play.
- Whether Java mods are turned on. A pack can bring Java mods and their loader (ZombieBuddy)
  like any other Workshop item, but the instance it makes starts with them off: whoever
  imports it is asked, on their own computer, before any of that code runs.
- Logs, `console.txt`, `version.txt` and the game's `Workshop` cache: the game rebuilds them.
- The folders of Workshop mods, which travel as references.
- The secrets in any file under `Server/`, whatever its encoding: `Password`, `RCONPassword` and
  `DiscordToken` go blank, with the rest of the settings.
- `mods/default.txt` and the game's `reset-mods-*.txt`: the active list is in the index.

`Saves/` and `options.ini` are offered unchecked: worlds can be gigabytes, and the options
are one player's own.
