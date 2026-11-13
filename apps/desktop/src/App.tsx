import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { get, subscribe } from "./api";
import { MarkIndicator } from "./brand/Mark";
import { Setup } from "./screens/Setup";
import { Shell } from "./screens/Shell";
import { Unlock } from "./screens/Unlock";
import type { Status } from "./types";
import { Button } from "./ui/Button";
import { TitleBar } from "./ui/TitleBar";
import { useToast } from "./ui/toast";

/** Every screen sits under Sotto's own title bar (the window has no system frame). */
export function App() {
  const status = useQuery({ queryKey: ["status"], queryFn: () => get<Status>("/api/status") });
  const unlocked = !!status.data && status.data.state !== "new" && status.data.state !== "locked";
  return (
    <div className="flex h-svh flex-col bg-background">
      <TitleBar brand={unlocked} />
      <div className="min-h-0 flex-1">
        <Screens />
      </div>
    </div>
  );
}

function Screens() {
  const qc = useQueryClient();
  const toast = useToast();
  const status = useQuery({ queryKey: ["status"], queryFn: () => get<Status>("/api/status") });

  // Live updates: every change arrives as an event and refreshes exactly what it touches.
  useEffect(
    () =>
      subscribe((e) => {
        switch (e.type) {
          case "state":
          case "resync":
          case "connected":
            qc.invalidateQueries();
            break;
          case "contacts":
          case "contact":
            qc.invalidateQueries({ queryKey: ["contacts"] });
            qc.invalidateQueries({ queryKey: ["tag"] });
            break;
          case "message":
          case "message_removed":
            qc.invalidateQueries({ queryKey: ["messages", e.contact_id] });
            qc.invalidateQueries({ queryKey: ["contacts"] });
            qc.invalidateQueries({ queryKey: ["files"] });
            break;
          case "group":
            qc.invalidateQueries({ queryKey: ["groups"] });
            if (e.id) qc.invalidateQueries({ queryKey: ["group-messages", e.id] });
            break;
          case "channel":
            qc.invalidateQueries({ queryKey: ["channels"] });
            if (e.id) {
              qc.invalidateQueries({ queryKey: ["channel-posts", e.id] });
              qc.invalidateQueries({ queryKey: ["channel-replies", e.id] });
            }
            break;
          case "me":
            qc.invalidateQueries({ queryKey: ["me-avatar"] });
            break;
          case "update":
            qc.invalidateQueries({ queryKey: ["updates"] });
            break;
          case "secret":
            if (e.contact_id) qc.invalidateQueries({ queryKey: ["secrets", e.contact_id] });
            break;
          case "call":
            qc.invalidateQueries({ queryKey: ["calls"] });
            break;
          case "scheduled":
            qc.invalidateQueries({ queryKey: ["scheduled"] });
            break;
          case "relay":
            qc.invalidateQueries({ queryKey: ["status"] });
            break;
          case "drop":
            qc.invalidateQueries({ queryKey: ["drops"] });
            qc.invalidateQueries({ queryKey: ["drop-sources"] });
            qc.invalidateQueries({ queryKey: ["drop-items"] });
            qc.invalidateQueries({ queryKey: ["files"] });
            break;
          case "review":
            qc.invalidateQueries({ queryKey: ["files"] });
            qc.invalidateQueries({ queryKey: ["review"] });
            qc.invalidateQueries({ queryKey: ["drop-items"] });
            break;
          case "case":
            qc.invalidateQueries({ queryKey: ["cases"] });
            break;
          case "notice":
            toast("info", String(e.text ?? ""));
            break;
        }
      }),
    [qc, toast],
  );

  if (status.isPending) {
    return (
      <div className="grid h-full place-items-center">
        <MarkIndicator size={28} label="Starting Sotto…" />
      </div>
    );
  }
  if (status.isError) {
    return (
      <div className="grid h-full place-items-center p-6">
        <div className="max-w-sm text-center">
          <h1 className="text-lg font-semibold">Sotto's background service isn't responding</h1>
          <p className="mt-2 text-[13.5px] text-muted-foreground">It keeps your vault and connections. Quit Sotto and open it again; if that doesn't help, restart this computer.</p>
          <Button className="mt-5" onClick={() => status.refetch()}>
            Try again
          </Button>
        </div>
      </div>
    );
  }
  switch (status.data.state) {
    case "new":
      return <Setup />;
    case "locked":
      return <Unlock />;
    default:
      return <Shell status={status.data} />;
  }
}
