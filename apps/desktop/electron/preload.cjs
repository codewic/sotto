// The only bridge between the sandboxed page and the outside world.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("sotto", {
  request: (method, path, body, headers) => ipcRenderer.invoke("sotto:request", method, path, body, headers),
  onEvent: (fn) => {
    const handler = (_e, data) => fn(data);
    ipcRenderer.on("sotto:event", handler);
    return () => ipcRenderer.removeListener("sotto:event", handler);
  },
  saveDialog: (name) => ipcRenderer.invoke("sotto:save-dialog", name),
  setContentProtection: (on) => ipcRenderer.invoke("sotto:content-protection", on),
});
