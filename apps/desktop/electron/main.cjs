// Sotto desktop shell. The renderer is sandboxed with no Node access; every request goes
// through a narrow IPC bridge to the local agent, which listens on a private Unix socket.
const { app, BrowserWindow, Menu, ipcMain, dialog, nativeTheme, session } = require("electron");
const { spawn } = require("node:child_process");
const http = require("node:http");
const path = require("node:path");
const fs = require("node:fs");

app.enableSandbox();

let agent = null;
let token = null;
let win = null;
// The call window: at most one, for one call.
let callWin = null;
let callId = null;
let callHost = null;
const userData = app.getPath("userData");

// The Sotto release key's public half (Ed25519), the same one pinned in sotto-core. The shell checks
// an update's signature itself before installing, so a tampered agent alone can't install one.
const RELEASE_PUBLIC_KEY = "7f1ed181be7057fd7cc171dadafff768aeda09c5f7ddbba2839e21a65f518a3d";

/** How this copy was installed, so the agent fetches the package that can replace it. */
function packageKind() {
  if (process.env.APPIMAGE) return "appimage";
  if (!app.isPackaged) return "";
  if (process.platform === "linux") return "deb";
  if (process.platform === "win32") return "exe";
  return "";
}
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
      env: { ...process.env, SOTTO_RENDER: agentBinary("sotto-render"), SOTTO_PACKAGE: packageKind() },
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
          // "photo" or "video": the agent cleans the file before sending it.
          ...(headers["x-send-as"] === "photo" || headers["x-send-as"] === "video" ? { "x-send-as": headers["x-send-as"] } : {}),
        },
      },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const buf = Buffer.concat(chunks);
          const type = res.headers["content-type"] || "";
          if (type.startsWith("image/") || type.startsWith("audio/") || type.startsWith("video/")) return resolve({ status: res.statusCode, bytes: new Uint8Array(buf), type });
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
        if (line && callWin) callWin.webContents.send("sotto:event", line.slice(5).trim());
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
    // Close to the theme the page will pick; the page sends the exact colour once it loads.
    backgroundColor: nativeTheme.shouldUseDarkColors ? "#0b0b0c" : "#f3f3f3",
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

/** The call window lives in its own session: the only place a microphone is allowed, and it may
 *  reach exactly one call server. The main window (which renders message text) keeps denying all. */
function callSession() {
  const s = session.fromPartition("sotto-call");
  if (!s.__sottoReady) {
    s.__sottoReady = true;
    const isCallWindow = (wc) => Boolean(callWin && wc === callWin.webContents);
    s.setPermissionRequestHandler((wc, perm, cb, details) => {
      const audioOnly = Array.isArray(details.mediaTypes) && details.mediaTypes.length > 0 && details.mediaTypes.every((t) => t === "audio");
      cb(isCallWindow(wc) && perm === "media" && audioOnly);
    });
    s.setPermissionCheckHandler((wc, perm) => isCallWindow(wc) && perm === "media");
    s.webRequest.onBeforeRequest((details, cb) => {
      let ok = details.url.startsWith("file://") || details.url.startsWith("devtools://");
      if (!ok && callHost) {
        try {
          const u = new URL(details.url);
          ok = (u.protocol === "wss:" || u.protocol === "https:") && u.host === callHost;
        } catch {
          ok = false;
        }
      }
      cb({ cancel: !ok });
    });
  }
  return s;
}

async function openCall(id) {
  if (callWin && callId === id) {
    callWin.show();
    callWin.focus();
    return;
  }
  // Learn which call server this call uses, so the window may reach that one host and no other.
  const r = await request("GET", `/api/calls/${id}/join`, null);
  if (r.status !== 200 || !r.body || typeof r.body.url !== "string") return;
  let host;
  try {
    host = new URL(r.body.url).host;
  } catch {
    return;
  }
  if (callWin) callWin.close();
  callId = id;
  callHost = host;
  callWin = new BrowserWindow({
    width: 400,
    height: 620,
    minWidth: 340,
    minHeight: 520,
    title: "Sotto call",
    backgroundColor: nativeTheme.shouldUseDarkColors ? "#0b0b0c" : "#f3f3f3",
    show: false,
    webPreferences: {
      preload: path.join(__dirname, "preload-call.cjs"),
      session: callSession(),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
      spellcheck: false,
      devTools: !app.isPackaged,
    },
  });
  const me = callWin;
  me.once("ready-to-show", () => me.show());
  me.webContents.on("will-navigate", (e) => e.preventDefault());
  me.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  me.setContentProtection(true);
  me.on("closed", () => {
    // Closing the window hangs up. Harmless if the call already ended.
    request("POST", `/api/calls/${id}/end`, {});
    if (callWin === me) {
      callWin = null;
      callId = null;
      callHost = null;
    }
  });
  me.loadFile(path.join(__dirname, "../dist/call.html"), { hash: id });
}

