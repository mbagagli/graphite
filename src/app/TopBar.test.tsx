import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";

import { browserWorkspace } from "../adapters/demo/browserDiagnostics";
import { ThemeContext } from "../design-system/theme";
import { TopBar } from "./TopBar";

vi.mock("../services/runtime", () => ({ isDesktopRuntime: true }));

it.each(["demo", "zotero"] as const)(
  "keeps %s source switching first and indexing recovery visible",
  (mode) => {
    const workspace = browserWorkspace;
    const offline = "Zotero is unavailable.";
    const interrupted = "Indexing was interrupted. Continue the next batch.";
    const noop = vi.fn();
    const markup = renderToStaticMarkup(
      <ThemeContext
        value={{
          state: { theme: "dark", source: "system" },
          actions: { setTheme: noop },
          meta: { storageKey: "test-theme" },
        }}
      >
        <TopBar
          preparation={{
            progress: null,
            busy: false,
            archive: null,
            archiveError: null,
            demoCached: false,
            start: async () => {},
            cancel: async () => {},
          }}
          openPanel="source"
          onPanelChange={noop}
          view="graph"
          selectedCount={0}
          selectionOpen={false}
          onSelectionOpen={noop}
          onViewChange={noop}
          onSearchOpen={noop}
          source={{
            ...workspace.source,
            mode,
            health: "offline-cache",
            lastError: offline,
          }}
          corpus={workspace.corpus}
          extraction={workspace.extraction}
          citations={workspace.citations}
          semantic={{
            ...workspace.semantic,
            modelInstalled: true,
            indexedPapers: 10,
            eligiblePapers: 10,
            chunkedDocuments: 2,
            eligibleDocuments: 8,
            lastError: interrupted,
          }}
          libraries={workspace.snapshot.libraries}
          syncing={false}
          citationIndexing={false}
          semanticIndexing={false}
          syncError={offline}
          onSync={noop}
          onRebuild={noop}
          onRebuildCorpus={noop}
          onRebuildCitations={noop}
          onInstallSemanticModel={noop}
          onCancelSemantic={noop}
          onRebuildSemantic={noop}
          onLibraryChange={noop}
          onModeChange={noop}
        />
      </ThemeContext>,
    );
    expect(markup).toContain('aria-label="Review selection (0 papers)"');
    expect(markup).toContain('popover="auto"');
    expect(markup).toContain('aria-label="Close library setup"');
    expect(markup).toContain("Switch to light mode");
    expect(markup).toContain(">Help</button>");
    expect(markup).toContain(">About</button>");
    expect(markup).not.toContain("Notifications are not available");
    expect(markup).not.toContain("lucide-bell");
    expect(markup).not.toContain("Workspace settings");
    expect(markup).not.toContain("Local researcher profile");
    expect(markup).toContain("lucide-unlink");
    expect(markup).not.toContain("lucide-wifi-off");
    expect(markup.split(offline)).toHaveLength(2);
    expect(markup).toContain(interrupted);
    expect(markup).not.toContain("Index next semantic batch");
    expect(markup).not.toContain("Check semantic index");
    const switchLabel = mode === "demo" ? "Use Zotero" : "Use demo data";
    expect(markup.indexOf(`>${switchLabel}</button>`)).toBeGreaterThan(0);
    expect(markup.indexOf(`>${switchLabel}</button>`)).toBeLessThan(
      markup.indexOf('class="library-preparation"'),
    );
    expect(markup.slice(markup.indexOf("<footer>"))).not.toContain(
      `>${switchLabel}</button>`,
    );
  },
);
