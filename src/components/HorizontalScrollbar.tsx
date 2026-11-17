import { useRef } from "react";

interface HorizontalScrollbarProps {
  controlsId: string;
  label: string;
  viewport: number;
  maximum: number;
  position: number;
  onScroll: (position: number) => void;
}

/** Keep a proportional scrollbar visible even on systems with overlay bars. */
export function HorizontalScrollbar({
  controlsId,
  label,
  viewport,
  maximum,
  position,
  onScroll,
}: HorizontalScrollbarProps) {
  const dragOffset = useRef<number | null>(null);
  if (maximum <= 0) return null;
  const trackWidth = Math.max(1, viewport);
  const thumbWidth = Math.min(
    trackWidth,
    Math.max(28, (viewport / (viewport + maximum)) * trackWidth),
  );
  const travel = trackWidth - thumbWidth;
  const thumbLeft = travel * Math.min(1, Math.max(0, position / maximum));
  const scroll = (next: number) =>
    onScroll(Math.max(0, Math.min(maximum, next)));
  return (
    <div
      className="horizontal-scrollbar"
      role="scrollbar"
      tabIndex={0}
      aria-label={label}
      aria-controls={controlsId}
      aria-orientation="horizontal"
      aria-valuemin={0}
      aria-valuemax={Math.round(maximum)}
      aria-valuenow={Math.round(position)}
      onKeyDown={(event) => {
        const next = {
          ArrowLeft: position - 40,
          ArrowRight: position + 40,
          PageUp: position - viewport,
          PageDown: position + viewport,
          Home: 0,
          End: maximum,
        }[event.key];
        if (next === undefined) return;
        event.preventDefault();
        scroll(next);
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.focus();
        const x =
          event.clientX - event.currentTarget.getBoundingClientRect().left;
        if (x >= thumbLeft && x <= thumbLeft + thumbWidth) {
          dragOffset.current = x - thumbLeft;
          event.currentTarget.setPointerCapture(event.pointerId);
        } else {
          scroll(position + (x < thumbLeft ? -viewport : viewport));
        }
      }}
      onPointerMove={(event) => {
        if (dragOffset.current === null || travel <= 0) return;
        const x =
          event.clientX - event.currentTarget.getBoundingClientRect().left;
        scroll(((x - dragOffset.current) / travel) * maximum);
      }}
      onPointerUp={(event) => {
        dragOffset.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
      onLostPointerCapture={() => {
        dragOffset.current = null;
      }}
    >
      <span
        style={{
          width: `${(thumbWidth / trackWidth) * 100}%`,
          left: `${(thumbLeft / trackWidth) * 100}%`,
        }}
      />
    </div>
  );
}
