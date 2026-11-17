/* global document, window */

(() => {
  const root = document.documentElement;
  const storageKey = root.dataset.themeStorageKey;
  let storedTheme;

  try {
    storedTheme = storageKey ? window.localStorage.getItem(storageKey) : null;
  } catch {
    storedTheme = null;
  }

  const theme =
    storedTheme === "light" || storedTheme === "dark"
      ? storedTheme
      : window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";

  root.dataset.theme = theme;
  root.style.colorScheme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute(
      "content",
      theme === "dark"
        ? root.dataset.themeDarkColor
        : root.dataset.themeLightColor,
    );
})();
