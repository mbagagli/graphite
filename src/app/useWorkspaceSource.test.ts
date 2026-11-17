import { describe, expect, it, vi } from "vitest";

import { browserWorkspace } from "../adapters/demo/browserDiagnostics";
import type { WorkspacePayload, WorkspaceService } from "../services/contracts";
import {
  loadInitialWorkspace,
  runWorkspaceSourceOperation,
  selectWorkspaceSourceMode,
  switchWorkspaceLibrary,
  type WorkspaceSourceLifecycle,
} from "./useWorkspaceSource";

describe("workspace source lifecycle", () => {
  it("loads the initial demo workspace without synchronizing", async () => {
    const service = createService({
      load: vi.fn().mockResolvedValue(payload("initial")),
    });
    const observed = createLifecycle();

    await loadInitialWorkspace(service, observed.lifecycle, () => true);

    expect(service.load).toHaveBeenCalledWith();
    expect(service.sync).not.toHaveBeenCalled();
    expect(observed.events).toEqual(["initial:initial"]);
    expect(observed.data?.source.connectionStatus).toBe("initial");
  });

  it("auto-synchronizes Zotero and advances pending citations in order", async () => {
    const initial = payload("initial", "zotero");
    const synced = payload("synced", "zotero", 2);
    const advanced = payload("advanced", "zotero");
    const service = createService({
      load: vi.fn().mockResolvedValue(initial),
      sync: vi.fn().mockResolvedValue(synced),
    });
    const observed = createLifecycle({
      advanceAfterSync: vi
        .fn()
        .mockImplementation(async (_payload, _active, commitData) => {
          commitData(advanced);
        }),
    });

    await loadInitialWorkspace(service, observed.lifecycle, () => true);

    expect(observed.lifecycle.advanceAfterSync).toHaveBeenCalledWith(
      synced,
      expect.any(Function),
      expect.any(Function),
    );
    expect(observed.events).toEqual([
      "initial:initial",
      "syncing:true",
      "initial:synced",
      "sync-error:null",
      "initial:advanced",
      "syncing:false",
    ]);
  });

  it("uses a forced cached load after initial Zotero sync failure", async () => {
    const initial = payload("initial", "zotero");
    const cached = payload("cached", "zotero");
    const load = vi
      .fn()
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(cached);
    const service = createService({
      load,
      sync: vi.fn().mockRejectedValue(new Error("Zotero unavailable")),
    });
    const observed = createLifecycle();

    await loadInitialWorkspace(service, observed.lifecycle, () => true);

    expect(load).toHaveBeenNthCalledWith(1);
    expect(load).toHaveBeenNthCalledWith(2, true);
    expect(observed.events).toEqual([
      "initial:initial",
      "syncing:true",
      "sync-error:Zotero unavailable",
      "initial:offline",
      "syncing:false",
    ]);
    expect(observed.data?.source).toMatchObject({
      health: "offline-cache",
      connectionStatus: "offline",
      lastError: "Zotero unavailable",
    });
  });

  it("keeps a fresh empty workspace usable when Zotero is unavailable", async () => {
    const initial = emptyPayload("initial");
    const cached = emptyPayload("cached");
    const load = vi
      .fn()
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(cached);
    const service = createService({
      load,
      sync: vi
        .fn()
        .mockRejectedValue(
          new Error(
            "Zotero is not reachable at 127.0.0.1:23119. Start Zotero and enable its local API.",
          ),
        ),
    });
    const observed = createLifecycle();

    await loadInitialWorkspace(service, observed.lifecycle, () => true);

    expect(observed.events).toEqual([
      "initial:initial",
      "syncing:true",
      "sync-error:Zotero is not reachable at 127.0.0.1:23119. Start Zotero and enable its local API.",
      "initial:offline",
      "syncing:false",
    ]);
    expect(observed.events.some((event) => event.startsWith("fatal:"))).toBe(
      false,
    );
    expect(observed.data?.snapshot.items).toEqual([]);
    expect(observed.data?.source).toMatchObject({
      health: "error",
      connectionStatus: "offline",
    });
  });

  it("reports fatal initial and cached-fallback failures only while mounted", async () => {
    const initialFailure = createService({
      load: vi.fn().mockRejectedValue(new Error("database unavailable")),
    });
    const initialObserved = createLifecycle();
    await loadInitialWorkspace(
      initialFailure,
      initialObserved.lifecycle,
      () => true,
    );
    expect(initialObserved.events).toEqual(["fatal:database unavailable"]);

    const fallbackFailure = createService({
      load: vi
        .fn()
        .mockResolvedValueOnce(payload("initial", "zotero"))
        .mockRejectedValueOnce(new Error("cache unavailable")),
      sync: vi.fn().mockRejectedValue(new Error("sync unavailable")),
    });
    const fallbackObserved = createLifecycle();
    await loadInitialWorkspace(
      fallbackFailure,
      fallbackObserved.lifecycle,
      () => true,
    );
    expect(fallbackObserved.events).toEqual([
      "initial:initial",
      "syncing:true",
      "sync-error:sync unavailable",
      "syncing:false",
      "fatal:cache unavailable",
    ]);

    const lateLoad = deferred<WorkspacePayload>();
    const inactiveService = createService({
      load: vi.fn(() => lateLoad.promise),
    });
    const inactiveObserved = createLifecycle();
    let active = true;
    const loading = loadInitialWorkspace(
      inactiveService,
      inactiveObserved.lifecycle,
      () => active,
    );
    active = false;
    lateLoad.resolve(payload("late", "zotero"));
    await loading;
    expect(inactiveObserved.events).toEqual([]);
    expect(inactiveService.sync).not.toHaveBeenCalled();
  });

  it("ignores an initial Zotero synchronization that completes after unmount", async () => {
    const lateSync = deferred<WorkspacePayload>();
    const service = createService({
      load: vi.fn().mockResolvedValue(payload("initial", "zotero")),
      sync: vi.fn(() => lateSync.promise),
    });
    const observed = createLifecycle();
    let active = true;

    const loading = loadInitialWorkspace(
      service,
      observed.lifecycle,
      () => active,
    );
    await Promise.resolve();
    expect(service.sync).toHaveBeenCalledOnce();
    active = false;
    lateSync.resolve(payload("late", "zotero"));
    await loading;

    expect(observed.events).toEqual(["initial:initial", "syncing:true"]);
    expect(observed.data?.source.connectionStatus).toBe("initial");
  });

  it("preserves manual sync success, citation failure, and cached fallback", async () => {
    const synced = payload("synced", "zotero", 1);
    const service = createService({ sync: vi.fn().mockResolvedValue(synced) });
    const observed = createLifecycle({
      advanceAfterSync: vi.fn().mockRejectedValue(new Error("citation failed")),
    });

    await runWorkspaceSourceOperation(service, observed.lifecycle, "sync");

    expect(observed.events).toEqual([
      "syncing:true",
      "sync-error:null",
      "replace:synced",
      "sync-error:citation failed",
      "manual-finished",
      "syncing:false",
    ]);
    expect(service.load).not.toHaveBeenCalled();

    const cached = payload("cached", "zotero");
    const failedService = createService({
      load: vi.fn().mockResolvedValue(cached),
      rebuild: vi.fn().mockRejectedValue("rebuild failed"),
    });
    const failedObserved = createLifecycle();
    await runWorkspaceSourceOperation(
      failedService,
      failedObserved.lifecycle,
      "rebuild",
    );
    expect(failedService.load).toHaveBeenCalledWith(true);
    expect(failedObserved.events).toEqual([
      "syncing:true",
      "sync-error:null",
      "sync-error:rebuild failed",
      "replace:offline",
      "manual-finished",
      "syncing:false",
    ]);
  });

  it("allows overlapping manual loads to finish stale and clear status independently", async () => {
    const first = deferred<WorkspacePayload>();
    const second = deferred<WorkspacePayload>();
    const service = createService({
      sync: vi
        .fn()
        .mockImplementationOnce(() => first.promise)
        .mockImplementationOnce(() => second.promise),
    });
    const observed = createLifecycle();

    const firstRun = runWorkspaceSourceOperation(
      service,
      observed.lifecycle,
      "sync",
    );
    const secondRun = runWorkspaceSourceOperation(
      service,
      observed.lifecycle,
      "sync",
    );
    second.resolve(payload("newer"));
    await secondRun;
    first.resolve(payload("stale"));
    await firstRun;

    expect(
      observed.events.filter((event) => event.startsWith("replace:")),
    ).toEqual(["replace:newer", "replace:stale"]);
    expect(
      observed.events.filter((event) => event === "syncing:false"),
    ).toHaveLength(2);
    expect(observed.data?.source.connectionStatus).toBe("stale");
  });

  it("propagates a failed manual cache fallback after cleaning shared status", async () => {
    const service = createService({
      load: vi.fn().mockRejectedValue(new Error("cache unavailable")),
      sync: vi.fn().mockRejectedValue(new Error("sync unavailable")),
    });
    const observed = createLifecycle();

    await expect(
      runWorkspaceSourceOperation(service, observed.lifecycle, "sync"),
    ).rejects.toThrow("cache unavailable");
    expect(observed.events).toEqual([
      "syncing:true",
      "sync-error:null",
      "sync-error:sync unavailable",
      "manual-finished",
      "syncing:false",
    ]);
  });

  it("switches libraries without a loading flag and preserves stale completion", async () => {
    const first = deferred<WorkspacePayload>();
    const second = deferred<WorkspacePayload>();
    const setActiveLibrary = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const service = createService({ setActiveLibrary });
    const observed = createLifecycle();

    const firstSwitch = switchWorkspaceLibrary(
      service,
      observed.lifecycle,
      "library-a",
    );
    const secondSwitch = switchWorkspaceLibrary(
      service,
      observed.lifecycle,
      "library-b",
    );
    second.resolve(payload("library-b"));
    await secondSwitch;
    first.resolve(payload("library-a"));
    await firstSwitch;

    expect(setActiveLibrary.mock.calls).toEqual([["library-a"], ["library-b"]]);
    expect(observed.events).not.toContain("syncing:true");
    expect(
      observed.events.filter((event) => event.startsWith("replace:")),
    ).toEqual(["replace:library-b", "replace:library-a"]);
    expect(
      observed.events.filter((event) => event === "library-changed"),
    ).toHaveLength(2);
  });

  it("keeps library errors and source-mode selection on the existing service contract", async () => {
    const service = createService({
      setActiveLibrary: vi.fn().mockRejectedValue(new Error("missing library")),
    });
    const observed = createLifecycle();

    await switchWorkspaceLibrary(service, observed.lifecycle, "unavailable");
    selectWorkspaceSourceMode(service, "zotero");

    expect(observed.events).toEqual([
      "sync-error:null",
      "sync-error:missing library",
    ]);
    expect(service.setMode).toHaveBeenCalledWith("zotero");
  });
});

