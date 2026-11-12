// The call window: one call, nothing else. Opened by the app (or a browser popup in development)
// at call.html#<call id>.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import "../theme/appearance";
import { CallScreen } from "./CallScreen";

const callId = location.hash.replace(/^#/, "");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {/^[0-9a-f]{32}$/.test(callId) ? (
      <CallScreen callId={callId} />
    ) : (
      <div className="grid h-svh place-items-center p-6 text-center text-[13.5px] text-muted-foreground">This call link isn't valid. Close this window and call again from Sotto.</div>
    )}
  </StrictMode>,
);
