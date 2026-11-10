import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { ToastProvider } from "./ui/toast";
import "./styles.css";

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
