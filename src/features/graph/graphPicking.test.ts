import { expect, it } from "vitest";
import { createNodeRaycast, nodePickDistance } from "./graphPicking";

const camera = {
  fov: 90,
  zoom: 1,
  matrixWorldInverse: {
    elements: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
  },
};
const ray = {
  origin: { x: 0, y: 0, z: 0 },
  direction: { x: 0, y: 0, z: -1 },
};

it.each([100, 1000, 10000])(
  "keeps distant papers reachable within 12 CSS pixels at depth %i",
  (depth) => {
    const unitsPerPixel = (2 * depth) / 1000;
    expect(
      nodePickDistance(
        { x: 11 * unitsPerPixel, y: 0, z: -depth },
        1,
        ray,
        camera,
        1000,
      ),
    ).not.toBeNull();
    expect(
      nodePickDistance(
        { x: 13 * unitsPerPixel, y: 0, z: -depth },
        1,
        ray,
        camera,
        1000,
      ),
    ).toBeNull();
  },
);

it("keeps a large visible node clickable to its edge with a small margin", () => {
  expect(
    nodePickDistance({ x: 10, y: 0, z: -100 }, 10, ray, camera, 1000),
  ).not.toBeNull();
  expect(
    nodePickDistance({ x: 11, y: 0, z: -100 }, 10, ray, camera, 1000),
  ).toBeNull();
});

it("accounts for camera zoom, viewport resizing and nodes behind the viewer", () => {
  const point = { x: 18, y: 0, z: -1000 };
  expect(nodePickDistance(point, 1, ray, camera, 1000)).not.toBeNull();
  expect(
    nodePickDistance(point, 1, ray, { ...camera, zoom: 2 }, 1000),
  ).toBeNull();
  expect(nodePickDistance(point, 1, ray, camera, 2000)).toBeNull();
  expect(
    nodePickDistance({ x: 0, y: 0, z: 1 }, 1, ray, camera, 1000),
  ).toBeNull();
  expect(nodePickDistance(point, 1, ray, camera, 0)).toBeNull();
});

it("ranks overlapping targets by the visible front, not their hit padding", () => {
  const front = nodePickDistance(
    { x: 20, y: 0, z: -1000 },
    1,
    ray,
    camera,
    1000,
  )!;
  const back = nodePickDistance(
    { x: 0, y: 0, z: -1010 },
    1,
    ray,
    camera,
    1000,
  )!;
  expect(front).toBeLessThan(back);
  expect(
    nodePickDistance({ x: 0, y: 0, z: -1000 }, 20, ray, camera, 1000),
  ).toBe(980);
});

it("uses camera space after rotating the view", () => {
  const rotated = {
    ...camera,
    matrixWorldInverse: {
      elements: [0, 0, 1, 0, 0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 0, 1],
    },
  };
  expect(
    nodePickDistance(
      { x: -1000, y: 18, z: 0 },
      1,
      { ...ray, direction: { x: -1, y: 0, z: 0 } },
      rotated,
      1000,
    ),
  ).not.toBeNull();
});

it("returns the original node to the renderer and honors clipping", () => {
  const node = {
    matrixWorld: { elements: [...camera.matrixWorldInverse.elements] },
    geometry: { parameters: { radius: 1 } },
    raycast: createNodeRaycast(() => 1000),
  };
  node.matrixWorld.elements[14] = -1000;
  const vector = { ...ray.origin, clone: () => ({ ...vector }) };
  const raycaster = {
    camera,
    near: 0,
    far: 2000,
    ray: {
      ...ray,
      origin: vector,
      at: (distance: number) => ({ ...vector, z: -distance }),
    },
  };
  const hits: Parameters<typeof node.raycast>[1] = [];
  node.raycast(raycaster, hits);
  expect(hits).toHaveLength(1);
  expect(hits[0].object).toBe(node);
  expect(hits[0].distance).toBe(999);
  node.raycast({ ...raycaster, far: 900 }, hits);
  node.raycast({ ...raycaster, near: 1100 }, hits);
  expect(hits).toHaveLength(1);
});
