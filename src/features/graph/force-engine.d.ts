// Declare the used subset of the untyped, existing layout engine.
declare module "d3-force-3d" {
  interface Node {
    id: string;
    x: number;
    y: number;
    z: number;
  }
  interface Force {
    (alpha: number): void;
    strength(value: number): Force;
    distance(value: number): Force;
    iterations(value: number): Force;
    id(accessor: (node: Node) => string): Force;
  }
  export function forceSimulation(
    nodes: Node[],
    dimensions: number,
  ): {
    stop(): ReturnType<typeof forceSimulation>;
    force(name: string, force: Force): ReturnType<typeof forceSimulation>;
    tick(iterations: number): ReturnType<typeof forceSimulation>;
  };
  export function forceLink(links: { source: string; target: string }[]): Force;
  export function forceManyBody(): Force;
  export function forceCenter(x?: number, y?: number, z?: number): Force;
  export function forceCollide(radius: number): Force;
}
