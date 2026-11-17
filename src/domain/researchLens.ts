import type { ResearchLens } from "./research";

export const MAX_LENS_PAPERS = 1200;

/** Compare draft membership with the saved set, ignoring duplicate IDs. */
export function lensMembershipChanges(original: string[], draft: string[]) {
  const before = new Set(original);
  const after = new Set(draft);
  return {
    added: [...after].filter((id) => !before.has(id)),
    removed: [...before].filter((id) => !after.has(id)),
  };
}

/** Build an unsaved source-paper snapshot, preserving the full question. */
export function createEvidenceLens(
  id: string,
  question: string,
  paperIds: string[],
): ResearchLens {
  return {
    id,
    name: question.length > 54 ? `${question.slice(0, 51)}...` : question,
    description: question,
    paperIds: [...new Set(paperIds)],
    groups: ["Agent evidence"],
    saved: false,
  };
}
