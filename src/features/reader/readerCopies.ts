export interface ReaderCopy {
  documentId: string;
  sourceRevision: string;
  pdf: Blob;
  savedAt: string;
}

const DATABASE_NAME = "graphite-reader-copies";
const STORE_NAME = "copies";

/** Store edited copies independently from source files and retrieval indexes. */
class ReaderCopyStore {
  private open(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(DATABASE_NAME, 1);
      request.onupgradeneeded = () => {
        request.result.createObjectStore(STORE_NAME, {
          keyPath: ["documentId", "sourceRevision"],
        });
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () =>
        reject(
          new Error(
            "Local PDF copies could not be opened. Check available disk space.",
          ),
        );
      request.onblocked = () =>
        reject(
          new Error(
            "Close other Graphite windows and retry opening the PDF copy.",
          ),
        );
    });
  }

  async load(
    documentId: string,
    sourceRevision: string,
  ): Promise<ReaderCopy | null> {
    const database = await this.open();
    try {
      return await new Promise((resolve, reject) => {
        const transaction = database.transaction(STORE_NAME, "readonly");
        const request = transaction
          .objectStore(STORE_NAME)
          .get([documentId, sourceRevision]);
        request.onsuccess = () =>
          resolve((request.result as ReaderCopy | undefined) ?? null);
        request.onerror = () =>
          reject(new Error("The saved PDF copy could not be read."));
      });
    } finally {
      database.close();
    }
  }

  async save(
    documentId: string,
    sourceRevision: string,
    bytes: Uint8Array<ArrayBuffer>,
  ): Promise<void> {
    const database = await this.open();
    try {
      await new Promise<void>((resolve, reject) => {
        const transaction = database.transaction(STORE_NAME, "readwrite");
        transaction.objectStore(STORE_NAME).put({
          documentId,
          sourceRevision,
          pdf: new Blob([bytes], { type: "application/pdf" }),
          savedAt: new Date().toISOString(),
        } satisfies ReaderCopy);
        transaction.oncomplete = () => resolve();
        transaction.onabort = () =>
          reject(
            new Error(
              "The PDF copy could not be saved locally. Export it to keep your changes, then check available disk space.",
            ),
          );
      });
    } finally {
      database.close();
    }
  }
}

export const readerCopies = new ReaderCopyStore();
