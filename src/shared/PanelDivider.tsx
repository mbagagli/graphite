import { useEffect, useRef, useState } from "react";

const MIN_PERCENT = 20;
const MAX_PERCENT = 80;
const DEFAULT_PERCENT = 50;
const KEYBOARD_STEP = 5;

/** Restore a bounded layout preference; unavailable storage is optional. */
function readPercent(storageKey: string): number {
  if (typeof window === "undefined") return DEFAULT_PERCENT;
  try {
    const stored = window.localStorage.getItem(storageKey);
    if (!stored?.trim()) return DEFAULT_PERCENT;
    const value = Number(stored);
    return Number.isFinite(value)
      ? Math.min(MAX_PERCENT, Math.max(MIN_PERCENT, value))
      : DEFAULT_PERCENT;
  } catch {
    console.warn("Panel sizes could not be restored from local storage.");
    return DEFAULT_PERCENT;
  }
}

/** Resize the two adjacent panels without rerendering their contents. */
export function PanelDivider({
  label,
  enabled,
  storageKey,
}: {
  label: string;
  enabled: boolean;
  storageKey: string;
}) {
  const [percent, setPercent] = useState(() => readPercent(storageKey));
  const element = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    id: number;
    y: number;
    percent: number;
    height: number;
  } | null>(null);
  const update = (value: number) => {
    const next = Math.min(MAX_PERCENT, Math.max(MIN_PERCENT, value));
    setPercent(next);
    try {
      window.localStorage.setItem(storageKey, String(next));
    } catch {
      console.warn("Panel sizes could not be saved to local storage.");
    }
  };
  useEffect(() => {
    element.current?.parentElement?.style.setProperty(
      "--panel-split",
      String(percent),
    );
  }, [percent]);
  return (
    <div
      ref={element}
      className={`panel-divider${enabled ? "" : " is-disabled"}`}
      role={enabled ? "separator" : undefined}
      aria-hidden={!enabled || undefined}
      aria-label={enabled ? label : undefined}
      aria-orientation={enabled ? "horizontal" : undefined}
      aria-valuemin={enabled ? MIN_PERCENT : undefined}
      aria-valuemax={enabled ? MAX_PERCENT : undefined}
      aria-valuenow={enabled ? Math.round(percent) : undefined}
      aria-valuetext={
        enabled
          ? `${Math.round(percent)} percent for the upper panel`
          : undefined
      }
      tabIndex={enabled ? 0 : undefined}
      title={
        enabled
          ? "Drag or use Up/Down to resize; double-click to reset"
          : undefined
      }
      onDoubleClick={() => {
        if (enabled) update(DEFAULT_PERCENT);
      }}
      onKeyDown={(event) => {
        if (!enabled) return;
        const value = {
          ArrowUp: percent - KEYBOARD_STEP,
          ArrowDown: percent + KEYBOARD_STEP,
          Home: MIN_PERCENT,
          End: MAX_PERCENT,
        }[event.key];
        if (value === undefined) return;
        event.preventDefault();
        update(value);
      }}
      onPointerDown={(event) => {
        if (!enabled || event.button !== 0) return;
        const parent = event.currentTarget.parentElement;
        if (!parent) return;
        drag.current = {
          id: event.pointerId,
          y: event.clientY,
          percent,
          height: Math.max(
            1,
            parent.clientHeight - event.currentTarget.clientHeight,
          ),
        };
        event.currentTarget.setPointerCapture(event.pointerId);
        event.preventDefault();
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start || !enabled || start.id !== event.pointerId) return;
        update(
          start.percent + (100 * (event.clientY - start.y)) / start.height,
        );
      }}
      onPointerUp={(event) => {
        drag.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={() => {
        drag.current = null;
      }}
      onLostPointerCapture={() => {
        drag.current = null;
      }}
    />
  );
}
