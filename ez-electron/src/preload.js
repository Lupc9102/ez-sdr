const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("ezSdr", {
  getBackendUrl: () => ipcRenderer.invoke("get-backend-url"),
  isBackendReady: () => ipcRenderer.invoke("is-backend-ready"),
  onBackendReady: (cb) => ipcRenderer.on("backend-ready", (_e, url) => cb(url)),
});
