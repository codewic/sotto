import { useCallback, useEffect, useRef, useState } from "react";

/** A pane width this person chose, remembered on this computer (best effort: storage can be off). */
export function useStoredWidth(key: string, fallback: () => number, min: number, max: number): [number, (w: number) => void, () => void] {
  const [width, setWidth] = useState(() => {
    try {
      const v = Number(localStorage.getItem(key));
      if (Number.isFinite(v) && v >= min && v <= max) return v;
    } catch {
      // Storage unavailable: use the default.
    }
    return fallback();
  });
  const save = useCallback(
    (w: number) => {
      setWidth(w);
      try {
        localStorage.setItem(key, String(Math.round(w)));
      } catch {
        // Not remembered this time; the pane still resizes.
      }
    },
    [key],
  );
  const reset = useCallback(() => {
    try {
      localStorage.removeItem(key);
    } catch {
      // Nothing to forget.
    }
    setWidth(fallback());
  }, [key, fallback]);
  return [width, save, reset];
}

/**
 * The drag handle on a pane's edge. Drag it, use the arrow keys when it has focus, or double-click
 * to put the pane back to its default width. `snap` lets a pane jump between states (for example a
 * sidebar that collapses to icons).
 */
export function ResizeHandle({
  label,
  value,
  min,
  max,
  onChange,
  onReset,
  snap = (v) => v,
  className = "",
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (w: number) => void;
  onReset: () => void;
  snap?: (w: number) => number;
  className?: string;
}) {
  const start = useRef<{ x: number; w: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const clamp = (v: number) => Math.min(max, Math.max(min, v));

  useEffect(() => {
    if (!dragging) return;
    // While dragging, the whole window shows the resize cursor and text doesn't get selected.
    const prev = document.body.style.cursor;
    document.body.style.cursor = "col-resize";
    document.body.classList.add("select-none");
    return () => {
      document.body.style.cursor = prev;
      document.body.classList.remove("select-none");
    };
  }, [dragging]);

  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable separator is the ARIA pattern for a resizable pane edge.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      title={`${label}. Drag to resize, double-click to reset.`}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, w: value };
        setDragging(true);
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        onChange(snap(clamp(start.current.w + e.clientX - start.current.x)));
      }}
      onPointerUp={(e) => {
        start.current = null;
        setDragging(false);
        (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
      }}
      onDoubleClick={onReset}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 64 : 16;
        if (e.key === "ArrowLeft") onChange(snap(clamp(value - step)));
        else if (e.key === "ArrowRight") onChange(snap(clamp(value + step)));
        else if (e.key === "Home") onChange(snap(min));
        else if (e.key === "End") onChange(snap(max));
        else return;
        e.preventDefault();
      }}
      className={`no-drag group/resize absolute top-0 z-20 flex h-full w-3 cursor-col-resize justify-center outline-none ${className}`}
    >
      {/* A hairline that shows on hover, focus and drag, in the accent colour. */}
      <span
        aria-hidden="true"
        className={`h-full w-0.5 rounded-full transition-colors duration-150 ${dragging ? "bg-primary" : "bg-transparent group-hover/resize:bg-[color-mix(in_srgb,var(--primary)_55%,transparent)] group-focus-visible/resize:bg-primary"}`}
      />
    </div>
  );
}
