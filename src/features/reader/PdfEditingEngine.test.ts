/// <reference types="node" />
import { readFile } from "node:fs/promises";
import { beforeAll, expect, it } from "vitest";
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";
import { PdfEditingEngine } from "./PdfEditingEngine";

let engine: PdfEditingEngine;

beforeAll(async () => {
  const wasm = await readFile(
    new URL(
      "../../../node_modules/@embedpdf/pdfium/dist/pdfium.wasm",
      import.meta.url,
    ),
  );
  const font = await readFile(
    new URL("../../assets/fonts/NotoSans-Regular.ttf", import.meta.url),
  );
  engine = await PdfEditingEngine.create(
    new Uint8Array(wasm).buffer,
    new Uint8Array(font),
  );
});

/** Build a deterministic three-page document with selectable text. */
function documentBytes(withLink = false) {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R] /Count 3 >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  for (const [index, label] of [
    "First page",
    "Second page",
    "Third page",
  ].entries()) {
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 600] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + index * 2} 0 R ${withLink && index === 0 ? "/Annots [10 0 R]" : ""} >>`,
    );
    const content = `BT /F1 12 Tf 1 0 0 1 40 550 Tm (${label}) Tj ET`;
    objects.push(
      `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    );
  }
  if (withLink) {
    objects.push(
      "<< /Type /Annot /Subtype /Link /Rect [40 546 96 562] /A << /S /URI /URI (https://example.org/paper) >> >>",
    );
  }
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  for (let i = 0; i < objects.length; i++) {
    offsets.push(pdf.length);
    pdf += `${i + 1} 0 obj\n${objects[i]}\nendobj\n`;
  }
  const xref = pdf.length;
  pdf += `xref\n0 ${offsets.length}\n0000000000 65535 f \n`;
  pdf += offsets
    .slice(1)
    .map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`)
    .join("");
  pdf += `trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return new TextEncoder().encode(pdf);
}

async function readPages(bytes: Uint8Array<ArrayBuffer>) {
  const task = getDocument({ data: bytes.slice(), useSystemFonts: true });
  const pdf = await task.promise;
  try {
    const pages: {
      text: string;
      rotation: number;
      width: number;
      height: number;
    }[] = [];
    for (let index = 1; index <= pdf.numPages; index++) {
      const page = await pdf.getPage(index);
      const text = await page.getTextContent();
      const viewport = page.getViewport({ scale: 1 });
      pages.push({
        text: text.items
          .map((item) => ("str" in item ? item.str : ""))
          .join(""),
        rotation: page.rotate,
        width: viewport.width,
        height: viewport.height,
      });
    }
    return pages;
  } finally {
    await task.destroy();
  }
}

it("replaces actual text and keeps the source byte-for-byte unchanged", async () => {
  const source = documentBytes();
  const before = source.slice();
  const original = engine.inspect(source, 0).text[0];
  const edited = engine.edit(source, {
    kind: "text",
    page: 0,
    object: original.index,
    expectedText: original.text,
    text: "Revised paper",
    font: "original",
  });
  const pages = await readPages(edited);
  expect(pages.map((page) => page.text)).toEqual([
    "Revised paper",
    "Second page",
    "Third page",
  ]);
  expect(source).toEqual(before);
  expect(new TextDecoder().decode(edited)).not.toContain("First page");
});

it("embeds the explicitly chosen font and round-trips non-ASCII text", async () => {
  const source = documentBytes();
  const original = engine.inspect(source, 0).text[0];
  const edited = engine.edit(source, {
    kind: "text",
    page: 0,
    object: original.index,
    expectedText: original.text,
    text: "Wave speed \\u03b1 = 3.5 km/s".replace("\\u03b1", "\u03b1"),
    font: "noto",
  });
  expect((await readPages(edited))[0].text).toBe(
    "Wave speed \u03b1 = 3.5 km/s",
  );
  expect(engine.inspect(edited, 0).text[0].font).toContain("NotoSans");
});

