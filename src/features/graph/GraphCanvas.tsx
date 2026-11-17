import ForceGraph3D, {
  type NodeObject,
  type LinkObject,
  type ForceGraph3DInstance,
} from "3d-force-graph";
import { useEffect, useRef, useState, type RefObject } from "react";
import type {
  GraphNode,
  ResearchGraph,
  GraphLayerVisibility,
} from "../../domain/graph";
import type { Topic } from "../../domain/research";
import { edgeColors, strengthenNodeColor } from "./graphColors";
import { createNodeRaycast } from "./graphPicking";
import {
  projectionKey,
  rebasePaperCamera,
  rendererData,
  placeNodes,
  type GraphMode,
  type GraphGrouping,
  type GraphPosition,
  type GraphCamera,
  type GraphLayout,
  type GraphLayouts,
  type GraphSession,
} from "./graphLayout";
import {
  GraphTransition,
  paperCamera,
  interpolatePoint,
  interpolateCamera,
  arrivalCamera,
  arrivalDestination,
  takeArrival,
  ARRIVAL_MS,
  MODE_TRANSITION_MS,
  PROJECTION_TRANSITION_MS,
  CAMERA_ACTION_MS,
} from "./graphMotion";

export interface GraphFocusRequest {
  id: string;
  serial: number;
  dive?: boolean;
}

export interface GraphControls {
  capture: () => void;
  focus: (id: string, dive?: boolean) => boolean;
  fit: (ids?: string[]) => void;
  zoom: (factor: number) => void;
  mode: (mode: GraphMode) => void;
  navigate: () => void;
  key: (key: string) => void;
}
type GraphRenderer = ForceGraph3DInstance<
  NodeObject,
  LinkObject & { id?: string }
>;
interface OrbitControls {
  target: GraphPosition;
  enableDamping: boolean;
  enableRotate: boolean;
  mouseButtons: { LEFT: number; MIDDLE: number; RIGHT: number };
  touches: { ONE: number; TWO: number };
  addEventListener: (event: string, listener: () => void) => void;
  removeEventListener: (event: string, listener: () => void) => void;
}
interface Props {
  graph: ResearchGraph;
  mode: GraphMode;
  grouping: GraphGrouping;
  topics: Topic[];
  selectedIds: string[];
  activePaperId?: string | null;
  selectionConnectionsOnly?: boolean;
  restoreSerial?: number;
  highlightedIds: string[];
  lensIds?: string[];
  layers: GraphLayerVisibility;
  theme: string;
  controlsRef: RefObject<GraphControls | null>;
  sessionRef: RefObject<GraphSession>;
  focusRequest?: GraphFocusRequest | null;
  focusSerialRef: RefObject<number>;
  onSelectNode: (node: GraphNode, additive: boolean) => void;
  onHoverNode: (node: GraphNode | null) => void;
  onSelectEdge: (id: string) => void;
  onClearSelection: () => void;
  onFailure: () => void;
}
const reducedMotion = () =>
  matchMedia("(prefers-reduced-motion: reduce)").matches;
const label = (text: string) => {
  const element = document.createElement("span");
  element.textContent = text;
  element.className = "graph-label";
  return element;
};

