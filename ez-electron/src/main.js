const { app, BrowserWindow, ipcMain } = require("electron");
const path = require("path");
const { spawn } = require("child_process");

app.commandLine.appendSwitch("disable-gpu");
app.commandLine.appendSwitch("disable-software-rasterizer");
if (process.platform === "linux") {
  app.commandLine.appendSwitch("ozone-platform-hint", "auto");
}

let mainWindow = null;
let backendProcess = null;
let backendReady = false;

const BACKEND_PORT = 5347;
const BACKEND_WS = `ws://127.0.0.1:${BACKEND_PORT}`;

function findBackendBinary() {
  const isPackaged = app.isPackaged;
  const baseDir = isPackaged ? process.resourcesPath : path.join(__dirname, "..", "..");

  const candidates = [
    path.join(baseDir, "target", "release", "ez-backend"),
    path.join(baseDir, "target", "debug", "ez-backend"),
    path.join(baseDir, "ez-backend", "target", "release", "ez-backend"),
  ];

  for (const p of candidates) {
    try {
      require("fs").accessSync(p, require("fs").constants.X_OK);
      return p;
    } catch {}
  }
  return null;
}

function startBackend() {
  const bin = findBackendBinary();
  if (!bin) {
    console.error("[electron] ez-backend binary not found — run 'cargo build -p ez-backend' first");
    console.error("[electron] Starting in frontend-only mode (no SDR data)");
    return false;
  }

  console.log(`[electron] starting backend: ${bin}`);
  backendProcess = spawn(bin, [], {
    env: { ...process.env, EZ_PORT: String(BACKEND_PORT) },
    stdio: ["ignore", "pipe", "pipe"],
  });

  backendProcess.stdout.on("data", (d) => {
    const line = d.toString().trim();
    if (line) console.log(`[backend] ${line}`);
    if (line.includes("listening")) {
      backendReady = true;
      if (mainWindow) mainWindow.webContents.send("backend-ready", BACKEND_WS);
    }
  });

  backendProcess.stderr.on("data", (d) => {
    const line = d.toString().trim();
    if (line) console.error(`[backend:err] ${line}`);
  });

  backendProcess.on("exit", (code) => {
    console.log(`[electron] backend exited with code ${code}`);
    backendReady = false;
    backendProcess = null;
  });

  return true;
}

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    backgroundColor: "#1a1a2e",
    title: "EZ-SDR",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  mainWindow.loadFile(path.join(__dirname, "index.html"));

  mainWindow.on("closed", () => {
    mainWindow = null;
  });

  mainWindow.webContents.on("did-finish-load", () => {
    if (backendReady) {
      mainWindow.webContents.send("backend-ready", BACKEND_WS);
    }
  });
}

app.whenReady().then(() => {
  startBackend();
  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  if (backendProcess) {
    backendProcess.kill("SIGTERM");
    backendProcess = null;
  }
  app.quit();
});

ipcMain.handle("get-backend-url", () => BACKEND_WS);
ipcMain.handle("is-backend-ready", () => backendReady);