it("rotates, reorders, inserts, and deletes real PDF pages", async () => {
  let pdf = documentBytes();
  pdf = engine.edit(pdf, { kind: "rotate", page: 0, turns: 1 });
  let pages = await readPages(pdf);
  expect(pages[0]).toMatchObject({ rotation: 90, width: 600, height: 400 });
  pdf = engine.edit(pdf, { kind: "move-page", page: 0, destination: 2 });
  expect((await readPages(pdf)).map((page) => page.text)).toEqual([
    "Second page",
    "Third page",
    "First page",
  ]);
  pdf = engine.edit(pdf, { kind: "insert-blank", page: 0 });
  pages = await readPages(pdf);
  expect(pages.map((page) => page.text)).toEqual([
    "Second page",
    "",
    "Third page",
    "First page",
  ]);
  pdf = engine.edit(pdf, { kind: "delete-page", page: 1 });
  pdf = engine.edit(pdf, {
    kind: "insert-pdf",
    page: 0,
    bytes: documentBytes(),
  });
  expect((await readPages(pdf)).map((page) => page.text)).toEqual([
    "Second page",
    "First page",
    "Second page",
    "Third page",
    "Third page",
    "First page",
  ]);
});

it("saves interoperable highlights, notes, and drawings", async () => {
  let pdf = documentBytes();
  pdf = engine.edit(pdf, {
    kind: "highlight",
    page: 0,
    rects: [{ left: 40, bottom: 546, right: 96, top: 562 }],
  });
  pdf = engine.edit(pdf, {
    kind: "note",
    page: 0,
    point: { x: 150, y: 550 },
    text: "Check this estimate",
  });
  pdf = engine.edit(pdf, {
    kind: "drawing",
    page: 0,
    points: [
      { x: 40, y: 500 },
      { x: 60, y: 495 },
      { x: 80, y: 505 },
    ],
  });
  const task = getDocument({ data: pdf.slice() });
  const document = await task.promise;
  try {
    const annotations = await (await document.getPage(1)).getAnnotations();
    expect(annotations.map((a) => a.subtype)).toEqual([
      "Highlight",
      "Text",
      "Ink",
    ]);
    expect(annotations[1].contentsObj.str).toBe("Check this estimate");
  } finally {
    await task.destroy();
  }
  const note = engine
    .inspect(pdf, 0)
    .annotations.find((a) => a.kind === "note")!;
  pdf = engine.edit(pdf, {
    kind: "edit-note",
    page: 0,
    annotation: note.index,
    text: "Reviewed",
  });
  expect(engine.inspect(pdf, 0).annotations[note.index].text).toBe("Reviewed");
  pdf = engine.edit(pdf, {
    kind: "remove-annotation",
    page: 0,
    annotation: note.index,
  });
  expect(engine.inspect(pdf, 0).annotations.map((a) => a.kind)).toEqual([
    "highlight",
    "drawing",
  ]);
});

it("rejects stale selections, invalid coordinates and deleting the last page", () => {
  let source = documentBytes();
  const before = source.slice();
  expect(() =>
    engine.edit(source, {
      kind: "text",
      page: 0,
      object: 0,
      expectedText: "Wrong selection",
      text: "new",
      font: "original",
    }),
  ).toThrow("changed");
  expect(source).toEqual(before);
  expect(() =>
    engine.edit(source, {
      kind: "note",
      page: 0,
      point: { x: NaN, y: 0 },
      text: "Note",
    }),
  ).toThrow("finite");
  expect(() => engine.inspect(source, 100)).toThrow("Choose a page");
  source = engine.edit(source, { kind: "delete-page", page: 2 });
  source = engine.edit(source, { kind: "delete-page", page: 1 });
  expect(() => engine.edit(source, { kind: "delete-page", page: 0 })).toThrow(
    "at least one",
  );
});

it("preserves PDF links without listing them as removable review notes", async () => {
  const source = documentBytes(true);
  expect(engine.inspect(source, 0).annotations).toEqual([]);
  let pdf = engine.edit(source, {
    kind: "note",
    page: 0,
    point: { x: 150, y: 550 },
    text: "Review this",
  });
  const notes = engine.inspect(pdf, 0).annotations;
  expect(notes).toHaveLength(1);
  expect(notes[0]).toMatchObject({ index: 1, kind: "note" });
  pdf = engine.edit(pdf, {
    kind: "remove-annotation",
    page: 0,
    annotation: notes[0].index,
  });
  const task = getDocument({ data: pdf.slice() });
  const document = await task.promise;
  try {
    const annotations = await (await document.getPage(1)).getAnnotations();
    expect(annotations).toHaveLength(1);
    expect(annotations[0]).toMatchObject({
      subtype: "Link",
      url: "https://example.org/paper",
    });
  } finally {
    await task.destroy();
  }
});
