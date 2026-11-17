# Reading and editing PDFs

Open a paper in Reader. Page arrows and keyboard navigation change pages
without reopening the document. Enter a page number and press Enter, or
leave the field, to navigate. Scrolling moves within the page; a continuous
wheel gesture can turn at most one page at a boundary. Thumbnails render
independently, and a finished page replaces the previous canvas atomically.

## Edit a copy

Choose **Edit PDF** in the reader toolbar. All operations work on a separate
copy; Graphite does not modify Zotero attachments or downloaded source PDFs.

- **Select** selects text for copying.
- **Highlight** marks selected text. Drag across the words to highlight.
- **Note** places a note where you click. Edit or remove it in the side panel.
- **Draw** adds a freehand mark. The side panel can remove it.
- **Edit text** makes existing text runs selectable for replacement or
  deletion. Enter replacement text and apply the change.
- **Pages** offers rotation, moving a page to another position, inserting a
  blank page, inserting another PDF, and deleting a page. Deletion asks for
  confirmation; a PDF must retain at least one page.

Undo and Redo cover document changes for the current editing session.
Keyboard shortcuts are Ctrl/Command+Z and Ctrl/Command+Shift+Z. History is
bounded to 12 revisions and approximately 128 MB, retaining the latest
revision even when it alone reaches that budget.

**Save copy** stores the edited PDF locally in Graphite. Reopening the paper
shows that saved copy. **View original** and **View edited copy** switch
between versions. **Export PDF** saves a separate, interoperable PDF using
the desktop save dialog or the browser download action. Export does not
mark the Graphite copy as saved. Closing with unsaved changes offers Save,
Discard, or Keep editing.

## Text and page limitations

Text editing replaces actual PDF text objects; it does not cover old text
with a white rectangle. A text run is one selectable fragment in the PDF,
which may be shorter than a sentence. Nearby text does not reflow. Review
longer replacements for overlap before exporting.

The original embedded font is used when it can encode the replacement
exactly. If it cannot, the editor keeps the PDF unchanged and offers the
explicit **Noto Sans** alternative. Choosing that font may change the
appearance. The bundled font does not contain every writing system.

Scanned images and text nested inside grouped artwork are not editable text
runs. OCR or an external PDF editor is needed for those cases. The editor
accepts PDFs up to 128 MB and requires an unencrypted copy. It does not
provide general layout, figure, form, signature, or OCR editing. Editing a
signed document changes its content and does not preserve signature validity.

Annotations are stored as PDF highlights, text notes, and ink marks so other
PDF applications can read them. Existing links are preserved and are not
listed as removable review notes.

## Search, citations, and storage

Library search, indexed text, semantic retrieval, and assistant citations
continue to use the original document. Citation navigation opens the
original page. When viewing an edited copy, the assistant receives the
original document context without a potentially shifted page number.
Reordering or deleting copied pages therefore does not change indexed
citation coordinates.

Copies are stored in Graphite's local application storage, keyed by document
and original revision. A changed original does not silently reuse a copy
made from an earlier version. Export important edits before clearing app
storage or uninstalling. A copy-storage error is shown without preventing
reading the original. PDF bytes stay on the device during editing.

## Implementation and verification

Rendering uses PDF.js. Editing runs PDFium in a local Web Worker, loaded only
when the editor is opened. The locked `@embedpdf/pdfium` package supplies the
editing engine; bundled Noto Sans supplies the optional replacement font.
Their license notices are included in desktop packages.

Unit checks independently reopen edited output with PDF.js to verify true
text replacement, source preservation, page operations, and annotation
interoperability. The focused browser check uses a checksum-verified public
paper from the demo manifest:

```sh
node scripts/pdf-editor-smoke.mjs <verified-demo-pdf-directory>
```

Run against `npm run dev` with Firefox and Geckodriver available. Its isolated
profile checks stable page rendering, multi-digit navigation, wheel gestures,
text selection, highlighting, drawing, text edits, undo/redo, page operations,
notes, saved-copy reopening, and the minimum desktop viewport. Evidence is
written under the ignored `artifacts/qa/pdf-editor` directory.