function payload(
  label: string,
  mode: "demo" | "zotero" = "demo",
  queuedDocuments = 0,
): WorkspacePayload {
  return {
    ...browserWorkspace,
    source: {
      ...browserWorkspace.source,
      mode,
      connectionStatus: label,
      health: mode === "demo" ? "demo" : "ready",
    },
    citations: {
      ...browserWorkspace.citations,
      queuedDocuments,
      queuedResolutions: 0,
    },
  };
}

function emptyPayload(label: string): WorkspacePayload {
  const current = payload(label, "zotero");
  return {
    ...current,
    snapshot: {
      ...current.snapshot,
      items: [],
      totalItemCount: 0,
      loadedItemCount: 0,
      libraries: [],
      activeLibraryId: null,
    },
    graph: {
      ...current.graph,
      nodes: [],
      edges: [],
      totalNodeCount: 0,
      hasMore: false,
    },
  };
}

function createService(
  overrides: Partial<WorkspaceService> = {},
): WorkspaceService & Record<string, ReturnType<typeof vi.fn>> {
  return {
    load: vi.fn().mockResolvedValue(payload("loaded")),
    sync: vi.fn().mockResolvedValue(payload("synced")),
    rebuild: vi.fn().mockResolvedValue(payload("rebuilt")),
    syncCorpus: vi.fn().mockResolvedValue(payload("corpus")),
    rebuildCorpus: vi.fn().mockResolvedValue(payload("corpus-rebuilt")),
    setActiveLibrary: vi.fn().mockResolvedValue(payload("library")),
    setMode: vi.fn(),
    ...overrides,
  } as WorkspaceService & Record<string, ReturnType<typeof vi.fn>>;
}

function createLifecycle(overrides: Partial<WorkspaceSourceLifecycle> = {}): {
  lifecycle: WorkspaceSourceLifecycle;
  events: string[];
  data: WorkspacePayload | null;
} {
  const observed = {
    events: [] as string[],
    data: null as WorkspacePayload | null,
  };
  const lifecycle: WorkspaceSourceLifecycle = {
    setInitialData: (next) => {
      observed.data = next;
      observed.events.push(`initial:${next.source.connectionStatus}`);
    },
    replaceData: (next) => {
      observed.data = next;
      observed.events.push(`replace:${next.source.connectionStatus}`);
    },
    setFatalError: (message) => observed.events.push(`fatal:${message}`),
    setSyncing: (syncing) => observed.events.push(`syncing:${syncing}`),
    setSyncError: (message) =>
      observed.events.push(`sync-error:${message ?? "null"}`),
    advanceAfterSync: vi.fn().mockResolvedValue(undefined),
    finishManualSourceOperation: () => observed.events.push("manual-finished"),
    activeLibraryChanged: () => observed.events.push("library-changed"),
    ...overrides,
  };
  return {
    lifecycle,
    events: observed.events,
    get data() {
      return observed.data;
    },
  };
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}
