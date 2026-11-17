import type { ResearchItem, ResearchSelection } from "../domain/research";

export interface PaperSelectionState {
  papers: ResearchItem[];
  source: ResearchSelection["source"];
  undo: ResearchItem[] | null;
  notice: string | null;
}

export type PaperSelectionAction =
  | {
      type: "add" | "toggle" | "replace";
      paper: ResearchItem;
      source: ResearchSelection["source"];
    }
  | { type: "remove"; id: string }
  | { type: "clear" | "undo" | "reset" };

export const emptyPaperSelection: PaperSelectionState = {
  papers: [],
  source: "graph",
  undo: null,
  notice: null,
};

/** Keep collected metadata independent of bounded graph projections. */
export function paperSelectionReducer(
  state: PaperSelectionState,
  action: PaperSelectionAction,
): PaperSelectionState {
  switch (action.type) {
    case "reset":
      return {
        ...emptyPaperSelection,
        notice: state.papers.length
          ? "Selection cleared because you changed libraries."
          : null,
      };
    case "undo":
      return state.undo
        ? {
            ...state,
            papers: state.undo,
            undo: null,
            notice: "Selection restored.",
          }
        : state;
    case "clear":
      return state.papers.length
        ? {
            ...state,
            papers: [],
            undo: state.papers,
            notice: "Selection cleared.",
          }
        : state;
    case "remove": {
      const paper = state.papers.find((item) => item.id === action.id);
      if (!paper) return state;
      return {
        ...state,
        papers: state.papers.filter((item) => item.id !== action.id),
        undo: state.papers,
        notice: `Removed ${paper.title}.`,
      };
    }
    case "replace":
      return {
        papers: [action.paper],
        source: action.source,
        undo: state.papers.length ? state.papers : null,
        notice: null,
      };
    case "add":
    case "toggle": {
      const included = state.papers.some((p) => p.id === action.paper.id);
      if (included) {
        return action.type === "toggle"
          ? paperSelectionReducer(state, {
              type: "remove",
              id: action.paper.id,
            })
          : state;
      }
      return {
        papers: [...state.papers, action.paper],
        source: action.source,
        undo: null,
        notice: null,
      };
    }
  }
}
