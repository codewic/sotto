// Development: a local relay, two desks (yours and a test peer) and the Vite UI.
// Uses plain TCP between local processes; real installs go through Tor.
//   desk "you":  agent on 127.0.0.1:7801, UI on http://localhost:5174
//   desk "peer": agent on 127.0.0.1:7802 (drive it with curl to simulate a contact)
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, "..");
const bin = resolve(app, "../../target/debug");
const data = join(app, ".dev-data");
mkdirSync(data, { recursive: true });

const procs = [];
function run(name, cmd, args, env = {}) {
  const p = spawn(cmd, args, { stdio: ["ignore", "inherit", "inherit"], env: { ...process.env, ...env } });
  p.on("exit", (code) => {
    if (code !== null && code !== 0) console.error(`[dev] ${name} exited with ${code}`);
  });
  procs.push(p);
  return p;
}

const relay = "ws://127.0.0.1:7400";
run("relay", join(bin, "sotto-relay"), ["--listen", "127.0.0.1:7400", "--data-dir", join(data, "relay")]);
for (const [name, port] of [["you", 7801], ["peer", 7802]]) {
  run(`agent-${name}`, join(bin, "sotto-agent"), ["--data", join(data, name), "--listen", `127.0.0.1:${port}`, "--token", "dev-token", "--direct", "--relay", relay], {
    SOTTO_RENDER: join(bin, "sotto-render"),
  });
}
run("vite", resolve(app, "node_modules/.bin/vite"), [], { SOTTO_AGENT_URL: "http://127.0.0.1:7801", SOTTO_AGENT_TOKEN: "dev-token" });

function stop() {
  for (const p of procs) p.kill("SIGTERM");
  process.exit(0);
}
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
