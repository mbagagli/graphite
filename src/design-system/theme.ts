import { createContext, use } from "react";

export type Theme = "light" | "dark";
export type ThemeSource = "system" | "stored" | "session";

export interface ThemeState {
  theme: Theme;
  source: ThemeSource;
}

export interface ThemeActions {
  setTheme: (theme: Theme) => void;
}

export interface ThemeMeta {
  storageKey: string;
}

export interface ThemeContextValue {
  state: ThemeState;
  actions: ThemeActions;
  meta: ThemeMeta;
}

interface StorageReader {
  getItem: (key: string) => string | null;
}

interface StorageWriter {
  setItem: (key: string, value: string) => void;
}

export const ThemeContext = createContext<ThemeContextValue | null>(null);

export function isTheme(value: string | null | undefined): value is Theme {
  return value === "light" || value === "dark";
}

export function resolveTheme(
  storedTheme: string | null,
  prefersDark: boolean,
): Theme {
  if (isTheme(storedTheme)) {
    return storedTheme;
  }
  return prefersDark ? "dark" : "light";
}

export function readStoredTheme(
  storage: StorageReader,
  storageKey: string,
): Theme | null {
  try {
    const value = storage.getItem(storageKey);
    return isTheme(value) ? value : null;
  } catch {
    return null;
  }
}

export function writeStoredTheme(
  storage: StorageWriter,
  storageKey: string,
  theme: Theme,
): boolean {
  try {
    storage.setItem(storageKey, theme);
    return true;
  } catch {
    return false;
  }
}

export function useTheme(): ThemeContextValue {
  const value = use(ThemeContext);
  if (!value) {
    throw new Error("useTheme must be used within ThemeProvider.");
  }
  return value;
}
