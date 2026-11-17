import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import {
  isTheme,
  readStoredTheme,
  resolveTheme,
  ThemeContext,
  writeStoredTheme,
  type Theme,
  type ThemeContextValue,
  type ThemeState,
} from "./theme";

export function ThemeProvider({ children }: { children: ReactNode }) {
  const storageKey = getThemeStorageKey();
  const [state, setState] = useState<ThemeState>(() => {
    const storedTheme = readStoredTheme(window.localStorage, storageKey);
    const appliedTheme = document.documentElement.dataset.theme;
    const theme = isTheme(appliedTheme)
      ? appliedTheme
      : resolveTheme(storedTheme, prefersDarkTheme());
    applyTheme(theme);
    return {
      theme,
      source: storedTheme ? "stored" : "system",
    };
  });

  const setTheme = useCallback(
    (theme: Theme) => {
      applyTheme(theme);
      const persisted = writeStoredTheme(
        window.localStorage,
        storageKey,
        theme,
      );
      setState({ theme, source: persisted ? "stored" : "session" });
    },
    [storageKey],
  );

  useEffect(() => {
    if (state.source !== "system") {
      return;
    }

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleSystemThemeChange = (event: MediaQueryListEvent) => {
      const theme = event.matches ? "dark" : "light";
      applyTheme(theme);
      setState((current) =>
        current.source === "system" ? { ...current, theme } : current,
      );
    };
    mediaQuery.addEventListener("change", handleSystemThemeChange);
    return () =>
      mediaQuery.removeEventListener("change", handleSystemThemeChange);
  }, [state.source]);

  const value = useMemo<ThemeContextValue>(
    () => ({
      state,
      actions: { setTheme },
      meta: { storageKey },
    }),
    [setTheme, state, storageKey],
  );

  return <ThemeContext value={value}>{children}</ThemeContext>;
}

function getThemeStorageKey(): string {
  const key = document.documentElement.dataset.themeStorageKey;
  if (!key) {
    throw new Error("Graphite could not find the theme storage key.");
  }
  return key;
}

function prefersDarkTheme(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  const themeColor =
    theme === "dark"
      ? root.dataset.themeDarkColor
      : root.dataset.themeLightColor;
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
  if (themeColor) {
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute("content", themeColor);
  }
}
