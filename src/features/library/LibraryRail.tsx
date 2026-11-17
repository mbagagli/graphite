import { PanelDivider } from "../../shared/PanelDivider";
import {
  BookOpen,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Folder,
  Layers3,
  Network,
  Sparkles,
} from "lucide-react";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import type {
  LibraryDescriptor,
  ResearchCollection,
  ResearchLens,
} from "../../domain/research";

interface LibraryRailProps {
  paperCount: number;
  collections: ResearchCollection[];
  lenses: ResearchLens[];
  activeCollectionId: string | null;
  activeLensId: string | null;
  onAllPapers: () => void;
  onCollection: (collectionId: string) => void;
  onLens: (lens: ResearchLens) => void;
  onEditLens?: (lens: ResearchLens) => void;
  lensTools?: import("react").ReactNode;
  provenance: "real-demo" | "zotero-local-cache";
  activeLibrary: LibraryDescriptor | undefined;
}

export function LibraryRail({
  paperCount,
  collections,
  lenses,
  activeCollectionId,
  activeLensId,
  onAllPapers,
  onCollection,
  onLens,
  provenance,
  activeLibrary,
  lensTools,
  onEditLens,
}: LibraryRailProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [collectionsExpanded, setCollectionsExpanded] = useState(true);
  const [lensesExpanded, setLensesExpanded] = useState(true);
  const [railWidth, setRailWidth] = useState(getInitialRailWidth);
  const railRef = useRef<HTMLElement>(null);
  const railWidthRef = useRef(railWidth);
  const resizeSession = useRef<{
    pointerId: number;
    startX: number;
    startWidth: number;
  } | null>(null);
  const collectionDepths = useMemo(
    () => getCollectionDepths(collections),
    [collections],
  );
  const orderedCollections = useMemo(
    () => orderCollections(collections),
    [collections],
  );

  useEffect(() => {
    const appShell = railRef.current?.closest<HTMLElement>(".app-shell");
    appShell?.style.setProperty(
      "--workspace-rail-expanded-width",
      `${railWidthRef.current}px`,
    );
    return () => {
      appShell?.style.removeProperty("--workspace-rail-expanded-width");
    };
  }, []);

  const updateRailWidth = (candidate: number, commit: boolean) => {
    const nextWidth = clampRailWidth(candidate);
    railWidthRef.current = nextWidth;
    railRef.current
      ?.closest<HTMLElement>(".app-shell")
      ?.style.setProperty("--workspace-rail-expanded-width", `${nextWidth}px`);
    if (commit) setRailWidth(nextWidth);
  };

  const finishResize = () => {
    if (!resizeSession.current) return;
    resizeSession.current = null;
    railRef.current?.classList.remove("is-resizing");
    setRailWidth(railWidthRef.current);
  };

  const handleResizeKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const widthByKey: Partial<Record<string, number>> = {
      ArrowLeft: railWidthRef.current - RAIL_KEYBOARD_STEP,
      ArrowRight: railWidthRef.current + RAIL_KEYBOARD_STEP,
      Home: RAIL_MIN_WIDTH,
      End: RAIL_MAX_WIDTH,
    };
    const nextWidth = widthByKey[event.key];
    if (nextWidth === undefined) return;
    event.preventDefault();
    updateRailWidth(nextWidth, true);
  };

  return (
    <aside
      ref={railRef}
      className={`library-rail${collapsed ? " is-collapsed" : ""}`}
    >
      <div className="rail-heading">
        {!collapsed && <span>LIBRARY</span>}
        <button
          type="button"
          aria-label={
            collapsed
              ? "Expand library navigation"
              : "Collapse library navigation"
          }
          onClick={() => setCollapsed((current) => !current)}
        >
          {collapsed ? <ChevronRight size={15} /> : <ChevronLeft size={15} />}
        </button>
      </div>

      <nav aria-label="Library navigation">
        <button
          type="button"
          className={`rail-item${
            activeCollectionId === null && activeLensId === null
              ? " is-active"
              : ""
          }`}
          onClick={onAllPapers}
          aria-label={collapsed ? `All papers, ${paperCount}` : undefined}
          aria-current={
            activeCollectionId === null && activeLensId === null
              ? "page"
              : undefined
          }
        >
          <Network size={16} />
          {!collapsed && (
            <>
              <span>All papers</span>
              <small>{paperCount}</small>
            </>
          )}
        </button>
      </nav>

      {!collapsed && (
        <div className="rail-sections resizable-panels">
          <section
            className={`rail-section${collectionsExpanded ? " is-expanded" : ""}`}
          >
            <button
              type="button"
              className="rail-section-heading"
              aria-expanded={collectionsExpanded}
              aria-controls="library-collections"
              onClick={() => setCollectionsExpanded((current) => !current)}
            >
              <span>COLLECTIONS</span>
              <span className="rail-section-affordance">
                <Folder size={12} />
                <ChevronDown size={13} />
              </span>
            </button>
            <div
              id="library-collections"
              className="rail-section-content"
              role="region"
              aria-label="Collections"
              hidden={!collectionsExpanded}
            >
              <div className="rail-list">
                {orderedCollections.map((collection) => (
                  <button
                    type="button"
                    key={collection.id}
                    className={
                      activeCollectionId === collection.id ? "is-active" : ""
                    }
                    aria-current={
                      activeCollectionId === collection.id ? "page" : undefined
                    }
                    onClick={() => onCollection(collection.id)}
                    style={{
                      paddingLeft: `${8 + Math.min(collectionDepths.get(collection.id) ?? 0, 6) * 13}px`,
                    }}
                  >
                    <span className="collection-tick" />
                    <span>{collection.name}</span>
                  </button>
                ))}
                {collections.length === 0 && (
                  <span className="rail-empty">No collections cached</span>
                )}
              </div>
            </div>
          </section>

          <PanelDivider
            storageKey="graphite.library-panel-split"
            label="Resize Collections and Lenses panels"
            enabled={collectionsExpanded && lensesExpanded}
          />

          <section
            className={`rail-section${lensesExpanded ? " is-expanded" : ""}`}
          >
            <button
              type="button"
              className="rail-section-heading"
              aria-expanded={lensesExpanded}
              aria-controls="library-lenses"
              onClick={() => setLensesExpanded((current) => !current)}
            >
              <span>LENSES</span>
              <span className="rail-section-affordance">
                <Sparkles size={12} />
                <ChevronDown size={13} />
              </span>
            </button>
            <div
              id="library-lenses"
              className="rail-section-content"
              role="region"
              aria-label="Lenses"
              hidden={!lensesExpanded}
            >
              {lensTools}
              <div className="lens-list">
                {lenses.map((lens) => (
                  <div className="lens-entry" key={lens.id}>
                    <button
                      type="button"
                      key={lens.id}
                      className={activeLensId === lens.id ? "is-active" : ""}
                      aria-pressed={activeLensId === lens.id}
                      title={`${lens.name}: ${lens.paperIds.length} papers`}
                      onClick={() => onLens(lens)}
                    >
                      <Layers3 size={14} />
                      <span>
                        {lens.name} ({lens.paperIds.length})
                      </span>
                    </button>
                    {onEditLens && (
                      <button
                        type="button"
                        aria-label={`Edit ${lens.name}`}
                        onClick={() => onEditLens(lens)}
                      >
                        Edit
                      </button>
                    )}
                  </div>
                ))}
                {lenses.length === 0 && (
                  <span className="rail-empty">No saved lenses</span>
                )}
              </div>
            </div>
          </section>
        </div>
      )}

      <div className="rail-provenance">
        <BookOpen size={14} />
        {!collapsed && (
          <div>
            <strong>
              {provenance !== "zotero-local-cache"
                ? "Demo library"
                : "Zotero cache"}
            </strong>
            <span>
              {activeLibrary?.name ?? "No library"} / {paperCount} records
            </span>
          </div>
        )}
      </div>

      {!collapsed && (
        <div
          className="library-rail-resize-handle"
          role="separator"
          aria-label="Resize library sidebar"
          aria-orientation="vertical"
          aria-valuemin={RAIL_MIN_WIDTH}
          aria-valuemax={RAIL_MAX_WIDTH}
          aria-valuenow={railWidth}
          aria-valuetext={`${railWidth} pixels`}
          tabIndex={0}
          onKeyDown={handleResizeKeyDown}
          onDoubleClick={() => updateRailWidth(RAIL_DEFAULT_WIDTH, true)}
          onPointerDown={(event: ReactPointerEvent<HTMLDivElement>) => {
            resizeSession.current = {
              pointerId: event.pointerId,
              startX: event.clientX,
              startWidth: railWidthRef.current,
            };
            railRef.current?.classList.add("is-resizing");
            event.currentTarget.setPointerCapture(event.pointerId);
            event.preventDefault();
          }}
          onPointerMove={(event: ReactPointerEvent<HTMLDivElement>) => {
            const session = resizeSession.current;
            if (!session || session.pointerId !== event.pointerId) return;
            updateRailWidth(
              session.startWidth + event.clientX - session.startX,
              false,
            );
          }}
          onPointerUp={(event: ReactPointerEvent<HTMLDivElement>) => {
            if (event.currentTarget.hasPointerCapture(event.pointerId)) {
              event.currentTarget.releasePointerCapture(event.pointerId);
            }
            finishResize();
          }}
          onPointerCancel={finishResize}
          onLostPointerCapture={finishResize}
        />
      )}
    </aside>
  );
}

