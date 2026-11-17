import manifest from "../../../demo/manifest.json";
import type { ResearchGraph } from "../../domain/graph";
import type { LibrarySnapshot, ResearchItem } from "../../domain/research";

export const demoManifest = manifest;
const libraryId = "graphite-demo:seismology-ml-v1";
const names = [
  ...new Set(manifest.papers.flatMap((paper) => paper.themes)),
].sort((a, b) => a.localeCompare(b, "en"));
const topics: LibrarySnapshot["topics"] = [];

export const demoCollections = [
  {
    id: `${libraryId}:root`,
    libraryId,
    parentId: null,
    name: "Machine learning in seismology",
    description: manifest.selectionNote,
  },
  ...names.map((name) => ({
    id: `${libraryId}:theme:${name}`,
    libraryId,
    parentId: `${libraryId}:root`,
    name,
    description: "Curated subject theme",
  })),
];

export const demoItems: ResearchItem[] = manifest.papers.map((paper) => ({
  id: paper.id,
  title: paper.title,
  authors: paper.authors.map((name) => ({
    id: name,
    name,
    creatorType: "author",
  })),
  year: paper.year,
  date: String(paper.year),
  venue: paper.journal,
  abstractText: null,
  topics: [],
  methods: [],
  tags: paper.themes,
  collectionIds: [
    `${libraryId}:root`,
    ...paper.themes.map((theme) => `${libraryId}:theme:${theme}`),
  ],
  citationCount: 0,
  importance: 1,
  attachments: [
    {
      id: `${paper.id}:pdf`,
      documentId: `${paper.id}:pdf`,
      mediaType: "application/pdf",
      pageCount: paper.pdfPages,
      availability: "metadata-only",
      title: paper.title,
      filename: `${paper.id}.pdf`,
      linkMode: null,
      sourceVersion: 1,
      text: null,
    },
  ],
  notes: [],
  volume: null,
  issue: null,
  pages: null,
  publisher: null,
  doi: paper.doi,
  isbn: null,
  issn: null,
  url: paper.sourceUrl,
  provenance: {
    sourceId: "graphite-demo",
    libraryId,
    zoteroKey: paper.id,
    zoteroItemType: "journalArticle",
    sourceVersion: 1,
    dateAdded: null,
    dateModified: null,
  },
}));

export const demoSnapshot: LibrarySnapshot = {
  items: demoItems,
  collections: demoCollections,
  topics,
  lenses: [],
  libraries: [
    {
      id: libraryId,
      name: `${demoItems.length} real seismology papers`,
      kind: "demo",
      itemCount: demoItems.length,
      lastSyncedAt: null,
    },
  ],
  activeLibraryId: libraryId,
  provenance: "real-demo",
  totalItemCount: demoItems.length,
  loadedItemCount: demoItems.length,
};

export const demoGraph: ResearchGraph = {
  nodes: demoItems.map((item) => ({
    id: item.id,
    paperId: item.id,
    entityKind: "research-item",
    label: item.title,
    detail: `${item.authors[0]?.name} / ${item.year}`,
    topicId: "uncategorized",
    x: 0,
    y: 0,
    importance: 1,
  })),
  edges: demoItems.flatMap((source, index) =>
    demoItems.slice(index + 1).flatMap((target) => {
      const shared = source.tags.filter((tag) => target.tags.includes(tag));
      return shared.length
        ? [
            {
              id: `demo-tag:${source.id}:${target.id}`,
              source: source.id,
              target: target.id,
              relation: "shared-tag" as const,
              weight: shared.length,
              directed: false,
              evidenceCount: shared.length,
              evidenceLabel: `Shared collection themes: ${shared.join(", ")}`,
            },
          ]
        : [];
    }),
  ),
  availableRelations: ["shared-tag"],
  totalNodeCount: demoItems.length,
  hasMore: false,
};
