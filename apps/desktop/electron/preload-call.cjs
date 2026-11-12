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
});
