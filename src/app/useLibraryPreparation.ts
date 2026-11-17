import { useCallback, useEffect, useRef, useState } from "react";
import { isDesktopRuntime } from "../services/runtime";
import type { WorkspacePayload } from "../services/contracts";
import { getDemoStatus } from "../adapters/demo/demoSetup";
import {
  cancelLibraryPreparation,
  getDemoArchiveInfo,
  preparationOperations,
  type DemoArchiveInfo,
} from "../adapters/tauri/libraryPreparation";
import {
  prepareLibrary,
  type PreparationProgress,
} from "../features/library/libraryPreparation";

/** Keep a library preparation run alive while its setup popover is closed. */
export function useLibraryPreparation(
  data: WorkspacePayload | null,
  onData: (data: WorkspacePayload) => void,
) {
  const [progress, setProgress] = useState<PreparationProgress | null>(null);
  const [archive, setArchive] = useState<DemoArchiveInfo | null>(null);
  const [demoCached, setDemoCached] = useState(false);
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const libraryId = data?.source.activeLibraryId;
  const demo = data?.source.mode === "demo";
  const status = progress?.status;

  useEffect(() => {
    if (!isDesktopRuntime || !demo) return;
    let active = true;
    void Promise.all([getDemoArchiveInfo(), getDemoStatus()]).then(
      ([info, available]) => {
        if (!active) return;
        setArchive(info);
        setDemoCached(available.pdfCount === available.totalPdfs);
        setArchiveError(null);
      },
      (reason: unknown) => {
        if (active) setArchiveError(String(reason));
      },
    );
    return () => {
      active = false;
    };
  }, [libraryId, demo, status]);

  useEffect(
    () => () => {
      if (!controller.current) return;
      controller.current.abort();
      void cancelLibraryPreparation().catch(() => {
        // The frontend has stopped; native work is bounded and committed locally.
      });
    },
    [],
  );

  const start = useCallback(
    async (offline = false) => {
      if (controller.current || !data || !libraryId || !isDesktopRuntime)
        return;
      const active = new AbortController();
      controller.current = active;
      try {
        await prepareLibrary(
          data.source.mode,
          libraryId,
          offline,
          preparationOperations,
          active.signal,
          setProgress,
          onData,
        );
      } finally {
        controller.current = null;
      }
    },
    [data, libraryId, onData],
  );

  const cancel = useCallback(async () => {
    controller.current?.abort();
    setProgress((current) =>
      current?.status === "running"
        ? {
            ...current,
            detail: "Cancelling preparation. Completed work will be kept.",
          }
        : current,
    );
    try {
      await cancelLibraryPreparation();
    } catch (reason) {
      setProgress((current) =>
        current
          ? {
              ...current,
              detail: `Stopping after the current operation. ${String(reason)}`,
            }
          : current,
      );
    }
  }, []);

  const current = progress?.libraryId === libraryId ? progress : null;
  return {
    progress: current,
    busy: current?.status === "running",
    archive,
    archiveError,
    demoCached,
    start,
    cancel,
  };
}

export type LibraryPreparationController = ReturnType<
  typeof useLibraryPreparation
>;
