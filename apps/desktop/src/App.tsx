import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { get, subscribe } from "./api";
import { MarkIndicator } from "./brand/Mark";
import { Setup } from "./screens/Setup";
import { Shell } from "./screens/Shell";
import { Unlock } from "./screens/Unlock";
import type { Status } from "./types";
import { Button } from "./ui/Button";
import { useToast } from "./ui/toast";

export function App() {
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
            break;
          case "message":
          case "message_removed":
            qc.invalidateQueries({ queryKey: ["messages", e.contact_id] });
            qc.invalidateQueries({ queryKey: ["contacts"] });
            qc.invalidateQueries({ queryKey: ["files"] });
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
