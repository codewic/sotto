import { X } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import { IconButton } from "./Button";

/** Native modal dialog: focus trapping, Escape to close and inert background come from the platform. */
export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  width = 480,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: ReactNode;
  children: ReactNode;
  width?: number;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      className="sotto-dialog m-auto"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
      aria-labelledby="dialog-title"
    >
      {open && (
        <div className="dialog-card max-h-[85vh] overflow-y-auto rounded-2xl bg-pane p-6 text-foreground shadow-float" style={{ width: `min(${width}px, calc(100vw - 32px))` }}>
          <div className="mb-4 flex items-start justify-between gap-4">
            <div>
              <h2 id="dialog-title" className="text-[17px] font-semibold tracking-tight">
                {title}
              </h2>
              {description && <div className="mt-1 text-[13.5px] text-muted-foreground">{description}</div>}
            </div>
            <IconButton label="Close" onClick={onClose} className="-mt-1 -mr-2">
              <X size={16} />
            </IconButton>
          </div>
          {children}
        </div>
      )}
    </dialog>
  );
}
