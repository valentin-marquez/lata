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
  "build": { "branch": "unstable", "id": 19876543 },
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
| `build` | Steam branch, and the build the pack was made on. |
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
3. The game installs. Steam only serves each branch's current build, so an older pack gets
   that one, and lata says so.
4. The Workshop items download at their manifest. When Steam no longer serves that version,
   the current one comes instead, and lata says so.

## What never goes in a pack

- Java and game arguments: one like `-javaagent` would run whatever the pack bundles as soon
  as someone presses Play.
- Logs, `console.txt`, `version.txt` and the game's `Workshop` cache: the game rebuilds them.
- The folders of Workshop mods, which travel as references.
- The secrets in any text file under `Server/`: `Password`, `RCONPassword` and
  `DiscordToken` go blank, with the rest of the settings.
- `mods/default.txt` and the game's `reset-mods-*.txt`: the active list is in the index.

`Saves/` and `options.ini` are offered unchecked: worlds can be gigabytes, and the options
are one player's own.
