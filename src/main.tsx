import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter/wght.css";

import { App } from "./app/App";
import "./design-system/tokens.css";
import "./design-system/global.css";
import "./features/library/library.css";
import "./features/search/search.css";
import "./features/reader/reader.css";
import "./features/agent/agent.css";
import { ThemeProvider } from "./design-system/ThemeProvider";

const root = document.getElementById("root");

if (!root) {
  throw new Error("Graphite could not find the application root element.");
}

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      <App />
    </ThemeProvider>
  </StrictMode>,
);
