import { invoke } from "@tauri-apps/api/core";

export interface DemoProgress {
  phase: string;
  completed: number;
  total: number;
  bytes: number;
  totalBytes: number;
  paperId: string | null;
}
export interface DemoStatus {
  pdfCount: number;
  totalPdfs: number;
  totalBytes: number;
  busy: boolean;
  progress: DemoProgress | null;
  documents: {
    paperId: string;
    documentId: string;
    pages: number;
    available: boolean;
    textIndexed: boolean;
  }[];
}

export const getDemoStatus = () => invoke<DemoStatus>("get_demo_status");
