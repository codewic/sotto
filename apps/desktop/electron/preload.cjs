// The only bridge between the sandboxed page and the outside world.
const { contextBridge, ipcRenderer, webFrame } = require("electron");

contextBridge.exposeInMainWorld("sotto", {
  request: (method, path, body, headers) => ipcRenderer.invoke("sotto:request", method, path, body, headers),
  onEvent: (fn) => {
    const handler = (_e, data) => fn(data);
    ipcRenderer.on("sotto:event", handler);
    return () => ipcRenderer.removeListener("sotto:event", handler);
  },
  saveDialog: (name) => ipcRenderer.invoke("sotto:save-dialog", name),
  setContentProtection: (on) => ipcRenderer.invoke("sotto:content-protection", on),
  // Interface size from Appearance. Native zoom keeps layout and pointer positions exact.
  setZoom: (factor) => {
    const f = Number(factor);
    if (Number.isFinite(f) && f >= 0.8 && f <= 1.5) webFrame.setZoomFactor(f);
  },
  // Open (or bring forward) the call window for one call.
  openCall: (id) => ipcRenderer.invoke("sotto:open-call", id),
  // Lets native parts (dialogs, the window behind the page) follow Sotto's theme.
  setAppearance: (a) => ipcRenderer.invoke("sotto:appearance", a),
});
