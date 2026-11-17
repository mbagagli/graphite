import { browserDemoServices } from "../adapters/demo/BrowserDemoServices";
import { createTauriServices } from "../adapters/tauri/TauriServices";
import { readSourceMode, resolveRuntimeSourceMode } from "./storage";

export const isDesktopRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const requestedMode =
  typeof window === "undefined" ? "demo" : readSourceMode(window.localStorage);

export const sourceMode = resolveRuntimeSourceMode(
  isDesktopRuntime,
  import.meta.env.DEV,
  requestedMode,
);

export const services = isDesktopRuntime
  ? createTauriServices(sourceMode)
  : browserDemoServices;
