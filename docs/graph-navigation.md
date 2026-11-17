# Unified graph navigation

## Current decision

Every launch starts in 3D. GraphCanvas uses one renderer for the spatial
network and its readable, collapsed 2D plane. The 3D and 2D controls are
stacked vertically. There is no alternate rendering implementation.

## Continuity and motion

A worker prepares deterministic spatial and flat layouts from the same
bounded projection. Flat layout uses repulsion and collision spacing to
reduce overlap. Both layouts are cached; selection, themes and relationship layers do not
rearrange papers. Explicit grouping changes the layout as described below. Graph proximity is a navigation
aid, not a scientific similarity measure.

The camera approaches the first nonempty graph over 1.8 seconds with a gentle
arc, finishing at 45 percent of the fitted overview distance. Fit graph
returns to the complete overview. Collapsing or expanding takes 600 ms and moves the same nodes and edges.
The transition can reverse from its current positions. Reduced-motion users
receive immediate state changes.

The flat view locks rotation and supports pan/zoom; the expanded view supports
orbit/pan/zoom. Each mode retains its own camera during the session and through
Reader round-trips. Returning to the graph does not replay the arrival.

## Topic map and grouping

The topic legend starts closed. **TOPIC MAP** opens a compact popover with
topic names, colors, and counts for the visible graph. Its close button,
Escape, or a click outside dismisses it.

