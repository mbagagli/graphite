import { Info, X } from "lucide-react";
import { useEffect, useId, useRef } from "react";
import type { ResearchItem } from "../../domain/research";
import type { GraphMode } from "./graphLayout";
import { IconButton } from "../../components/IconButton";

export function GraphSelection({
  paper,
  title,
  additionalCount,
}: {
  paper?: ResearchItem;
  title?: string;
  additionalCount: number;
}) {
  if (!paper && !title) return null;
  return (
    <div className="graph-selection" role="status" aria-atomic="true">
      <strong>
        {paper?.title ?? title}
        {paper?.year != null && ` [${paper.year}]`}
      </strong>
      <em className="graph-selection-authors">
        {paper?.authors.map((author) => author.name).join(", ") ||
          "Unknown author"}
      </em>
      {additionalCount > 0 && <span>+{additionalCount} more selected</span>}
    </div>
  );
}

export function GraphNavigationHelp({ mode }: { mode: GraphMode }) {
  const id = useId();
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const dismiss = () => panel.current?.hidePopover();
    window.addEventListener("resize", dismiss);
    return () => window.removeEventListener("resize", dismiss);
  }, []);
  return (
    <>
      <IconButton
        label="Canvas navigation help"
        icon={Info}
        size="small"
        popoverTarget={id}
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          const element = panel.current;
          if (!element) return;
          element.style.left = `${Math.max(8, rect.left - 292)}px`;
          element.style.top = `${Math.max(8, Math.min(rect.top, window.innerHeight - 290))}px`;
        }}
      />
      <div
        ref={panel}
        id={id}
        popover="auto"
        className="graph-navigation-help"
        role="dialog"
        aria-label={`${mode.toUpperCase()} navigation`}
      >
        <header>
          <strong>{mode.toUpperCase()} navigation</strong>
          <IconButton
            label="Close navigation help"
            icon={X}
            size="small"
            popoverTarget={id}
            popoverTargetAction="hide"
          />
        </header>
        <ul>
          <li>
            {mode === "3d"
              ? "Drag to orbit; right-drag to pan."
              : "Drag to pan."}
          </li>
          <li>
            {mode === "3d"
              ? "Scroll or pinch to dive."
              : "Scroll or pinch to zoom."}
          </li>
          <li>Click to select; Shift/Command-click to add papers.</li>
          <li>Double-click or Focus selection to focus a paper.</li>
          <li>Fit graph restores the overview.</li>
          <li>
            With the canvas focused: arrow keys{" "}
            {mode === "3d" ? "orbit" : "pan"}, + / - zoom, Home fits.
          </li>
        </ul>
        <p>
          Graph proximity aids navigation; it is not a scientific similarity
          measure.
        </p>
      </div>
    </>
  );
}
