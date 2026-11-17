import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { SourceDiagnostics } from "../services/contracts";
import { SourceConnectionNotice } from "./SourceConnectionNotice";

describe("Zotero connection notice", () => {
  it("stays hidden for demo, ready, and actively syncing sources", () => {
    const onRetry = vi.fn();
    const onUseDemo = vi.fn();
    const render = (source: SourceDiagnostics, syncing = false) =>
      renderToStaticMarkup(
        <SourceConnectionNotice
          source={source}
          syncing={syncing}
          syncError={source.lastError}
          onRetry={onRetry}
          onUseDemo={onUseDemo}
        />,
      );

    expect(render(source({ mode: "demo", health: "demo" }))).toBe("");
    expect(render(source({ health: "ready", lastError: null }))).toBe("");
    expect(render(source(), true)).toBe("");
  });

  it("presents an unavailable fresh source as a polite recoverable status", () => {
    const markup = renderToStaticMarkup(
      <SourceConnectionNotice
        source={source()}
        syncing={false}
        syncError="Zotero is not reachable at 127.0.0.1:23119."
        onRetry={vi.fn()}
        onUseDemo={vi.fn()}
      />,
    );

    expect(markup).toContain("lucide-unlink");
    expect(markup).not.toContain("lucide-wifi-off");
    expect(markup).toContain('role="status"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain("Zotero is not available");
    expect(markup).toContain("Retry Zotero");
    expect(markup).toContain("Use demo data");
    expect(markup).toContain("Dismiss Zotero connection notice");
    expect(markup).toContain("has not changed your Zotero library");
  });

  it("gives the exact recovery for disabled local API access", () => {
    const markup = renderToStaticMarkup(
      <SourceConnectionNotice
        source={source({
          lastError: "Zotero local API access is disabled.",
        })}
        syncing={false}
        syncError="Zotero local API access is disabled."
        onRetry={vi.fn()}
        onUseDemo={vi.fn()}
      />,
    );

    expect(markup).toContain("Zotero local access is disabled");
    expect(markup).toContain(
      "Allow other applications on this computer to communicate with Zotero",
    );
  });

  it("keeps cached-library failures explicitly non-blocking", () => {
    const markup = renderToStaticMarkup(
      <SourceConnectionNotice
        source={source({
          health: "offline-cache",
          lastError: "The local Zotero request failed.",
        })}
        syncing={false}
        syncError="The local Zotero request failed."
        onRetry={vi.fn()}
        onUseDemo={vi.fn()}
      />,
    );

    expect(markup).toContain("Zotero could not be synchronized");
    expect(markup).toContain("cached library remains available");
  });
});

function source(overrides: Partial<SourceDiagnostics> = {}): SourceDiagnostics {
  return {
    mode: "zotero",
    health: "error",
    connectionStatus: "offline",
    serverId: null,
    apiVersion: null,
    schemaVersion: null,
    lastSyncedAt: null,
    lastError: "Zotero is not reachable at 127.0.0.1:23119.",
    activeLibraryId: null,
    sourceChanged: false,
    syncSummary: null,
    ...overrides,
  };
}
