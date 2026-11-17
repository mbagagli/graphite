import type { GraphRelationType } from "../../domain/graph";

export const edgeColors: Record<
  GraphRelationType,
  readonly [number, number, number]
> = {
  citation: [94, 202, 206],
  "semantically-similar": [220, 166, 90],
  "bibliographic-coupling": [211, 158, 92],
  "co-cited": [164, 131, 219],
  "shared-creator": [211, 158, 92],
  "shared-collection": [94, 202, 206],
  "shared-tag": [103, 166, 151],
  "zotero-related": [126, 137, 216],
};

export function strengthenNodeColor(color: string): string {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color);
  if (!match) {
    return color;
  }

  const channels = match
    .slice(1)
    .map((channel) => Math.round(Number.parseInt(channel, 16) * 0.72));
  return `rgb(${channels.join(", ")})`;
}
