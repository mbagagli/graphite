import { useCallback, useEffect, useMemo, useState } from "react";

import type { WorkspacePayload, WorkspaceService } from "../services/contracts";

export type WorkspaceSourceOperation = "sync" | "rebuild";

export interface WorkspaceSourceLifecycle {
  setInitialData: (payload: WorkspacePayload) => void;
  replaceData: (payload: WorkspacePayload) => void;
  setFatalError: (message: string) => void;
  setSyncing: (syncing: boolean) => void;
  setSyncError: (message: string | null) => void;
  advanceAfterSync: (
    payload: WorkspacePayload,
    isActive: () => boolean,
    commitData: (payload: WorkspacePayload) => void,
  ) => Promise<void>;
  finishManualSourceOperation: () => void;
  activeLibraryChanged: () => void;
}

interface UseWorkspaceSourceOptions {
  dataReplaced: (payload: WorkspacePayload) => void;
  advanceAfterSync: WorkspaceSourceLifecycle["advanceAfterSync"];
  finishManualSourceOperation: () => void;
  activeLibraryChanged: () => void;
}

export function useWorkspaceSource(
  service: WorkspaceService,
  {
    dataReplaced,
    advanceAfterSync,
    finishManualSourceOperation,
    activeLibraryChanged,
  }: UseWorkspaceSourceOptions,
) {
  const [data, setData] = useState<WorkspacePayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState<string | null>(null);

  const replaceData = useCallback(
    (payload: WorkspacePayload) => {
      setData(payload);
      dataReplaced(payload);
    },
    [dataReplaced],
  );

  const lifecycle = useMemo<WorkspaceSourceLifecycle>(
    () => ({
      setInitialData: setData,
      replaceData,
      setFatalError: setError,
      setSyncing,
      setSyncError,
      advanceAfterSync,
      finishManualSourceOperation,
      activeLibraryChanged,
    }),
    [
      activeLibraryChanged,
      advanceAfterSync,
      finishManualSourceOperation,
      replaceData,
    ],
  );

  useEffect(() => {
    let active = true;
    void loadInitialWorkspace(service, lifecycle, () => active);
    return () => {
      active = false;
    };
  }, [lifecycle, service]);

  const runSourceOperation = useCallback(
    (operation: WorkspaceSourceOperation) =>
      runWorkspaceSourceOperation(service, lifecycle, operation),
    [lifecycle, service],
  );

  const setActiveLibrary = useCallback(
    (libraryId: string) =>
      switchWorkspaceLibrary(service, lifecycle, libraryId),
    [lifecycle, service],
  );

  const setSourceMode = useCallback(
    (mode: "demo" | "zotero") => selectWorkspaceSourceMode(service, mode),
    [service],
  );

  return {
    data,
    setData,
    error,
    syncing,
    setSyncing,
    syncError,
    setSyncError,
    replaceData,
    syncNow: () => runSourceOperation("sync"),
    rebuildCache: () => runSourceOperation("rebuild"),
    setActiveLibrary,
    setSourceMode,
  };
}

export async function loadInitialWorkspace(
  service: WorkspaceService,
  lifecycle: WorkspaceSourceLifecycle,
  isActive: () => boolean,
): Promise<void> {
  try {
    const payload = await service.load();
    if (!isActive()) return;

    lifecycle.setInitialData(payload);
    if (payload.source.mode !== "zotero") return;

    lifecycle.setSyncing(true);
    try {
      let synced: WorkspacePayload;
      try {
        synced = await service.sync();
      } catch (reason: unknown) {
        if (!isActive()) return;
        const message = workspaceErrorMessage(reason);
        lifecycle.setSyncError(message);
        const cached = await service.load(true);
        if (isActive()) {
          lifecycle.setInitialData(withSourceFailure(cached, message));
        }
        return;
      }

      if (!isActive()) return;
      lifecycle.setInitialData(synced);
      lifecycle.setSyncError(null);
      try {
        await lifecycle.advanceAfterSync(
          synced,
          isActive,
          lifecycle.setInitialData,
        );
      } catch (reason: unknown) {
        if (isActive()) lifecycle.setSyncError(workspaceErrorMessage(reason));
      }
    } finally {
      if (isActive()) lifecycle.setSyncing(false);
    }
  } catch (reason: unknown) {
    if (isActive()) lifecycle.setFatalError(workspaceErrorMessage(reason));
  }
}

export async function runWorkspaceSourceOperation(
  service: WorkspaceService,
  lifecycle: WorkspaceSourceLifecycle,
  operation: WorkspaceSourceOperation,
): Promise<void> {
  lifecycle.setSyncing(true);
  lifecycle.setSyncError(null);
  try {
    let payload: WorkspacePayload;
    try {
      payload = await service[operation]();
    } catch (reason: unknown) {
      const message = workspaceErrorMessage(reason);
      lifecycle.setSyncError(message);
      const cached = await service.load(true);
      lifecycle.replaceData(withSourceFailure(cached, message));
      return;
    }

    lifecycle.replaceData(payload);
    if (payload.source.mode === "zotero") {
      try {
        await lifecycle.advanceAfterSync(
          payload,
          () => true,
          lifecycle.replaceData,
        );
      } catch (reason: unknown) {
        lifecycle.setSyncError(workspaceErrorMessage(reason));
      }
    }
  } finally {
    lifecycle.finishManualSourceOperation();
    lifecycle.setSyncing(false);
  }
}

export async function switchWorkspaceLibrary(
  service: WorkspaceService,
  lifecycle: WorkspaceSourceLifecycle,
  libraryId: string,
): Promise<void> {
  lifecycle.setSyncError(null);
  try {
    lifecycle.replaceData(await service.setActiveLibrary(libraryId));
    lifecycle.activeLibraryChanged();
  } catch (reason: unknown) {
    lifecycle.setSyncError(workspaceErrorMessage(reason));
  }
}

export function selectWorkspaceSourceMode(
  service: WorkspaceService,
  mode: "demo" | "zotero",
): void {
  service.setMode(mode);
}

export function workspaceErrorMessage(reason: unknown): string {
  return reason instanceof Error
    ? reason.message
    : typeof reason === "string"
      ? reason
      : "The research workspace could not be loaded.";
}

function withSourceFailure(
  payload: WorkspacePayload,
  message: string,
): WorkspacePayload {
  const incompatible =
    message.includes("Zotero-Server-ID") || message.includes("modern Zotero");
  const hasCache = payload.snapshot.items.length > 0;
  return {
    ...payload,
    source: {
      ...payload.source,
      health: incompatible
        ? "incompatible"
        : hasCache
          ? "offline-cache"
          : "error",
      connectionStatus: incompatible ? "incompatible" : "offline",
      lastError: message,
    },
  };
}
