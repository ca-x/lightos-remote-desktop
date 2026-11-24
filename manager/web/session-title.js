// Older viewers may assign a generic title after the manager serves the page.
(() => {
  const name = document.querySelector('meta[name="lightos-desktop-name"]')?.content;
  const title = document.querySelector("title");
  if (!name || !title) return;
  const apply = () => { if (document.title !== name) document.title = name; };
  apply();
  new MutationObserver(apply).observe(title, { childList: true, characterData: true, subtree: true });
})();
