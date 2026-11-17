export type TopicId = string;

export interface Author {
  id: string;
  name: string;
  creatorType: string;
}

export interface ResearchCollection {
  id: string;
  name: string;
  description: string;
  parentId: string | null;
  libraryId: string;
}

export interface Topic {
  analyzed?: number | null;
  eligible?: number | null;
  representatives?: string[];
  id: TopicId;
  name: string;
  shortName: string;
  color: string;
  origin:
    "demo-theme" | "zotero-tag-facet" | "semantic-cluster" | "uncategorized";
}

export interface DocumentAttachment {
  id: string;
  documentId: string;
  mediaType: string;
  pageCount: number | null;
  availability: "metadata-only" | "ready" | "unavailable" | "unsupported";
  title: string;
  filename: string | null;
  linkMode: string | null;
  sourceVersion: number;
  text: DocumentTextSummary | null;
}

export interface DocumentTextSummary {
  provider: "zotero-indexed" | "fixture";
  status: "full" | "partial";
  indexedPages: number | null;
  totalPages: number | null;
  indexedChars: number | null;
  totalChars: number | null;
  sourceVersion: number;
}

export interface ResearchNote {
  id: string;
  title: string;
  content: string;
}

export interface ItemProvenance {
  sourceId: string;
  libraryId: string;
  zoteroKey: string;
  zoteroItemType: string;
  sourceVersion: number;
  dateAdded: string | null;
  dateModified: string | null;
}

export interface ResearchItem {
  id: string;
  title: string;
  authors: Author[];
  year: number | null;
  date: string | null;
  venue: string | null;
  abstractText: string | null;
  topics: TopicId[];
  methods: string[];
  tags: string[];
  collectionIds: string[];
  citationCount: number;
  importance: number;
  attachments: DocumentAttachment[];
  notes: ResearchNote[];
  volume: string | null;
  issue: string | null;
  pages: string | null;
  publisher: string | null;
  doi: string | null;
  isbn: string | null;
  issn: string | null;
  url: string | null;
  provenance: ItemProvenance;
}

export interface ResearchLens {
  libraryId?: string;
  revision?: number;
  unavailablePaperIds?: string[];
  paperTitles?: Record<string, string>;
  id: string;
  name: string;
  description: string;
  paperIds: string[];
  groups: string[];
  saved: boolean;
}

export interface ResearchSelection {
  paperIds: string[];
  source: "graph" | "search" | "agent" | "inspector" | "citation";
}

export interface LibrarySnapshot {
  items: ResearchItem[];
  collections: ResearchCollection[];
  topics: Topic[];
  lenses: ResearchLens[];
  libraries: LibraryDescriptor[];
  activeLibraryId: string | null;
  provenance: "real-demo" | "zotero-local-cache";
  totalItemCount: number;
  loadedItemCount: number;
}

export interface LibraryDescriptor {
  id: string;
  name: string;
  kind: "user" | "group" | "demo";
  itemCount: number;
  lastSyncedAt: string | null;
}
