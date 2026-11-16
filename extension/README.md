# lata link

The browser extension of lata.

Adds an **Install in lata** button under Subscribe on the Steam Workshop page of a Project
Zomboid mod, and **Install collection in lata** under Subscribe to all on a collection. Each
is a plain link (`lata://install-mod?id=<id>`, `lata://install-collection?id=<id>`): lata checks
the item against Steam again and always asks which instance it goes into before installing
anything. The button reads in the language the Steam page is in (English or Spanish).

It has no background script, asks for no permissions beyond running on the two Workshop page
addresses in `manifest.json`, and sends nothing anywhere.

The button shows only where the page has its own Subscribe button or a collection's Subscribe
to all (so not on guides or art) and its breadcrumb links to Project Zomboid (app 108600).

## Try it

lata has to be able to open `lata://` links: on Windows, turn on *Open links from the browser* in
lata's Settings, Behavior. The first click may make the browser ask to allow opening lata.

- **Chrome, Edge, Brave**: `chrome://extensions`, turn on Developer mode, *Load unpacked*, pick
  this folder.
- **Firefox**: `about:debugging`, *This Firefox*, *Load Temporary Add-on*, pick `manifest.json`.

Then open any Project Zomboid mod on the Workshop.

## Package it

The `Extension` workflow checks it with Mozilla's `web-ext lint` and builds the zip on every
change under `extension/`; the zip is the run's `lata-link` artifact. Pushing a tag named
`extension-v<version>`, with the version `manifest.json` has, also attaches it to a draft release.

By hand it is these files zipped with `manifest.json` at the top of the archive:

```sh
cd extension
python -m zipfile -c ../lata-link.zip manifest.json content.js content.css _locales icons
```

## Publish it

lata link keeps its existing store listings, so the people who already have it get each
version as an update. The texts to paste are in `STORE.md`.

- **Firefox**: `addons.mozilla.org/firefox/addon/lata-link`. The add-on id in `manifest.json`
  (`browser_specific_settings.gecko.id`) is that listing's; with another id the upload would be
  a new add-on.
- **Chrome**: item `fopkfokkcmmhgjjdlbjjmdgkffddjbpm`. The id comes from the account that
  uploads, not from the manifest.

Both stores only take a version higher than the one they have. The old extension was 1.0.0,
which is why lata link starts at 2.0.0.

## Not done yet

- Pictures for the listings.