export function GraphCanvas(props: Props) {
  const container = useRef<HTMLDivElement>(null);
  const renderer = useRef<GraphRenderer | null>(null);
  const latest = useRef(props);
  const loadGraph = useRef<(() => void) | null>(null);
  const [displayedGraph, setDisplayedGraph] = useState(props.graph);
  const [preparing, setPreparing] = useState(true);
  const projectionReady = useRef(false);
  useEffect(() => {
    latest.current = props;
  });
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let instance: GraphRenderer | undefined;
    let worker: Worker | undefined;
    let observer: ResizeObserver | undefined;
    let disposed = false;
    let ready = false;
    let frame = 0;
    let clickTimer: ReturnType<typeof setTimeout> | undefined;
    let currentMode = latest.current.mode;
    let data: ReturnType<typeof rendererData>;
    let layouts: GraphLayouts;
    const transition = new GraphTransition();
    const session = latest.current.sessionRef.current;
    let requestedGrouping = latest.current.grouping;
    let key = projectionKey(latest.current.graph, requestedGrouping);
    let requestedGraph = latest.current.graph;
    const media = matchMedia("(prefers-reduced-motion: reduce)");
    const cleanup: (() => void)[] = [() => clearTimeout(clickTimer)];
    const fail = (event?: Event) => {
      event?.preventDefault();
      transition.cancel();
      session.startupPlayed = true;
      if (!disposed) latest.current.onFailure();
    };
    let domainNodes = new Map(
      latest.current.graph.nodes.map((node) => [node.id, node]),
    );
    let domainEdges = new Map(
      latest.current.graph.edges.map((edge) => [edge.id, edge]),
    );
    try {
      instance = new ForceGraph3D(element, {
        controlType: "orbit",
        rendererConfig: { alpha: true, antialias: true },
      }) as GraphRenderer;
      const view = instance;
      renderer.current = view;
      for (const light of view.lights()) {
        if (light.type === "AmbientLight") light.intensity = Math.PI * 0.65;
        if (light.type === "DirectionalLight") {
          light.intensity = Math.PI * 0.85;
          light.position.set(-1, 2, 3);
        }
      }
      const orbit = view.controls() as OrbitControls;
      orbit.enableDamping = false;
      const camera = (): GraphCamera => ({
        position: { ...view.cameraPosition() },
        target: { x: orbit.target.x, y: orbit.target.y, z: orbit.target.z },
      });
      const setCamera = (value: GraphCamera) =>
        view.cameraPosition(value.position, value.target, 0);
      const configureControls = () => {
        orbit.enableRotate = currentMode === "3d";
        orbit.mouseButtons.LEFT = currentMode === "3d" ? 0 : 2;
        orbit.touches.ONE = currentMode === "3d" ? 0 : 1;
      };
      const fitCamera = (layout: GraphLayout, ids?: string[]): GraphCamera => {
        const included = ids?.length ? new Set(ids) : null;
        const points = [...layout]
          .filter(([id]) => !included || included.has(id))
          .map(([, point]) => point);
        if (!points.length)
          return {
            target: { x: 0, y: 0, z: 0 },
            position: { x: 0, y: 0, z: 200 },
          };
        const min = { x: Infinity, y: Infinity, z: Infinity };
        const max = { x: -Infinity, y: -Infinity, z: -Infinity };
        for (const point of points)
          for (const axis of ["x", "y", "z"] as const) {
            min[axis] = Math.min(min[axis], point[axis]);
            max[axis] = Math.max(max[axis], point[axis]);
          }
        const target = interpolatePoint(min, max, 0.5);
        const aspect = Math.max(
          0.2,
          element.clientWidth / element.clientHeight,
        );
        const fov = (view.camera().fov * Math.PI) / 180;
        const distance =
          Math.max(
            90,
            (max.y - min.y) / (2 * Math.tan(fov / 2)),
            (max.x - min.x) / (2 * Math.tan(fov / 2) * aspect),
          ) *
            1.25 +
          (max.z - min.z) / 2;
        return { target, position: { ...target, z: target.z + distance } };
      };
      const remember = () => {
        if (ready && !transition.kind) session.cameras[currentMode] = camera();
      };
      const navigate = () => {
        clearTimeout(clickTimer);
        if (transition.kind === "mode" || transition.kind === "projection")
          transition.finish();
        else transition.cancel();
        remember();
      };
      const moveCamera = (target: GraphCamera, duration = CAMERA_ACTION_MS) => {
        const start = camera();
        transition.run(
          "camera",
          reducedMotion() ? 0 : duration,
          (t) => setCamera(interpolateCamera(start, target, t)),
          remember,
        );
      };
      const applyMode = (next: GraphMode) => {
        if (!ready || next === currentMode) return;
        remember();
        transition.cancel();
        const starts = new Map(
          data.nodes.map(({ id, x, y, z }) => [id, { x, y, z }]),
        );
        currentMode = next;
        configureControls();
        const startCamera = camera();
        const targetCamera =
          session.cameras[next] ?? fitCamera(layouts.positions[next]);
        const target = layouts.positions[next];
        transition.run(
          "mode",
          reducedMotion() ? 0 : MODE_TRANSITION_MS,
          (t) => {
            for (const node of data.nodes) {
              const point = interpolatePoint(
                starts.get(node.id)!,
                target.get(node.id)!,
                t,
              );
              node.x = node.fx = point.x;
              node.y = node.fy = point.y;
              node.z = node.fz = point.z;
            }
            view.d3ReheatSimulation();
            setCamera(
              interpolateCamera(
                startCamera,
                targetCamera,
                t,
                ((next === "2d" ? 1 : -1) * Math.PI) / 36,
              ),
            );
          },
          () => {
            placeNodes(data.nodes, target);
            remember();
          },
        );
      };
      const focus = (id: string, dive = true) => {
        if (!ready) return false;
        navigate();
        const node = data.nodes.find((candidate) => candidate.id === id);
        if (!node) return false;
        const matrix = view.camera().matrixWorld.elements;
        moveCamera(
          paperCamera(
            camera(),
            node,
            { x: matrix[0], y: matrix[1], z: matrix[2] },
            element.clientWidth / Math.max(1, element.clientHeight),
            view.camera().fov,
            dive,
          ),
        );
        return true;
      };
      latest.current.controlsRef.current = {
        capture: () => {
          navigate();
          saveSession();
        },
        focus,
        mode: applyMode,
        navigate,
        fit: (ids) => {
          if (ready) {
            navigate();
            moveCamera(fitCamera(layouts.positions[currentMode], ids));
          }
        },
        zoom: (factor) => {
          if (!ready) return;
          navigate();
          const value = camera();
          moveCamera({
            target: value.target,
            position: interpolatePoint(value.target, value.position, factor),
          });
        },
        key: (key) => {
          if (!ready) return;
          navigate();
          const value = camera();
          const horizontal = key === "ArrowLeft" || key === "ArrowRight";
          const sign = key === "ArrowLeft" || key === "ArrowDown" ? -1 : 1;
          if (currentMode === "2d") {
            const step = Math.abs(value.position.z) * 0.05 * sign;
            const axis = horizontal ? "x" : "y";
            value.position[axis] += step;
            value.target[axis] += step;
          } else {
            const point = {
              x: value.position.x - value.target.x,
              y: value.position.y - value.target.y,
              z: value.position.z - value.target.z,
            };
            const axis = horizontal ? "x" : "y";
            const c = Math.cos(0.15 * sign),
              s = Math.sin(0.15 * sign);
            value.position[axis] =
              value.target[axis] + point[axis] * c - point.z * s;
            value.position.z = value.target.z + point[axis] * s + point.z * c;
          }
          moveCamera(value);
        },
      };
      configureControls();
      const raycastNode = createNodeRaycast(() => element.clientHeight);
      let lastClick = { id: "", time: 0 };
      view
        .backgroundColor("#00000000")
        .showNavInfo(false)
        .enableNodeDrag(false)
        .nodeResolution(32)
        .nodeOpacity(1)
        .nodePositionUpdate((object) => {
          object.raycast = raycastNode;
          return false;
        })
        .linkResolution(4)
        .warmupTicks(0)
        .cooldownTicks(0)
        .nodeLabel(() => "")
        .onNodeHover((node) =>
          latest.current.onHoverNode(
            node ? (domainNodes.get(String(node.id)) ?? null) : null,
          ),
        )
        .linkLabel((link) =>
          label(
            domainEdges.get(String(link.id))?.evidenceLabel ?? "Relationship",
          ),
        )
        .linkDirectionalArrowLength((link) =>
          domainEdges.get(String(link.id))?.directed ? 4 : 0,
        )
        .linkDirectionalArrowRelPos(1)
        .onNodeClick((node, event) => {
          const id = String(node.id),
            domain = domainNodes.get(id);
          if (domain)
            latest.current.onSelectNode(
              domain,
              event.shiftKey || event.metaKey,
            );
          const now = performance.now();
          const doubleClick = lastClick.id === id && now - lastClick.time < 350;
          clearTimeout(clickTimer);
          if (doubleClick) focus(id);
          else clickTimer = setTimeout(() => focus(id, false), 350);
          lastClick = doubleClick ? { id: "", time: 0 } : { id, time: now };
        })
        .onLinkClick((link) => latest.current.onSelectEdge(String(link.id)))
        .onBackgroundClick(() => latest.current.onClearSelection());
      const resize = () =>
        view.width(element.clientWidth).height(element.clientHeight);
      resize();
      observer = new ResizeObserver(resize);
      observer.observe(element);
      const canvas = view.renderer().domElement;
      canvas.addEventListener("webglcontextlost", fail);
      element.addEventListener("pointerdown", navigate, true);
      element.addEventListener("wheel", navigate, {
        capture: true,
        passive: true,
      });
      const motionChanged = () => {
        orbit.enableDamping = false;
        if (media.matches) transition.finish();
      };
      media.addEventListener("change", motionChanged);
      cleanup.push(() => {
        canvas.removeEventListener("webglcontextlost", fail);
        element.removeEventListener("pointerdown", navigate, true);
        element.removeEventListener("wheel", navigate, true);
        media.removeEventListener("change", motionChanged);
      });
      const saveSession = () => {
        if (!ready) return;
        const id =
          latest.current.activePaperId ?? latest.current.selectedIds[0];
        session.anchors = {};
        for (const mode of ["2d", "3d"] as const) {
          const point =
            mode === currentMode
              ? data.nodes.find((node) => node.id === id)
              : layouts.positions[mode].get(id);
          if (point)
            session.anchors[mode] = {
              id,
              position: { x: point.x, y: point.y, z: point.z },
            };
        }
        session.cameras[currentMode] = camera();
      };
      const install = (result: GraphLayouts) => {
        if (disposed) return;
        const restoring = Boolean(session.restore);
        if (ready && !restoring) {
          if (transition.kind === "projection" || transition.kind === "mode")
            transition.finish();
          else transition.cancel();
          saveSession();
        }
        const previousCamera = ready && !restoring ? camera() : null;
        const previousPositions: GraphLayout = new Map(
          ready ? data.nodes.map(({ id, x, y, z }) => [id, { x, y, z }]) : [],
        );
        layouts = result;
        if (!restoring && session.layouts?.key !== key) {
          for (const mode of ["2d", "3d"] as const) {
            const anchor = session.anchors?.[mode];
            const previousCamera = session.cameras[mode];
            const position = anchor && result.positions[mode].get(anchor.id);
            if (
              anchor &&
              position &&
              previousCamera &&
              latest.current.selectedIds.includes(anchor.id)
            ) {
              session.cameras[mode] = rebasePaperCamera(
                previousCamera,
                anchor.position,
                position,
              );
            } else delete session.cameras[mode];
          }
        }
        session.layouts = result;
        currentMode = latest.current.mode;
        configureControls();
        data = rendererData(
          latest.current.graph,
          result.positions[currentMode],
        );
        placeNodes(data.nodes, result.positions[currentMode]);
        const startPositions: GraphLayout = new Map(
          data.nodes.map((node) => [
            node.id,
            previousPositions.get(node.id) ?? previousCamera?.target ?? node,
          ]),
        );
        if (previousCamera) placeNodes(data.nodes, startPositions);
        domainNodes = new Map(
          latest.current.graph.nodes.map((node) => [node.id, node]),
        );
        domainEdges = new Map(
          latest.current.graph.edges.map((edge) => [edge.id, edge]),
        );
        setDisplayedGraph(latest.current.graph);
        view.graphData(data);
        element.dataset.layoutMs = result.preparationMs.toFixed(1);
        frame = requestAnimationFrame(() => {
          if (disposed) return;
          ready = true;
          projectionReady.current = true;
          const destination =
            session.cameras[currentMode] ??
            fitCamera(result.positions[currentMode], latest.current.lensIds);
          setCamera(previousCamera ?? destination);
          const request = latest.current.focusRequest;
          session.restore = false;
          if (restoring) {
            session.startupPlayed = true;
            if (request) latest.current.focusSerialRef.current = request.serial;
          } else if (
            request &&
            latest.current.focusSerialRef.current !== request.serial
          ) {
            session.startupPlayed = true;
            focus(request.id, request.dive);
            latest.current.focusSerialRef.current = request.serial;
          } else if (previousCamera) {
            transition.run(
              "projection",
              reducedMotion() ? 0 : PROJECTION_TRANSITION_MS,
              (t) => {
                for (const node of data.nodes) {
                  const point = interpolatePoint(
                    startPositions.get(node.id)!,
                    result.positions[currentMode].get(node.id)!,
                    t,
                  );
                  node.x = node.fx = point.x;
                  node.y = node.fy = point.y;
                  node.z = node.fz = point.z;
                }
                view.d3ReheatSimulation();
                setCamera(interpolateCamera(previousCamera, destination, t));
              },
              remember,
            );
          } else if (takeArrival(session, data.nodes.length)) {
            if (currentMode === "3d") {
              const start = arrivalCamera(destination);
              const finish = arrivalDestination(destination);
              transition.run(
                "arrival",
                reducedMotion() ? 0 : ARRIVAL_MS,
                (t) => setCamera(interpolateCamera(start, finish, t)),
                remember,
              );
            }
          }
          setPreparing(false);
        });
      };
      const prepare = () => {
        projectionReady.current = false;
        setPreparing(true);
        worker?.terminate();
        cancelAnimationFrame(frame);
        requestedGraph = latest.current.graph;
        requestedGrouping = latest.current.grouping;
        key = projectionKey(requestedGraph, requestedGrouping);
        if (session.layouts?.key === key) {
          install(session.layouts);
          return;
        }
        setPreparing(true);
        const requestKey = key;
        worker = new Worker(
          new URL("./graphLayout.worker.ts", import.meta.url),
          { type: "module" },
        );
        worker.onmessage = (event: MessageEvent<GraphLayouts>) => {
          if (requestKey !== key || disposed) return;
          install(event.data);
          worker?.terminate();
        };
        worker.onerror = () => fail();
        worker.postMessage({
          graph: requestedGraph,
          grouping: requestedGrouping,
        });
      };
      loadGraph.current = () => {
        if (
          requestedGraph !== latest.current.graph ||
          requestedGrouping !== latest.current.grouping ||
          session.restore
        )
          prepare();
      };
      cleanup.push(saveSession);
      prepare();
    } catch {
      fail();
    }
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      if (transition.kind === "mode" || transition.kind === "projection")
        transition.finish();
      transition.cancel();
      worker?.terminate();
      observer?.disconnect();
      for (const dispose of cleanup) dispose();
      if (instance) {
        instance._destructor();
      }
      renderer.current = null;
      latest.current.controlsRef.current = null;
      loadGraph.current = null;
    };
  }, []);
  useEffect(() => {
    loadGraph.current?.();
  }, [props.graph, props.grouping, props.restoreSerial]);
  useEffect(() => {
    props.controlsRef.current?.mode(props.mode);
  }, [props.mode, props.controlsRef]);
  useEffect(() => {
    const request = props.focusRequest;
    const serialRef = props.focusSerialRef;
    if (
      request &&
      props.focusSerialRef.current !== request.serial &&
      !preparing &&
      projectionReady.current
    ) {
      if (props.controlsRef.current?.focus(request.id, request.dive))
        serialRef.current = request.serial;
    }
  }, [props.focusRequest, props.controlsRef, props.focusSerialRef, preparing]);
  useEffect(() => {
    const view = renderer.current;
    if (!view) return;
    const selected = new Set(props.selectedIds);
    const highlighted = new Set(props.highlightedIds);
    const nodes = new Map(displayedGraph.nodes.map((node) => [node.id, node]));
    const edges = new Map(displayedGraph.edges.map((edge) => [edge.id, edge]));
    const topics = new Map(
      props.topics
        .filter((topic) => topic.origin === "semantic-cluster")
        .map((topic) => [
          topic.id,
          props.theme === "light"
            ? strengthenNodeColor(topic.color)
            : topic.color,
        ]),
    );
    const neighbors = new Set(
      displayedGraph.edges
        .filter((edge) => props.layers[edge.relation])
        .flatMap((edge) =>
          selected.has(edge.source)
            ? [edge.target]
            : selected.has(edge.target)
              ? [edge.source]
              : [],
        ),
    );
    const styles = getComputedStyle(document.documentElement);
    view
      .nodeColor((node) =>
        selected.has(String(node.id))
          ? styles.getPropertyValue("--graph-node-selected").trim()
          : neighbors.has(String(node.id))
            ? styles.getPropertyValue("--graph-node-hover").trim()
            : highlighted.has(String(node.id))
              ? styles.getPropertyValue("--graph-node-highlight").trim()
              : selected.size || highlighted.size
                ? styles.getPropertyValue("--graph-node-dimmed").trim()
                : nodes.get(String(node.id))?.entityKind === "external-work"
                  ? styles.getPropertyValue("--graph-node-external").trim()
                  : (topics.get(nodes.get(String(node.id))?.topicId ?? "") ??
                    styles.getPropertyValue("--color-text-muted").trim()),
      )
      .nodeVal((node) => (selected.has(String(node.id)) ? 35 : 10))
      .linkVisibility((link) => {
        const edge = edges.get(String(link.id))!;
        return (
          props.layers[edge.relation] &&
          (!props.selectionConnectionsOnly ||
            selected.has(edge.source) ||
            selected.has(edge.target))
        );
      })
      .linkColor((link) => {
        const edge = edges.get(String(link.id))!;
        const visible = props.layers[edge.relation];
        const touches = selected.has(edge.source) || selected.has(edge.target);
        const both =
          highlighted.has(edge.source) && highlighted.has(edge.target);
        const opacity = touches
          ? visible
            ? 0.86
            : 0.04
          : both
            ? visible
              ? 0.62
              : 0.03
            : selected.size || highlighted.size
              ? visible
                ? 0.08
                : 0.015
              : visible
                ? props.theme === "light"
                  ? 0.7
                  : 0.4
                : 0.025;
        return (
          "#" +
          edgeColors[edge.relation]
            .map((channel) => channel.toString(16).padStart(2, "0"))
            .join("") +
          Math.round(opacity * 255)
            .toString(16)
            .padStart(2, "0")
        );
      })
      .linkOpacity(1);
  }, [
    displayedGraph,
    props.topics,
    props.selectedIds,
    props.selectionConnectionsOnly,
    props.highlightedIds,
    props.layers,
    props.theme,
  ]);
  return (
    <>
      <div
        ref={container}
        className="graph-canvas graph-network-canvas"
        tabIndex={0}
        aria-label={`${props.mode.toUpperCase()} graph. Arrow keys ${props.mode === "3d" ? "orbit" : "pan"}, plus and minus zoom, Home fits graph.`}
        onKeyDown={(event) => {
          if (
            ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
              event.key,
            )
          ) {
            event.preventDefault();
            props.controlsRef.current?.key(event.key);
          } else if (["+", "=", "-", "Home"].includes(event.key)) {
            event.preventDefault();
            if (event.key === "Home") props.controlsRef.current?.fit();
            else
              props.controlsRef.current?.zoom(event.key === "-" ? 1.25 : 0.8);
          }
        }}
      />
      {preparing && (
        <div className="graph-preparing" role="status">
          Preparing network...
        </div>
      )}
    </>
  );
}
