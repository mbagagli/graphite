import type { GraphPosition } from "./graphLayout";

// CSS pixels keep distant papers reachable without changing their appearance.
export const MIN_NODE_PICK_RADIUS_PX = 12;
const NODE_PICK_MARGIN_PX = 3;

interface PickCamera {
  fov: number;
  zoom: number;
  matrixWorldInverse: { elements: ArrayLike<number> };
}

interface PickRay {
  origin: GraphPosition;
  direction: GraphPosition;
}

export function nodePickDistance(
  center: GraphPosition,
  radius: number,
  ray: PickRay,
  camera: PickCamera,
  viewportHeight: number,
): number | null {
  const matrix = camera.matrixWorldInverse.elements;
  const depth = -(
    matrix[2] * center.x +
    matrix[6] * center.y +
    matrix[10] * center.z +
    matrix[14]
  );
  if (depth <= 0 || viewportHeight <= 0) return null;
  const unitsPerPixel =
    (2 * depth * Math.tan((camera.fov * Math.PI) / 360)) /
    (viewportHeight * camera.zoom);
  const hitRadius = Math.max(
    radius + NODE_PICK_MARGIN_PX * unitsPerPixel,
    MIN_NODE_PICK_RADIUS_PX * unitsPerPixel,
  );
  const delta = {
    x: center.x - ray.origin.x,
    y: center.y - ray.origin.y,
    z: center.z - ray.origin.z,
  };
  const alongRay =
    delta.x * ray.direction.x +
    delta.y * ray.direction.y +
    delta.z * ray.direction.z;
  if (alongRay <= 0) return null;
  const distanceSquared =
    (delta.x - alongRay * ray.direction.x) ** 2 +
    (delta.y - alongRay * ray.direction.y) ** 2 +
    (delta.z - alongRay * ray.direction.z) ** 2;
  if (distanceSquared > hitRadius ** 2) return null;
  // Rank by the visible sphere's front, never by its enlarged target.
  return Math.max(
    0,
    alongRay - Math.sqrt(Math.max(0, radius ** 2 - distanceSquared)),
  );
}

interface PickVector extends GraphPosition {
  clone(): PickVector;
}

interface PickNode {
  matrixWorld: { elements: ArrayLike<number> };
  geometry: { parameters: { radius: number } };
}

interface NodeRaycaster {
  camera: PickCamera;
  near: number;
  far: number;
  ray: PickRay & {
    origin: PickVector;
    at(distance: number, target: PickVector): PickVector;
  };
}

interface NodeIntersection {
  distance: number;
  point: PickVector;
  object: PickNode;
}

export function createNodeRaycast(viewportHeight: () => number) {
  return function (
    this: PickNode,
    raycaster: NodeRaycaster,
    intersections: NodeIntersection[],
  ): void {
    const matrix = this.matrixWorld.elements;
    const distance = nodePickDistance(
      { x: matrix[12], y: matrix[13], z: matrix[14] },
      this.geometry.parameters.radius,
      raycaster.ray,
      raycaster.camera,
      viewportHeight(),
    );
    if (
      distance === null ||
      distance < raycaster.near ||
      distance > raycaster.far
    )
      return;
    intersections.push({
      distance,
      point: raycaster.ray.at(distance, raycaster.ray.origin.clone()),
      object: this,
    });
  };
}
