import { rebasePaperCamera } from "./graphLayout";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  GraphTransition,
  paperCamera,
  arrivalCamera,
  arrivalDestination,
  takeArrival,
  interpolateCamera,
  interpolatePoint,
} from "./graphMotion";

describe("graph motion", () => {
  let callback: FrameRequestCallback;
  beforeEach(() => {
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn((next: FrameRequestCallback) => {
        callback = next;
        return 1;
      }),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.spyOn(performance, "now").mockReturnValue(0);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });
  it("approaches from three times the distance without crossing the graph", () => {
    const end = {
      target: { x: 0, y: 0, z: 0 },
      position: { x: 0, y: 0, z: 100 },
    };
    const start = arrivalCamera(end);
    expect(
      Math.hypot(start.position.x, start.position.y, start.position.z),
    ).toBeCloseTo(300);
    expect(interpolateCamera(start, end, 0)).toEqual(start);
    expect(interpolateCamera(start, end, 1)).toEqual(end);
    expect(interpolateCamera(start, end, 0.5).position.z).toBeGreaterThan(100);
  });
  it("finishes aggregation before navigation and skips motion when reduced", () => {
    const motion = new GraphTransition();
    const update = vi.fn();
    motion.run("projection", 900, update);
    callback(300);
    expect(update.mock.lastCall![0]).toBeGreaterThan(0);
    expect(update.mock.lastCall![0]).toBeLessThan(1);
    motion.finish();
    expect(update).toHaveBeenLastCalledWith(1);
    motion.run("projection", 0, update);
    expect(update).toHaveBeenLastCalledWith(1);
    expect(motion.kind).toBeNull();
  });
  it("cancels an arrival without applying its destination", () => {
    const motion = new GraphTransition(),
      update = vi.fn(),
      done = vi.fn();
    motion.run("arrival", 1200, update, done);
    callback(300);
    motion.cancel();
    expect(done).not.toHaveBeenCalled();
    expect(update.mock.lastCall![0]).toBeLessThan(1);
    expect(motion.kind).toBeNull();
  });
  it("reverses from current positions, finishing only the latest transition", () => {
    const motion = new GraphTransition();
    const a = { x: 0, y: 0, z: 100 },
      b = { x: 20, y: 10, z: 0 };
    let point = a;
    const oldDone = vi.fn();
    motion.run(
      "mode",
      600,
      (t) => {
        point = interpolatePoint(a, b, t);
      },
      oldDone,
    );
    callback(200);
    const interrupted = point;
    motion.run("mode", 600, (t) => {
      point = interpolatePoint(interrupted, a, t);
    });
    expect(point).toEqual(interrupted);
    motion.finish();
    expect(point).toEqual(a);
    expect(oldDone).not.toHaveBeenCalled();
  });
  it("applies reduced-motion destinations synchronously and disposes frames", () => {
    const motion = new GraphTransition(),
      update = vi.fn(),
      done = vi.fn();
    motion.run("mode", 0, update, done);
    expect(update).toHaveBeenCalledExactlyOnceWith(1);
    expect(done).toHaveBeenCalledOnce();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
    motion.run("camera", 300, update);
    motion.cancel();
    expect(cancelAnimationFrame).toHaveBeenCalled();
  });
});

it("plays arrival once after nonempty data, not on Reader or projection returns", () => {
  const session = { cameras: {}, startupPlayed: false };
  expect(takeArrival(session, 0)).toBe(false);
  expect(takeArrival(session, 600)).toBe(true);
  expect(takeArrival(session, 600)).toBe(false);
  expect(takeArrival(session, 1200)).toBe(false);
});

it("adds a gentle arc even when both cameras initially face the plane", () => {
  const camera = {
    position: { x: 0, y: 0, z: 100 },
    target: { x: 0, y: 0, z: 0 },
  };
  expect(
    interpolateCamera(camera, camera, 0.5, Math.PI / 36).position.x,
  ).toBeGreaterThan(0);
  expect(interpolateCamera(camera, camera, 1, Math.PI / 36)).toEqual(camera);
});

it("arrives inside the overview without changing its center or mutating it", () => {
  const overview = {
    target: { x: 10, y: 20, z: 30 },
    position: { x: 10, y: 20, z: 230 },
  };
  const before = JSON.stringify(overview);
  expect(arrivalDestination(overview)).toEqual({
    target: overview.target,
    position: { x: 10, y: 20, z: 120 },
  });
  expect(JSON.stringify(overview)).toBe(before);
});

describe("paper framing", () => {
  const current = {
    target: { x: 0, y: 0, z: 0 },
    position: { x: 0, y: 0, z: 600 },
  };
  const paper = { x: 100, y: 20, z: 40 };
  const right = { x: 1, y: 0, z: 0 };
  it("recenters without zooming and places the paper at 35 percent width", () => {
    const result = paperCamera(current, paper, right, 2, 60, false);
    expect(result.position.z - result.target.z).toBe(600);
    expect(result.target.y).toBe(paper.y);
    const halfWidth = 600 * Math.tan(Math.PI / 6) * 2;
    expect(0.5 + (paper.x - result.target.x) / (2 * halfWidth)).toBeCloseTo(
      0.35,
    );
  });
  it("dives moderately and stops before oversized closeups", () => {
    const first = paperCamera(current, paper, right, 2, 60, true);
    expect(first.position.z - first.target.z).toBe(390);
    let next = first;
    for (let i = 0; i < 10; i++)
      next = paperCamera(next, paper, right, 2, 60, true);
    expect(next.position.z - next.target.z).toBe(240);
  });
  it("preserves an oblique camera direction during recentering", () => {
    const angled = { ...current, position: { x: 300, y: 400, z: 0 } };
    const result = paperCamera(
      angled,
      paper,
      { x: 0, y: 0, z: -1 },
      1,
      60,
      false,
    );
    expect(result.position.x - result.target.x).toBeCloseTo(300);
    expect(result.position.y - result.target.y).toBeCloseTo(400);
    expect(result.target.z).toBeLessThan(paper.z);
  });
});

it("keeps the selected paper fixed throughout a projection aggregation", () => {
  const start = { x: 30, y: -40, z: 50 };
  const end = { x: -120, y: 20, z: 0 };
  const camera = {
    position: { x: 100, y: 120, z: 400 },
    target: { x: 10, y: 20, z: 0 },
  };
  const destination = rebasePaperCamera(camera, start, end);
  for (const t of [0, 0.1, 0.5, 0.9, 1]) {
    const point = interpolatePoint(start, end, t);
    const moved = interpolateCamera(camera, destination, t);
    for (const axis of ["x", "y", "z"] as const) {
      expect(point[axis] - moved.position[axis]).toBeCloseTo(
        start[axis] - camera.position[axis],
      );
    }
  }
});
