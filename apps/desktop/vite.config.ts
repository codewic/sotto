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
    },
  },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
});