// Only the app's own page may use the bridge.
const fromApp = (e) => Boolean(e.senderFrame && e.senderFrame.url.startsWith("file://") && win && e.sender === win.webContents);
ipcMain.handle("sotto:request", (e, method, urlPath, body, headers) =>
  fromApp(e) ? request(method, urlPath, body, headers || {}) : { status: 403, body: { error: "Not allowed." } },
);
/** Verify a staged update again (signature and fingerprint), then install it and restart. */
async function installUpdate() {
  const crypto = require("node:crypto");
  const r = await request("GET", "/api/updates/staged", null, {});
  if (r.status !== 200) return { ok: false, error: "There's no update ready to install." };
  const { path: file, kind, manifest, sig } = r.body;
  const spki = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(RELEASE_PUBLIC_KEY, "hex")]);
  const key = crypto.createPublicKey({ key: spki, format: "der", type: "spki" });
  const signed = Buffer.concat([Buffer.from("sotto-release-manifest-v1\n"), Buffer.from(manifest, "utf8")]);
  if (!/^[0-9a-f]{128}$/.test(String(sig).trim()) || !crypto.verify(null, signed, key, Buffer.from(String(sig).trim(), "hex"))) {
    return { ok: false, error: "The update's signature didn't check out, so it wasn't installed." };
  }
  const m = JSON.parse(manifest);
  const entry = (m.files || []).find((f) => f.kind === kind && f.name === path.basename(file));
  const bytes = fs.readFileSync(file);
  if (!entry || crypto.createHash("sha256").update(bytes).digest("hex") !== entry.sha256) {
    return { ok: false, error: "The downloaded update didn't match its signed fingerprint, so it wasn't installed." };
  }
  if (kind === "appimage" && process.env.APPIMAGE) {
    const dest = process.env.APPIMAGE;
    const next = `${dest}.new`;
    fs.writeFileSync(next, bytes, { mode: 0o755 });
    fs.renameSync(next, dest);
    app.relaunch();
    app.exit(0);
    return { ok: true };
  }
  if (kind === "deb") {
    // The system asks for permission; nothing is installed without it.
    const code = await new Promise((resolve) => spawn("pkexec", ["apt-get", "install", "-y", file], { stdio: "ignore" }).on("exit", resolve).on("error", () => resolve(-1)));
    if (code !== 0) return { ok: false, error: "The update wasn't installed (permission was refused or the installer failed)." };
    app.relaunch();
    app.exit(0);
    return { ok: true };
  }
  if (kind === "exe") {
    spawn(file, ["/S"], { detached: true, stdio: "ignore" }).unref();
    app.quit();
    return { ok: true };
  }
  return { ok: false, error: "This copy of Sotto can't update itself. Download the new version from the releases page." };
}

ipcMain.handle("sotto:install-update", (e) => (fromApp(e) ? installUpdate().catch(() => ({ ok: false, error: "The update couldn't be installed." })) : { ok: false, error: "Not allowed." }));

ipcMain.handle("sotto:save-dialog", async (e, name) => {
  if (!fromApp(e)) return null;
  const r = await dialog.showSaveDialog(win, { defaultPath: String(name || "clean-copy.pdf"), title: "Save clean copy" });
  return r.canceled ? null : r.filePath;
});
ipcMain.handle("sotto:content-protection", (e, on) => fromApp(e) && win.setContentProtection(!!on));
ipcMain.handle("sotto:open-call", (e, id) => {
  if (!fromApp(e) || typeof id !== "string" || !/^[0-9a-f]{32}$/.test(id)) return;
  return openCall(id);
});
// The call window may only talk to its own call's routes.
const fromCall = (e) => Boolean(callWin && e.sender === callWin.webContents && e.senderFrame && e.senderFrame.url.startsWith("file://"));
ipcMain.handle("sotto:call-request", (e, method, urlPath, body) => {
  if (!fromCall(e) || typeof urlPath !== "string" || !callId || !urlPath.startsWith(`/api/calls/${callId}/`)) return { status: 403, body: { error: "Not allowed." } };
  return request(method, urlPath, body, {});
});
ipcMain.handle("sotto:appearance", (e, a) => {
  if (!fromApp(e) || !a || typeof a !== "object") return;
  if (a.scheme === "light" || a.scheme === "dark") nativeTheme.themeSource = a.scheme;
  if (typeof a.background === "string" && /^#[0-9a-f]{6}$/i.test(a.background)) win.setBackgroundColor(a.background);
});

/** No menu bar, and no built-in zoom keys: Sotto handles Ctrl +, Ctrl - and Ctrl 0 itself so the
 *  Appearance setting always matches. macOS keeps the menus it needs for copy and paste. */
function setMenu() {
  const dev = app.isPackaged ? [] : [{ label: "Develop", submenu: [{ role: "reload" }, { role: "toggleDevTools" }] }];
  if (process.platform === "darwin") Menu.setApplicationMenu(Menu.buildFromTemplate([{ role: "appMenu" }, { role: "editMenu" }, { role: "windowMenu" }, ...dev]));
  else Menu.setApplicationMenu(dev.length ? Menu.buildFromTemplate(dev) : null);
}

app.whenReady().then(async () => {
  setMenu();
  // Deny every browser permission request.
  // The main window may use the microphone for voice notes: audio only, and nothing else.
  session.defaultSession.setPermissionRequestHandler((_wc, perm, cb, details) => {
    const audioOnly = Array.isArray(details.mediaTypes) && details.mediaTypes.length > 0 && details.mediaTypes.every((t) => t === "audio");
    cb(perm === "media" && audioOnly);
  });
  session.defaultSession.setPermissionCheckHandler((_wc, perm) => perm === "media");
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
