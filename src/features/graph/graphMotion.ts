import type { GraphCamera, GraphPosition, GraphSession } from "./graphLayout";

export const ARRIVAL_MS = 1800;
export const MODE_TRANSITION_MS = 600;
export const PROJECTION_TRANSITION_MS = 900;
export const CAMERA_ACTION_MS = 140;
export const ARRIVAL_DISTANCE = 3;
export const ARRIVAL_FINISH_DISTANCE = 0.45;
const ARRIVAL_YAW = Math.PI / 12;
const ARRIVAL_ELEVATION = (Math.PI * 8) / 180;

export function interpolatePoint(
  a: GraphPosition,
  b: GraphPosition,
  t: number,
): GraphPosition {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
    z: a.z + (b.z - a.z) * t,
  };
}

export function interpolateCamera(
  a: GraphCamera,
  b: GraphCamera,
  t: number,
  arc = 0,
): GraphCamera {
  if (t === 0) return a;
  if (t === 1) return b;
  const spherical = (camera: GraphCamera) => {
    const x = camera.position.x - camera.target.x;
    const y = camera.position.y - camera.target.y;
    const z = camera.position.z - camera.target.z;
    const radius = Math.hypot(x, y, z) || 1;
    return {
      radius,
      yaw: Math.atan2(x, z),
      elevation: Math.asin(Math.max(-1, Math.min(1, y / radius))),
    };
  };
  const start = spherical(a),
    end = spherical(b);
  const delta = Math.atan2(
    Math.sin(end.yaw - start.yaw),
    Math.cos(end.yaw - start.yaw),
  );
  const bend = Math.sin(Math.PI * t) * arc;
  const yaw = start.yaw + delta * t + bend;
  const elevation =
    start.elevation + (end.elevation - start.elevation) * t + bend / 2;
  const radius = start.radius + (end.radius - start.radius) * t;
  const target = interpolatePoint(a.target, b.target, t);
  return {
    target,
    position: {
      x: target.x + radius * Math.cos(elevation) * Math.sin(yaw),
      y: target.y + radius * Math.sin(elevation),
      z: target.z + radius * Math.cos(elevation) * Math.cos(yaw),
    },
  };
}

export function arrivalCamera(destination: GraphCamera): GraphCamera {
  const radius =
    Math.hypot(
      destination.position.x - destination.target.x,
      destination.position.y - destination.target.y,
      destination.position.z - destination.target.z,
    ) * ARRIVAL_DISTANCE;
  return {
    target: { ...destination.target },
    position: {
      x:
        destination.target.x +
        radius * Math.cos(ARRIVAL_ELEVATION) * Math.sin(ARRIVAL_YAW),
      y: destination.target.y + radius * Math.sin(ARRIVAL_ELEVATION),
      z:
        destination.target.z +
        radius * Math.cos(ARRIVAL_ELEVATION) * Math.cos(ARRIVAL_YAW),
    },
  };
}

/** Own the single animation frame loop, including interruption and disposal. */
export class GraphTransition {
  private frame = 0;
  private complete: (() => void) | null = null;
  kind: "arrival" | "mode" | "projection" | "camera" | null = null;

  run(
    kind: NonNullable<GraphTransition["kind"]>,
    duration: number,
    update: (progress: number) => void,
    done: () => void = () => {},
  ) {
    this.cancel();
    this.kind = kind;
    this.complete = () => {
      update(1);
      done();
    };
    if (!duration) {
      this.finish();
      return;
    }
    const started = performance.now();
    update(0);
    const tick = (now: number) => {
      const progress = Math.min(1, (now - started) / duration);
      if (progress === 1) {
        this.finish();
        return;
      }
      update(1 - Math.pow(1 - progress, 3));
      this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }

  cancel() {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.complete = null;
    this.kind = null;
  }

  finish() {
    const complete = this.complete;
    this.cancel();
    complete?.();
  }
}

export function takeArrival(session: GraphSession, nodeCount: number): boolean {
  if (session.startupPlayed || nodeCount === 0) return false;
  session.startupPlayed = true;
  return true;
}

export function arrivalDestination(overview: GraphCamera): GraphCamera {
  return {
    target: { ...overview.target },
    position: interpolatePoint(
      overview.target,
      overview.position,
      ARRIVAL_FINISH_DISTANCE,
    ),
  };
}

const PAPER_SCREEN_X = 0.35;
const MIN_FOCUS_DISTANCE = 240;
const FOCUS_DISTANCE_RATIO = 0.65;

/** Frame a paper left of center, preserving scale unless diving. */
export function paperCamera(
  current: GraphCamera,
  paper: GraphPosition,
  right: GraphPosition,
  aspect: number,
  fovDegrees: number,
  dive: boolean,
): GraphCamera {
  const delta = {
    x: current.position.x - current.target.x,
    y: current.position.y - current.target.y,
    z: current.position.z - current.target.z,
  };
  const previousDistance = Math.hypot(delta.x, delta.y, delta.z) || 1;
  const distance = dive
    ? Math.max(MIN_FOCUS_DISTANCE, previousDistance * FOCUS_DISTANCE_RATIO)
    : previousDistance;
  const offset =
    (1 - 2 * PAPER_SCREEN_X) *
    distance *
    Math.tan((fovDegrees * Math.PI) / 360) *
    aspect;
  const target = {
    x: paper.x + right.x * offset,
    y: paper.y + right.y * offset,
    z: paper.z + right.z * offset,
  };
  return {
    target,
    position: {
      x: target.x + (delta.x / previousDistance) * distance,
      y: target.y + (delta.y / previousDistance) * distance,
      z: target.z + (delta.z / previousDistance) * distance,
    },
  };
}
