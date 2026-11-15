// Bridge for the call window. It can reach its own call's routes on the agent and receive live
// events, nothing else: no files, no other conversations, no settings.
const { contextBridge, ipcRenderer, webFrame } = require("electron");

contextBridge.exposeInMainWorld("sotto", {
  request: (method, path, body) => ipcRenderer.invoke("sotto:call-request", method, path, body),
  onEvent: (fn) => {
    const handler = (_e, data) => fn(data);
    ipcRenderer.on("sotto:event", handler);
    return () => ipcRenderer.removeListener("sotto:event", handler);
  },
  saveDialog: async () => null,
  setContentProtection: async () => {},
  setZoom: (factor) => {
    const f = Number(factor);
    if (Number.isFinite(f) && f >= 0.8 && f <= 1.5) webFrame.setZoomFactor(f);
  },
  // Tuck the call away and go back to the main window; the call keeps going.
  dock: () => ipcRenderer.send("sotto:call-dock"),
  // The same title bar as the main window.
  platform: process.platform,
  window: {
    minimize: () => ipcRenderer.send("sotto:window", "minimize"),
    toggleMaximize: () => ipcRenderer.send("sotto:window", "toggle-maximize"),
    close: () => ipcRenderer.send("sotto:window", "close"),
    isMaximized: () => ipcRenderer.sendSync("sotto:is-maximized"),
    onMaximized: (fn) => {
      const handler = (_e, on) => fn(!!on);
      ipcRenderer.on("sotto:maximized", handler);
      return () => ipcRenderer.removeListener("sotto:maximized", handler);
    },
  },
});
