// Adds an "Install in lata" button under Subscribe on the Steam Workshop page of a Project
// Zomboid mod. The button is a plain link to lata://install-mod?id=<id>; lata checks the item
// again and always asks before installing anything.
(() => {
  const PROJECT_ZOMBOID = "108600";
  const ID = "lata-install";

  // The item id is the page's `id` parameter, and nothing else is trusted to be one.
  const id = new URL(location.href).searchParams.get("id");
  if (!/^[0-9]{1,20}$/.test(id ?? "")) return;

  // Mods have the page's own Subscribe button. Guides, art and collections don't (a collection
  // only has the ones of its items, with the item's id added to this one).
  const subscribe = document.getElementById("SubscribeItemBtn");
  // The game comes from the breadcrumb's link, since the page's words change with the language
  // and another game's mod can mention Project Zomboid.
  const game = document.querySelector(`.breadcrumbs a[href*="/app/${PROJECT_ZOMBOID}"]`);
  if (!subscribe || !game || document.getElementById(ID)) return;

  const text = (key, fallback) =>
    (globalThis.chrome?.i18n?.getMessage(key) || fallback);

  const link = document.createElement("a");
  link.id = ID;
  link.className = ID;
  link.href = `lata://install-mod?id=${id}`;
  link.title = text("installTip", "Opens lata to install this mod");

  const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  icon.setAttribute("viewBox", "0 0 24 24");
  icon.setAttribute("fill", "none");
  icon.setAttribute("stroke", "currentColor");
  icon.setAttribute("stroke-width", "2");
  icon.setAttribute("aria-hidden", "true");
  const arrow = document.createElementNS("http://www.w3.org/2000/svg", "path");
  arrow.setAttribute("d", "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3");
  icon.append(arrow);

  const label = document.createElement("span");
  label.textContent = text("install", "Install in lata");

  link.append(icon, label);
  subscribe.insertAdjacentElement("afterend", link);
})();
