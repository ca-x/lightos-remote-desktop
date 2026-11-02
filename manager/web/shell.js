(() => {
  const root = document.documentElement;
  const system = matchMedia("(prefers-color-scheme: dark)");
  const bridge = window.webkit?.messageHandlers;
  let choice;
  try { choice = localStorage.getItem("lazycat-remote.theme"); } catch {}
  function apply() {
    const host = root.dataset.lzcTheme;
    const theme = choice === "light" || choice === "dark" ? choice : host === "light" || host === "dark" ? host : system.matches ? "dark" : "light";
    root.dataset.theme = theme;
    root.style.colorScheme = theme;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", theme === "dark" ? "#1c1c1e" : "#f5f5f5");
    if (bridge?.SetCloseButtonColors?.postMessage) bridge.SetCloseButtonColors.postMessage({ params: [{ iconTintColor: theme === "dark" ? "#f2f2f2" : "#202020" }] });
  }
  apply();
  system.addEventListener("change", apply);
  new MutationObserver(apply).observe(root, { attributes: true, attributeFilter: ["data-lzc-theme"] });
  document.addEventListener("DOMContentLoaded", () => {
    const toggle = document.getElementById("theme-toggle");
    toggle.setAttribute("aria-label", navigator.language.toLowerCase().startsWith("zh") ? "切换明暗主题" : "Toggle color theme");
    toggle.addEventListener("click", () => {
      choice = root.dataset.theme === "dark" ? "light" : "dark";
      try { localStorage.setItem("lazycat-remote.theme", choice); } catch {}
      apply();
    });
    if (bridge?.SetCloseBtnShowStatus?.postMessage) {
      root.classList.add("native-close-visible");
      bridge.SetCloseBtnShowStatus.postMessage({ params: [true] });
    }
  });
})();