Fresh libraries use neutral nodes. Topic colors require
[semantic library preparation](library-preparation.md#automatic-topics).
Labels describe automatic similarity groups, with unassigned papers kept neutral.

**Group By Topic**, directly below, gathers papers by their discovered topic
with an animated transition. Each group retains its internal layout; topic
placement does not create relationships or change scientific evidence.
Click again to return to the relationship layout. Grouping is disabled when
no useful map exists or fewer than two groups are present. Reduced motion applies the change
immediately. Exploration history preserves the grouping choice.

## Resource management and recovery

Only one graph canvas is active. Leaving the graph disposes the renderer and
any pending layout worker, while retaining layout and camera state. A graphics
failure offers Retry graph, preserving access to library search and Reader.
Retry recreates the same renderer, without a second rendering path.

## Large libraries

The default canvas contains every paper in the active library, including
papers without indexed relationships. All papers restores this complete view.
Collections show all their papers, and Lenses show all available members.
There is no Load a larger projection button. Layout preparation runs off the
UI thread; larger libraries take more time and memory. Focused citation and
similarity neighborhoods still use their existing evidence limits.

## Navigation feel and node appearance

Pointer navigation has no inertial damping. Keyboard and button camera
actions take 140 ms; collapse and expansion retain their 600 ms transitions.
Nodes use opaque, smoothly shaded 32-segment spheres with directional and
ambient lighting, preserving category and selection colors.

## Selection and navigation help

The selected paper's complete title and author list appear in an opaque,
full-contrast panel above the graph nodes and edges. Long author lists remain
available by scrolling the panel. Additional selections retain their count.

The info button immediately below 2D opens navigation instructions for the
current mode. Instructions are no longer permanently overlaid on the graph.
The nonmodal help box closes with Escape, an outside click or its close button.

A single click recenters a paper without zooming. Double-click or Focus
selection dives moderately, with a minimum distance to avoid oversized
closeups. Both place the paper at 35 percent of the canvas width, leaving
space for the right inspector. Hovering previews details above Tyron
instead of a floating tooltip; leaving restores the selection. Titles are
bold with the known year in brackets, and author names are italic.

Projection changes such as Similar preserve the active paper's screen
position and camera scale when that paper remains in the new graph.
Search library in the top bar is the single paper-search entry point.
The top-bar sun/moon button switches themes directly; the unused settings
popover and placeholder researcher initials have been removed. No user
account is required.

Similar keeps the live graph visible while local retrieval and layout
preparation run. Matching papers gather into their new neighborhood over
900 ms, with connected edges following them and the selected paper staying
anchored. New papers enter from the previous camera target; papers outside
the neighborhood leave when the result is installed. Reduced motion applies
the result immediately. Direct navigation completes the aggregation first.

## Scientific exploration

Select papers, choose Find similar or Explore citations, inspect connections,
and use Back to return. The latest selected paper is the active camera anchor.
Removing it makes the most recently selected remaining paper active.

The library overview enables citation connections. Shared authors, tags,
collections and Zotero-related links are off initially. Show connections groups
these overlays separately from citations and content similarity. Visibility
changes never retrieve papers, rearrange nodes or move the camera. Hidden types
remain hidden around selected papers. Selection connections only hides unrelated
lines; the default shows all enabled lines and emphasizes selected connections.

Layouts use citation, shared-reference, co-citation and available similarity
edges. Organizational metadata never supplies layout attraction. Disconnected
papers receive deterministic spacing, not inferred scientific clusters. Sparse
views may reflect unavailable indexing or incomplete local citation coverage.

The scope button beside Find similar opens a styled popover with radio
choices, matching Show connections. Choose the whole library or one
collection; Escape or an outside click dismisses the popover.

Find similar searches the whole library unless an individual collection is
chosen beside the action. The menu lists every collection in the active library,
including when the full library is displayed. Choosing a scope changes only the
next Similar request; it does not navigate the graph by itself. Multiple selected papers use their combined
title/abstract representation. Citation exploration includes connections to any
selected paper, retains all selected anchors, deduplicates evidence and identifies
which seeds connect to each result. Shared-reference and co-citation definitions
remain unchanged. Results are bounded; coverage notices identify seeds without
usable data. If none are usable, the existing view remains intact.

Click a line for Why are these connected? Bibliographic text, shared works or
actual matching metadata explain the link. Similarity scores live in expandable
details and do not represent certainty or scientific agreement. External cited
works are identified separately from local papers.

## Search and Back

Check search results to collect papers across queries without closing search.
The top-bar selected count opens a list with Remove, Clear, Undo, and Save as
Lens. Opening a search result focuses it without changing that list. Back and
Reader navigation preserve collected papers. The working selection lasts
until you clear it, replace it with a graph selection, switch libraries, or
restart. Save a Lens for later sessions. See the [Lens guide](lenses.md).

Typing in Search library changes only its results. Opening a paper commits
navigation after its projection loads. Show results on graph explicitly maps the
loaded result set; Load next results remains available inside search. Canceling
search preserves the graph, selection and camera.

Back stores up to 20 successful explorations during the current session,
including collections, lenses, paper searches and scientific neighborhoods. It
restores the projection, active paper, scope, connection settings,
settled layouts, mode and both cameras. Hovering, selection changes, navigation,
visibility toggles and failed requests do not create entries. Starting another
exploration supersedes older requests; Back invalidates pending responses.
Library/source changes and rebuilt data reset history with a short explanation.

The paper inspector prioritizes bibliographic details, opening the paper and its
abstract. Tags, collections and detailed extraction/indexing evidence are folded
into secondary sections. Exploration actions live only in the canvas toolbar.

## Workspace panel sizing

The exploration toolbar shows only its action buttons and scope menu, without
an enclosing selection box. Relationship inspection puts SOURCE and CITED WORK
labels on separate lines above their respective titles.

Drag the horizontal divider between Collections and Lenses, or between Reader
Contents and Thumbnails, to adjust their relative height. Each section keeps its
own scrolling area. Focus the divider and press Up/Down for five-percent steps;
Home/End select the 20/80-percent limits. Double-click resets the equal split.
Use section headings to collapse a panel; resizing is disabled while either
section is collapsed. Library and Reader divider proportions are saved separately
in local application preferences and restored after reopening a sidebar or
restarting the app. Double-click also saves the reset position. If preference
storage is unavailable, resizing still works for the current session.

## Assistant bar

Open Tyrion from its compact launcher. The expanded bar starts with the
assistant icon, Setup and Search Context. Setup opens the local model
settings. Search Context opens the answer-scope choices and current
context controls; the separate Broader search box is no longer used.

Click the assistant icon or either X control to collapse the composer
and workspace back to the launcher. Draft text and the selected answer
scope survive collapse. Opening and closing use a spring transition, with
a short fade for reduced-motion preferences. Keyboard focus moves to
the input on opening and returns to the launcher on collapse.

Preparation notices have their own dismiss button. Opening library setup
dismisses the notice and brings setup above the other workspace panels.
Escape, an outside click or its close button dismisses library setup.
The redundant Semantic Scale status box has been removed; graph zoom and
navigation controls remain available.

For the focused browser regression, start `npm run dev` and run
`npm run qa:agent-controls` with Firefox and Geckodriver available. It
uses the browser demo and checks both popovers, scope
persistence, collapse controls, keyboard focus, compact layout, light
and dark themes, and reduced motion.
