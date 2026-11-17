import type { ResearchItem } from "../../domain/research";

export interface PublicationYearGroup {
  year: number | null;
  papers: ResearchItem[];
}

/** Group the current view by year, with reproducible title/id ordering. */
export function groupPapersByYear(
  items: ResearchItem[],
): PublicationYearGroup[] {
  const groups = new Map<number | null, ResearchItem[]>();
  for (const paper of items) {
    const year = Number.isInteger(paper.year) ? paper.year : null;
    const group = groups.get(year);
    if (group) group.push(paper);
    else groups.set(year, [paper]);
  }
  return [...groups.entries()]
    .sort(([left], [right]) => {
      if (left === null) return 1;
      if (right === null) return -1;
      return left - right;
    })
    .map(([year, papers]) => ({
      year,
      papers: papers.sort(
        (left, right) =>
          left.title.localeCompare(right.title, "en", {
            sensitivity: "base",
          }) || left.id.localeCompare(right.id, "en"),
      ),
    }));
}
