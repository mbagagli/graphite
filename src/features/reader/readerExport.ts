import { invoke, isTauri } from "@tauri-apps/api/core";

export async function exportPdfCopy(
  bytes: Uint8Array<ArrayBuffer>,
  title: string,
) {
  const name = `${title.replace(/[^a-zA-Z0-9 _().-]/g, "_").slice(0, 100)}-edited.pdf`;
  if (isTauri()) {
    return invoke<boolean>("export_pdf_copy", bytes, {
      headers: { "x-graphite-file-name": name },
    });
  }
  const url = URL.createObjectURL(
    new Blob([bytes], { type: "application/pdf" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
  return true;
}
