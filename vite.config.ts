import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const developmentHost = "127.0.0.1";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: developmentHost,
    port: 1420,
    strictPort: true,
    watch: {
      // Tauri watches native sources; frontend reloads do not need build data.
      ignored: [
        "**/.cache/**",
        "**/target/**",
        "**/src-tauri/**",
        "**/tools/pdf-benchmark/**",
        "**/artifacts/**",
      ],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2021",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
});
