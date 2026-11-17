# Saved research Lenses

A Lens is a named, fixed set of papers for an investigation, with an optional
research question. Graphite stores it locally for the library where it was
created. It does not create or modify Zotero collections.

## Create and reopen

1. Open **Search library**. Check papers to collect them without closing
   search. Try a second query: the first papers stay selected. Opening a
   result's title selects that paper alone and centers it in the graph,
   just like a node click. Use checkboxes to keep a multiple-paper selection.
2. Choose **Review selection** in search, or **N selected** in the top bar.
   The panel lists full titles, first authors, and years, including papers
   outside the current graph. Remove individual papers or clear the list;
   **Undo** restores the last removal or clear until another paper is added.
3. Choose **Save as Lens**, enter a name and optionally a research question,
   then choose **Save Lens**. You can also use **Save selection as Lens** in
   the sidebar; this permits an empty Lens for later work.
4. Quit and reopen Graphite. Choose the same data source and library, then
   select the Lens in the sidebar to reopen its papers.

The working selection lasts for this session and library. Search queries,
filters, result pages, graph views, and Reader navigation keep it. Back
restores the selection saved before the corresponding navigation step.
Switching libraries clears it and its Undo history; save a Lens first to
keep the set. A graph click still selects that paper; Shift/Command-click
adds or removes papers. Use **Add to selection** in a paper's details to
collect a focused search result. Clicking empty graph space clears selection
and focus without moving the camera. Clear selection does the same and closes
the selection panel. Reopen the panel to Undo the last removal or clear:
Undo restores only the selected papers, not focus or camera. Closing paper
details alone dismisses focus without clearing the selection.

Checkboxes work with Tab and Space. In the selection panel, each title opens
its paper and each named Remove control edits membership. Escape closes the
panel and returns focus to the selected-count button.

To keep TYRION's evidence, choose **Temporary Lens** beneath a completed
answer, then **Save temporary Lens** in the sidebar. This includes papers
supplied to the answer, which may include uncited sources. Temporary Lenses
are not persisted until saved.

A Lens is a snapshot of membership, not a saved search. New search results or
model updates do not add papers automatically. Graph layout and camera state
are not saved with the Lens. Saving or deleting a Lens resets graph navigation history
to avoid reopening an obsolete membership view. A Lens can contain at most
1200 papers, and its graph opens with every available member.

## Edit membership

Choose **Edit** beside a saved Lens to rename it, change its question, or
remove individual papers. Choose **Add papers** to search the active library
without leaving the editor:

1. Search by title, author, tag, or indexed text. This uses local keyword
   search, so adding papers does not require the semantic model.
2. Choose **Add** beside a result. Existing draft members say **Already
   included**; new additions say **Added to draft** and cannot be duplicated.
   Search again or use **Next results** to find more papers.
3. Choose **Done adding** or **Back to Lens** to review the draft. The name,
   question, prior removals, and all additions are retained. The editor shows
   the number of papers added and removed before you save.
4. Choose **Save Lens** to apply the complete draft, or **Cancel** to discard
   it. Searching and adding papers never writes an intermediate Lens.

Escape in the picker returns to the Lens draft; Enter in its search box does
not save. A search error offers Retry search and leaves the draft intact.
The editor's search does not change the workspace's separate paper selection.

If papers are already selected in the workspace, **Add selected papers** is
also available. Its count includes only papers absent from the draft, and it
is disabled when they are all included. Missing papers remain in membership
until you explicitly remove them.

The editor works on a draft. Cancel leaves the saved version unchanged.
Conflicting edits are rejected rather than overwriting newer changes; close
and reopen the editor to load the latest saved version.

**Delete Lens** opens a confirmation view in the editor. Choose **Confirm
delete Lens** to remove it, or **Keep Lens** (or Escape) to return to the
unchanged draft. A failed deletion stays visible and can be retried. Deleting
a Lens keeps all papers, original PDFs, Zotero records, and other Lenses.
Deleted Lenses do not return after restarting Graphite.

## Ask within a Lens

Open a Lens and choose **Ask about this Lens**. TYRION opens with the research
question, if one was saved, and **This Lens only** selected. Review the
question before sending it. You can also choose this scope in TYRION's
**Answer scope** controls.

Lens-only requests use 1-64 available papers. Larger Lenses remain useful
for exploration; select a smaller set and use **Selected papers only**, or
save a smaller Lens. No papers are silently truncated for an explicit
Lens-only request. Earlier conversation answers are excluded from its
context. Reading, indexing, and model requirements still apply.

Check every answer citation against the supplied passage and original PDF.
**Open in Reader** collapses TYRION so the cited page is visible. Reopen
TYRION to return to the same answer and source details.
The Lens limits eligible sources; it does not guarantee complete retrieval
or a scientifically correct answer.

## Missing papers and library changes

Lenses belong to one library. Switching between demo, Zotero, or another
Zotero library shows that library's own Lenses. Switch back to recover the
original list.

Missing or trashed papers remain recorded in membership and are labeled
unavailable. They do not contribute evidence. You can keep them for later
resynchronization or explicitly remove them. An empty or entirely unavailable
Lens cannot fall back to answering from the whole library.

Lens definitions are user-authored state, separate from disposable indexes.
They survive cache rebuilds; membership can temporarily become unavailable
until the source is synchronized again. Keep Graphite's database in your
backups. Browser metadata previews do not save Lenses; use Graphite Desktop.

## Development checks

The native regression test launches a fresh process after saving five
papers and a question and verifies the restored workspace. It then checks editing, revision conflicts, library
isolation, missing papers, deletion and source-cache removal. A second fresh
process verifies that deleting a populated Lens preserves another Lens and
the papers while removing the deleted membership. Grounding tests
check Lens-only scope and failure cases. `npm run qa:lenses` runs the actual
interface with isolated command stubs; it does not claim native persistence
or model generation. Run it against `npm run dev`.
