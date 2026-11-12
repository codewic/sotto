import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In development the UI runs in a browser and talks to a local agent through this proxy.
// The proxy adds the agent's token, so the token never appears in page code or URLs.
const agent = process.env.SOTTO_AGENT_URL ?? "http://127.0.0.1:7801";
const token = process.env.SOTTO_AGENT_TOKEN ?? "dev-token";

export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  server: {
    port: Number(process.env.PORT ?? 5174),
    strictPort: true,
    proxy: {
      "/api": {
        target: agent,
        changeOrigin: true,
        headers: { authorization: `Bearer ${token}` },
      },
      // Development: the other test desks, reachable from this one page (?desk=peer, ?desk=third).
      "/desk/peer/api": {
        target: "http://127.0.0.1:7802",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/desk\/peer/, ""),
        headers: { authorization: `Bearer ${token}` },
      },
      "/desk/third/api": {
        target: "http://127.0.0.1:7803",
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/desk\/third/, ""),
        headers: { authorization: `Bearer ${token}` },
      },
    },
  },
  // Two pages: the app, and the call window (its own page, so it can be the only one with a microphone).
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: { input: { main: "index.html", call: "call.html" } },
  },
  worker: { format: "es" },
});
