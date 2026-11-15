// The call window: one call, nothing else. Opened by the app (or a browser popup in development)
// at call.html#<call id>.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import "../theme/appearance";
import { CallScreen } from "./CallScreen";
import { TitleBar } from "../ui/TitleBar";
import { PanelLeftOpen } from "lucide-react";

const callId = location.hash.replace(/^#/, "");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <div className="flex h-svh flex-col bg-background">
      <TitleBar
        brand
        actions={
          window.sotto?.dock ? (
            <button
              type="button"
              onClick={() => window.sotto?.dock?.()}
              className="press flex h-8 cursor-pointer items-center gap-1.5 rounded-lg px-2.5 text-[12.5px] font-medium text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground"
              title="Back to Sotto. The call keeps going; bring it back from the call bar."
            >
              <PanelLeftOpen size={14} aria-hidden="true" />
              Back to Sotto
            </button>
          ) : null
        }
      />
      <div className="min-h-0 flex-1">
        {/^[0-9a-f]{32}$/.test(callId) ? (
          <CallScreen callId={callId} />
        ) : (
          <div className="grid h-full place-items-center p-6 text-center text-[13.5px] text-muted-foreground">This call link isn't valid. Close this window and call again from Sotto.</div>
        )}
      </div>
    </div>
  </StrictMode>,
);
