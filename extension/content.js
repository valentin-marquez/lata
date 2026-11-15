// Adds lata's button to Project Zomboid pages of the Steam Workshop: "Install in lata" under
// Subscribe on a mod, and "Install collection in lata" under Subscribe to all on a
// collection. Each is a plain link (lata://install-mod?id=<id> or
// lata://install-collection?id=<id>); lata checks the item again and always asks before
// installing anything.
(() => {
  const PROJECT_ZOMBOID = "108600";
  const ID = "lata-install";

  // The item id is the page's `id` parameter, and nothing else is trusted to be one.
  const id = new URL(location.href).searchParams.get("id");
  if (!/^[0-9]{1,20}$/.test(id ?? "")) return;

  // The game comes from the breadcrumb's link, since the page's words change with the language
  // and another game's mod can mention Project Zomboid.
  const game = document.querySelector(`.breadcrumbs a[href*="/app/${PROJECT_ZOMBOID}"]`);
  if (!game || document.getElementById(ID)) return;

  // A mod has the page's own Subscribe button. A collection has a bar with Subscribe to all
  // instead (its items carry buttons of their own, with the item's id added to theirs).
  // Guides and art have neither, and get no button.
  const subscribe = document.getElementById("SubscribeItemBtn");
  const bar = document.querySelector(".subscribeCollection");
  const kind = subscribe ? "mod" : bar ? "collection" : null;
  if (!kind) return;

  // The button speaks the page's language, which is the one picked in Steam (`<html lang>`,
  // such as "es-419"), not the browser's: next to Steam's own button it should read the same.
  const TEXTS = {
    en: {
      mod: ["Install in lata", "Opens lata to install this mod"],
      collection: ["Install collection in lata", "Opens lata to install this collection's mods"],
    },
    es: {
      mod: ["Instalar en lata", "Abre lata para instalar este mod"],
      collection: [
        "Instalar colección en lata",
        "Abre lata para instalar los mods de esta colección",
      ],
    },
  };
  const language = document.documentElement.lang.toLowerCase().split("-")[0];
  const [words, tip] = (TEXTS[language] ?? TEXTS.en)[kind];

  const link = document.createElement("a");
  link.id = ID;
  link.className = kind === "mod" ? ID : `${ID} ${ID}-row`;
  link.href = `lata://install-${kind}?id=${id}`;
  link.title = tip;

  const label = document.createElement("span");
  label.textContent = words;

  // The can next to the words comes from content.css.
  link.append(label);
  if (kind === "mod") subscribe.insertAdjacentElement("afterend", link);
  else bar.insertAdjacentElement("afterend", link);
})();
