# lulo browser extension

Adds an **Install in lulo** button under Subscribe on the Steam Workshop page of a Project
Zomboid mod. The button is a plain link to `lulo://install-mod?id=<id>`: lulo checks the item
against Steam again and always asks which instance it goes into before installing anything.

It has no background script, asks for no permissions beyond running on the two Workshop page
addresses in `manifest.json`, and sends nothing anywhere.

The button shows only where the page has its own Subscribe button (so not on guides, art or
collections) and its breadcrumb links to Project Zomboid (app 108600).

## Try it

lulo has to be able to open `lulo://` links: on Windows, turn on *Open links from the browser* in
lulo's Settings, Behavior. The first click may make the browser ask to allow opening lulo.

- **Chrome, Edge, Brave**: `chrome://extensions`, turn on Developer mode, *Load unpacked*, pick
  this folder.
- **Firefox**: `about:debugging`, *This Firefox*, *Load Temporary Add-on*, pick `manifest.json`.

Then open any Project Zomboid mod on the Workshop.

## Package it

The extension is these files zipped with `manifest.json` at the top of the archive. This works the
same on every system:

```sh
cd extension
python -m zipfile -c ../lulo-extension.zip manifest.json content.js content.css _locales icons
```

## Not done yet

- A Firefox add-on id (`browser_specific_settings.gecko.id`) and the store listings, which need
  accounts and a privacy policy.
- A button on collection pages.