const RAIL_DEFAULT_WIDTH = 218;
const RAIL_COMPACT_WIDTH = 190;
const RAIL_MIN_WIDTH = 176;
const RAIL_MAX_WIDTH = 360;
const RAIL_KEYBOARD_STEP = 12;

function getInitialRailWidth(): number {
  return window.matchMedia("(max-width: 1180px)").matches
    ? RAIL_COMPACT_WIDTH
    : RAIL_DEFAULT_WIDTH;
}

function clampRailWidth(width: number): number {
  return Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, Math.round(width)));
}

function getCollectionDepths(
  collections: ResearchCollection[],
): Map<string, number> {
  const byId = new Map(
    collections.map((collection) => [collection.id, collection]),
  );
  const depths = new Map<string, number>();
  for (const collection of collections) {
    let current = collection;
    let depth = 0;
    const visited = new Set([collection.id]);
    while (current.parentId && depth < 8) {
      if (visited.has(current.parentId)) break;
      const parent = byId.get(current.parentId);
      if (!parent) break;
      visited.add(parent.id);
      current = parent;
      depth += 1;
    }
    depths.set(collection.id, depth);
  }
  return depths;
}

function orderCollections(
  collections: ResearchCollection[],
): ResearchCollection[] {
  const ids = new Set(collections.map((collection) => collection.id));
  const children = new Map<string | null, ResearchCollection[]>();
  for (const collection of collections) {
    const parentId =
      collection.parentId && ids.has(collection.parentId)
        ? collection.parentId
        : null;
    const group = children.get(parentId) ?? [];
    group.push(collection);
    children.set(parentId, group);
  }
  for (const group of children.values()) {
    group.sort((left, right) => left.name.localeCompare(right.name));
  }
  const ordered: ResearchCollection[] = [];
  const visited = new Set<string>();
  const append = (parentId: string | null) => {
    for (const collection of children.get(parentId) ?? []) {
      if (visited.has(collection.id)) continue;
      visited.add(collection.id);
      ordered.push(collection);
      append(collection.id);
    }
  };
  append(null);
  for (const collection of collections) {
    if (!visited.has(collection.id)) ordered.push(collection);
  }
  return ordered;
}
