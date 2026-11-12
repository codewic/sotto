import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { ToastProvider } from "./ui/toast";
import "./styles.css";
import { stepZoom, toggleScheme } from "./theme/appearance";

const queryClient = new QueryClient({
  defaultOptions: {
    // Data stays fresh through live events from the agent, not timers.
    queries: { staleTime: Infinity, refetchOnWindowFocus: false, retry: 1 },
  },
});

function Root() {
  useEffect(() => {
    window.__SPLASH__?.hide();
  }, []);
  // Appearance keys work everywhere, the lock screen included.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      const k = e.key.toLowerCase();
      if (e.shiftKey && k === "l") {
        e.preventDefault();
        toggleScheme();
      } else if (k === "=" || k === "+") {
        e.preventDefault();
        stepZoom(1);
      } else if (k === "-" || k === "_") {
        e.preventDefault();
        stepZoom(-1);
      } else if (k === "0" && !e.shiftKey) {
        e.preventDefault();
        stepZoom(0);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <App />
      </ToastProvider>
    </QueryClientProvider>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
