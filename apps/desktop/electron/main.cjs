// Sotto desktop shell. The renderer is sandboxed with no Node access; every request goes
// through a narrow IPC bridge to the local agent, which listens on a private Unix socket.
const { app, BrowserWindow, ipcMain, dialog, session } = require("electron");
const { spawn } = require("node:child_process");
const http = require("node:http");
const path = require("node:path");
const fs = require("node:fs");

app.enableSandbox();

let agent = null;
let token = null;
let win = null;
const userData = app.getPath("userData");
const socketPath = path.join(userData, "agent.sock");

function agentBinary(name) {
  const packaged = path.join(process.resourcesPath || "", "bin", name);
  if (fs.existsSync(packaged)) return packaged;
  return path.resolve(__dirname, "../../../target/release", name);
}

function startAgent() {
  return new Promise((resolve, reject) => {
    const args = ["--data", userData, "--socket", socketPath];
    if (process.env.SOTTO_RELAY) args.push("--relay", process.env.SOTTO_RELAY);
    if (process.env.SOTTO_TOR) args.push("--tor", process.env.SOTTO_TOR);
    agent = spawn(agentBinary("sotto-agent"), args, {
      stdio: ["ignore", "pipe", "inherit"],
      env: { ...process.env, SOTTO_RENDER: agentBinary("sotto-render") },
    });
    agent.stdout.setEncoding("utf8");
    agent.stdout.on("data", (chunk) => {
      const m = /SOTTO_AGENT_READY ([0-9a-f]{64})/.exec(chunk);
      if (m) {
        token = m[1];
        resolve();
      }
    });
    agent.on("exit", (code) => {
      if (!token) reject(new Error(`agent exited with ${code}`));
      else if (win) win.webContents.send("sotto:agent-exit", code);
    });
  });
}

/** One request to the agent over the Unix socket. Bodies are JSON or raw bytes. */
function request(method, urlPath, body, headers = {}) {
  return new Promise((resolve) => {
    if (typeof urlPath !== "string" || !urlPath.startsWith("/api/")) return resolve({ status: 400, body: { error: "Bad path." } });
    const isBytes = body instanceof Uint8Array;
    const payload = body == null ? null : isBytes ? Buffer.from(body) : Buffer.from(JSON.stringify(body));
    const req = http.request(
      {
        socketPath,
        method,
        path: urlPath,
        headers: {
          host: "sotto",
          authorization: `Bearer ${token}`,
          ...(payload ? { "content-type": isBytes ? headers["content-type"] || "application/octet-stream" : "application/json", "content-length": payload.length } : {}),
          ...(headers["x-file-name"] ? { "x-file-name": headers["x-file-name"] } : {}),
          ...(headers["x-caption"] ? { "x-caption": headers["x-caption"] } : {}),
        },
      },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const buf = Buffer.concat(chunks);
          const type = res.headers["content-type"] || "";
          if (type.startsWith("image/")) return resolve({ status: res.statusCode, bytes: new Uint8Array(buf), type });
          let parsed = null;
          try {
            parsed = buf.length ? JSON.parse(buf.toString("utf8")) : null;
          } catch {
            parsed = null;
          }
          resolve({ status: res.statusCode, body: parsed });
        });
      },
    );
    req.on("error", () => resolve({ status: 503, body: { error: "Sotto's background service isn't responding." } }));
    if (payload) req.write(payload);
    req.end();
  });
}

/** Live events: one long-lived SSE stream from the agent, forwarded to the window. */
function streamEvents() {
  const req = http.request({ socketPath, method: "GET", path: "/api/events", headers: { host: "sotto", authorization: `Bearer ${token}` } }, (res) => {
    res.setEncoding("utf8");
    let buf = "";
    res.on("data", (chunk) => {
      buf += chunk;
      let i;
      while ((i = buf.indexOf("\n\n")) >= 0) {
        const block = buf.slice(0, i);
        buf = buf.slice(i + 2);
        const line = block.split("\n").find((l) => l.startsWith("data:"));
        if (line && win) win.webContents.send("sotto:event", line.slice(5).trim());
      }
    });
    res.on("end", () => setTimeout(streamEvents, 1000));
  });
  req.on("error", () => setTimeout(streamEvents, 1000));
  req.end();
}

function createWindow() {
  win = new BrowserWindow({
    width: 1280,
    height: 820,
    minWidth: 960,
    minHeight: 640,
    title: "Sotto",
    backgroundColor: "#f5f5f5",
    show: false,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
      spellcheck: false,
      devTools: !app.isPackaged,
    },
  });
  win.once("ready-to-show", () => win.show());
  // No navigation away from the app and no new windows.
  win.webContents.on("will-navigate", (e) => e.preventDefault());
  win.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  // Keep window contents out of screenshots and screen sharing where the OS supports it.
  win.setContentProtection(true);
  win.loadFile(path.join(__dirname, "../dist/index.html"));
}

// Only the app's own page may use the bridge.
const fromApp = (e) => Boolean(e.senderFrame && e.senderFrame.url.startsWith("file://") && win && e.sender === win.webContents);
ipcMain.handle("sotto:request", (e, method, urlPath, body, headers) =>
  fromApp(e) ? request(method, urlPath, body, headers || {}) : { status: 403, body: { error: "Not allowed." } },
);
ipcMain.handle("sotto:save-dialog", async (e, name) => {
  if (!fromApp(e)) return null;
  const r = await dialog.showSaveDialog(win, { defaultPath: String(name || "clean-copy.pdf"), title: "Save clean copy" });
  return r.canceled ? null : r.filePath;
});
ipcMain.handle("sotto:content-protection", (e, on) => fromApp(e) && win.setContentProtection(!!on));

app.whenReady().then(async () => {
  // Deny every browser permission request.
  session.defaultSession.setPermissionRequestHandler((_wc, _perm, cb) => cb(false));
  // The renderer may only load its own files: nothing on the network.
  session.defaultSession.webRequest.onBeforeRequest((details, cb) => {
    cb({ cancel: !details.url.startsWith("file://") && !details.url.startsWith("devtools://") });
  });
  try {
    await startAgent();
  } catch (e) {
    dialog.showErrorBox("Sotto can't start", String(e && e.message ? e.message : e));
    app.quit();
    return;
  }
  streamEvents();
  createWindow();
});

app.on("window-all-closed", () => app.quit());
app.on("before-quit", () => {
  if (agent) agent.kill("SIGINT");
});
app.on("web-contents-created", (_e, contents) => {
  contents.on("will-attach-webview", (e) => e.preventDefault());
});
