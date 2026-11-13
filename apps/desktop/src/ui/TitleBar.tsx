import { Copy, Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Mark } from "../brand/Mark";

/**
 * Sotto's own title bar: the window has no system frame. The strip drags the window (double-click
 * maximizes), and the buttons on the right minimize, maximize or restore, and close. On macOS the
 * system's traffic lights stay, so only the space for them is kept.
 */
export function TitleBar({ brand }: { brand: boolean }) {
  const bridge = typeof window !== "undefined" ? window.sotto : undefined;
  const controls = bridge?.window;
  const mac = bridge?.platform === "darwin";
  const [max, setMax] = useState(() => controls?.isMaximized() ?? false);
  useEffect(() => controls?.onMaximized(setMax), [controls]);

  const button = "no-drag press grid h-8 w-10 cursor-pointer place-items-center rounded-lg text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground";
  return (
    <header className="drag flex h-10 shrink-0 items-center gap-2.5 pr-1.5 select-none" style={{ paddingLeft: mac ? 84 : 16 }} onDoubleClick={() => controls?.toggleMaximize()}>
      {brand && (
        <span className="flex items-center gap-2">
          <Mark size={20} />
          <span className="text-[13.5px] font-semibold tracking-tight">Sotto</span>
        </span>
      )}
      <span className="flex-1" />
      {controls && !mac && (
        <span className="flex items-center gap-0.5" onDoubleClick={(e) => e.stopPropagation()}>
          <button type="button" className={button} onClick={() => controls.minimize()} aria-label="Minimize" title="Minimize">
            <Minus size={15} />
          </button>
          <button type="button" className={button} onClick={() => controls.toggleMaximize()} aria-label={max ? "Restore" : "Maximize"} title={max ? "Restore" : "Maximize"}>
            {max ? <Copy size={12.5} className="-scale-x-100" /> : <Square size={12} />}
          </button>
          <button type="button" className={`${button} hover:!bg-destructive hover:!text-destructive-foreground`} onClick={() => controls.close()} aria-label="Close" title="Close">
            <X size={16} />
          </button>
        </span>
      )}
    </header>
  );
}
