// ============================================================
// EZ-SDR Electron Renderer
// WebSocket client + Golden Layout + Canvas spectrum
// ============================================================

let ws = null;
let lastState = null;
let layout = null;

const WS_URL = "ws://127.0.0.1:5347";

// ---- WebSocket ------------------------------------------------
function connectWs() {
  if (ws && ws.readyState <= 1) return;

  ws = new WebSocket(WS_URL);
  ws.onopen = () => {
    document.getElementById("conn-status").textContent = "Connected";
    document.getElementById("conn-status").className = "status connected";
  };
  ws.onclose = () => {
    document.getElementById("conn-status").textContent = "Disconnected";
    document.getElementById("conn-status").className = "status disconnected";
    setTimeout(connectWs, 2000);
  };
  ws.onerror = () => ws.close();
  ws.onmessage = (e) => {
    try {
      lastState = JSON.parse(e.data);
      handleStateUpdate(lastState);
    } catch {}
  };
}

function sendCmd(cmd) {
  if (ws && ws.readyState === 1) ws.send(JSON.stringify(cmd));
}

// ---- State update dispatcher -----------------------------------
function handleStateUpdate(s) {
  document.getElementById("freq-display").textContent =
    (s.center_freq_hz / 1e6).toFixed(3) + " MHz";

  if (s.spectrum && s.waterfall) {
    renderSpectrum(s.spectrum, s.waterfall, s.fft_size);
  }

  updateSourcePanel(s);
  updateSdrPanel(s);
  updateSatellitePanel(s);
  updateAdsbPanel(s);
  updateRecorderPanel(s);
}

// ---- Spectrum Canvas -------------------------------------------
let spectrumCanvas, spectrumCtx;
let waterfallCanvas, waterfallCtx;
let waterfallRows = [];
const WATERFALL_HISTORY = 256;

function initSpectrumCanvas() {
  spectrumCanvas = document.getElementById("spectrum-canvas");
  spectrumCtx = spectrumCanvas.getContext("2d");
  waterfallCanvas = document.getElementById("waterfall-canvas");
  waterfallCtx = waterfallCanvas.getContext("2d");
  resizeSpectrum();
}

function resizeSpectrum() {
  if (!spectrumCanvas) return;
  const container = spectrumCanvas.parentElement;
  if (!container) return;
  spectrumCanvas.width = container.clientWidth;
  spectrumCanvas.height = Math.floor(container.clientHeight * 0.35);
  waterfallCanvas.width = container.clientWidth;
  waterfallCanvas.height = Math.floor(container.clientHeight * 0.65);
}

function renderSpectrum(dbs, waterfallRow, fftSize) {
  if (!spectrumCtx || !waterfallCtx) return;
  const w = spectrumCanvas.width;
  const h = spectrumCanvas.height;
  if (w === 0 || h === 0) return;

  // Spectrum
  spectrumCtx.fillStyle = "#0a0a15";
  spectrumCtx.fillRect(0, 0, w, h);

  const n = dbs.length;
  const barW = Math.max(1, w / n);

  for (let i = 0; i < n; i++) {
    const norm = Math.max(0, Math.min(1, (dbs[i] + 100) / 80));
    const barH = norm * h;
    const x = (i / n) * w;
    const [r, g, b] = spectrumColor(norm);
    spectrumCtx.fillStyle = `rgb(${r},${g},${b})`;
    spectrumCtx.fillRect(x, h - barH, barW + 0.5, barH);
  }

  // dB grid lines
  spectrumCtx.strokeStyle = "rgba(60,60,80,0.5)";
  spectrumCtx.lineWidth = 0.5;
  for (const db of [-80, -60, -40, -20, 0]) {
    const norm = (db + 100) / 80;
    const y = h - norm * h;
    spectrumCtx.beginPath();
    spectrumCtx.moveTo(0, y);
    spectrumCtx.lineTo(w, y);
    spectrumCtx.stroke();
    spectrumCtx.fillStyle = "#556";
    spectrumCtx.font = "9px monospace";
    spectrumCtx.fillText(`${db} dB`, 2, y - 2);
  }

  // Waterfall
  waterfallRows.push(waterfallRow);
  if (waterfallRows.length > WATERFALL_HISTORY) waterfallRows.shift();

  const wh = waterfallCanvas.height;
  const rowH = Math.max(1, wh / WATERFALL_HISTORY);
  const imgData = waterfallCtx.createImageData(w, wh);
  const data = imgData.data;

  for (let row = 0; row < waterfallRows.length; row++) {
    const rowData = waterfallRows[row];
    const yStart = Math.floor(row * rowH);
    const yEnd = Math.floor((row + 1) * rowH);
    const srcLen = rowData.length / 4;
    for (let y = yStart; y < yEnd && y < wh; y++) {
      for (let x = 0; x < w; x++) {
        const srcIdx = Math.floor((x / w) * srcLen) * 4;
        const dstIdx = (y * w + x) * 4;
        data[dstIdx] = rowData[srcIdx] || 0;
        data[dstIdx + 1] = rowData[srcIdx + 1] || 0;
        data[dstIdx + 2] = rowData[srcIdx + 2] || 0;
        data[dstIdx + 3] = 255;
      }
    }
  }

  waterfallCtx.putImageData(imgData, 0, 0);
}

function spectrumColor(norm) {
  if (norm < 0.25) {
    const t = norm / 0.25;
    return [t * 80 | 0, 0, 128 + t * 127 | 0];
  } else if (norm < 0.5) {
    const t = (norm - 0.25) / 0.25;
    return [0, t * 200 | 0, 255 - t * 155 | 0];
  } else if (norm < 0.75) {
    const t = (norm - 0.5) / 0.25;
    return [t * 255 | 0, 200 + t * 55 | 0, 100 - t * 100 | 0];
  } else {
    const t = (norm - 0.75) / 0.25;
    return [255, 255 - t * 100 | 0, 0];
  }
}

// ---- Panel updaters -------------------------------------------
function updateSourcePanel(s) {
  const el = document.getElementById("source-panel");
  if (!el) return;
  el.innerHTML = `
    <div class="panel-header">RTL-SDR V4 Source</div>
    <div style="padding:8px">
      <div style="margin-bottom:8px">
        <span style="color:${s.source.running ? '#00cc88' : '#cc4444'}">
          ● ${s.source.status}
        </span>
      </div>
      <label style="color:#88a;font-size:11px">Frequency</label>
      <input type="number" id="inp-freq" value="${s.center_freq_hz}" step="1000"
        style="margin-bottom:8px">
      <label style="color:#88a;font-size:11px">Gain: ${s.source.gain_db.toFixed(1)} dB</label>
      <input type="range" id="inp-gain" min="0" max="49.6" step="0.1"
        value="${s.source.gain_db}">
      <label style="color:#88a;font-size:11px">Sample Rate</label>
      <input type="number" id="inp-rate" value="${s.sample_rate_hz}" step="100000"
        style="margin-bottom:8px">
      <div style="margin-top:8px">
        <label><input type="checkbox" id="inp-bias" ${s.source.bias_tee ? 'checked' : ''}>
          Bias Tee (4.5V)</label>
      </div>
      <div style="display:flex;gap:6px;margin-top:8px">
        <button class="${s.source.running ? '' : 'primary'}"
          onclick="sendCmd({action:'start_source'})">Start</button>
        <button class="${s.source.running ? 'primary' : ''}"
          onclick="sendCmd({action:'stop_source'})">Stop</button>
      </div>
    </div>`;

  const freqInp = document.getElementById("inp-freq");
  if (freqInp) freqInp.onchange = () => sendCmd({ action: "tune", hz: parseInt(freqInp.value) });
  const gainInp = document.getElementById("inp-gain");
  if (gainInp) gainInp.oninput = () => sendCmd({ action: "set_gain", db: parseFloat(gainInp.value) });
  const rateInp = document.getElementById("inp-rate");
  if (rateInp) rateInp.onchange = () => sendCmd({ action: "set_sample_rate", rate: parseInt(rateInp.value) });
  const biasInp = document.getElementById("inp-bias");
  if (biasInp) biasInp.onchange = () => sendCmd({ action: "toggle_bias_tee", on: biasInp.checked });
}

function updateSdrPanel(s) {
  const el = document.getElementById("sdr-panel");
  if (!el) return;
  const modes = ["RAW", "AM", "FM", "WFM", "LSB", "USB"];
  el.innerHTML = `
    <div class="panel-header">SDR Receiver</div>
    <div style="padding:8px">
      <div style="display:flex;gap:4px;margin-bottom:8px">
        ${modes.map(m => `<button class="${s.demod_mode === m ? 'active' : ''}"
          onclick="sendCmd({action:'set_demod',mode:'${m}'})">${m}</button>`).join("")}
      </div>
      <div id="sdr-info" style="color:#667;font-size:11px">
        Demod: ${s.demod_mode} | ${s.source.running ? 'Running' : 'Stopped'}
      </div>
    </div>`;
}

function updateSatellitePanel(s) {
  const el = document.getElementById("sat-panel");
  if (!el) return;
  el.innerHTML = `
    <div class="panel-header">Satellite Tracking</div>
    <div style="padding:8px">
      <div style="margin-bottom:8px">
        <label style="color:#88a;font-size:11px">Observer</label>
        <div style="display:flex;gap:4px">
          <input type="number" id="obs-lat" value="51.5" step="0.1" placeholder="Lat"
            style="width:50%">
          <input type="number" id="obs-lon" value="-0.1" step="0.1" placeholder="Lon"
            style="width:50%">
        </div>
      </div>
      <div style="margin-bottom:8px">
        <label style="color:#88a;font-size:11px">Selected: ${s.selected_satellite || 'None'}</label>
      </div>
      <table>
        <tr><th>Satellite</th><th>AOS</th><th>LOS</th><th>MaxEl</th><th></th></tr>
        ${(s.passes || []).map(p => `
          <tr class="${s.selected_satellite === p.satellite ? 'selected' : ''}">
            <td>${p.satellite}</td>
            <td>${p.aos}</td>
            <td>${p.los}</td>
            <td>${p.max_elevation.toFixed(0)}°</td>
            <td><button onclick="sendCmd({action:'select_satellite',name:'${p.satellite}'});
              sendCmd({action:'tune',hz:${p.frequency_hz}})">
              ${s.selected_satellite === p.satellite ? 'Selected' : 'Select'}</button></td>
          </tr>
        `).join("")}
      </table>
    </div>`;
}

function updateAdsbPanel(s) {
  const el = document.getElementById("adsb-panel");
  if (!el) return;
  el.innerHTML = `
    <div class="panel-header">ADS-B / Mode S (1090 MHz)</div>
    <div style="padding:8px">
      <div style="display:flex;gap:6px;margin-bottom:8px">
        <button class="${s.adsb_running ? 'primary' : ''}"
          onclick="sendCmd({action:'start_adsb'})">Start</button>
        <button class="${!s.adsb_running ? '' : 'primary'}"
          onclick="sendCmd({action:'stop_adsb'})">Stop</button>
        <span style="color:#667;font-size:11px">
          Aircraft: ${(s.aircraft || []).length}
        </span>
      </div>
      <div class="map-container" style="height:150px;margin-bottom:8px">
        Aircraft Map
      </div>
      <div style="max-height:200px;overflow-y:auto">
        <table>
          <tr><th>ICAO</th><th>Callsign</th><th>Alt</th><th>Spd</th><th>Lat</th><th>Lon</th></tr>
          ${(s.aircraft || []).map(a => `
            <tr>
              <td>${a.icao.toString(16).toUpperCase().padStart(6,'0')}</td>
              <td>${a.callsign}</td>
              <td>${a.altitude}</td>
              <td>${a.speed}</td>
              <td>${a.lat.toFixed(2)}</td>
              <td>${a.lon.toFixed(2)}</td>
            </tr>
          `).join("")}
        </table>
      </div>
    </div>`;
}

function updateRecorderPanel(s) {
  const el = document.getElementById("rec-panel");
  if (!el) return;
  el.innerHTML = `
    <div class="panel-header">Recorder</div>
    <div style="padding:8px">
      ${s.recording ? `
        <div style="margin-bottom:8px">
          <span class="rec-indicator"></span>
          <span style="color:#ff4444;font-weight:600">RECORDING</span>
          <span style="color:#667;font-size:11px;margin-left:8px">
            ${s.record_secs}s | ${(s.record_bytes / 1048576).toFixed(1)} MB
          </span>
        </div>
        <button class="primary" onclick="sendCmd({action:'stop_recording'})">Stop Recording</button>
      ` : `
        <button class="primary" onclick="sendCmd({action:'start_recording'})">Start Recording</button>
      `}
      <div style="margin-top:12px">
        <div style="color:#88a;font-size:11px">Output: ./recordings/</div>
      </div>
    </div>`;
}

// ---- Bookmarks panel ------------------------------------------
function renderBookmarksPanel() {
  const el = document.getElementById("bm-panel");
  if (!el || !lastState) return;

  const categories = {};
  (lastState.bookmarks || []).forEach(bm => {
    if (!categories[bm.category]) categories[bm.category] = [];
    categories[bm.category].push(bm);
  });

  let html = `<div class="panel-header">Bookmarks</div><div class="panel-content">`;
  for (const [cat, items] of Object.entries(categories)) {
    html += `<div class="bookmark-category">▾ ${cat}</div>`;
    items.forEach(bm => {
      html += `<div class="bookmark-item" onclick="sendCmd({action:'tune_bookmark',hz:${bm.frequency_hz}})">
        <span class="name">${bm.name}</span>
        <span class="freq">${(bm.frequency_hz / 1e6).toFixed(3)} MHz</span>
      </div>`;
    });
  }
  html += `</div>`;
  el.innerHTML = html;
}

// ---- AI panel -------------------------------------------------
function renderAiPanel() {
  return `
    <div class="panel-header">AI Agent</div>
    <div style="padding:8px">
      <div style="margin-bottom:6px">
        <input type="password" placeholder="OpenRouter API Key..." id="ai-key"
          style="margin-bottom:6px">
      </div>
      <div class="chat-messages" id="ai-chat"></div>
      <div class="chat-input-row">
        <input type="text" id="ai-input" placeholder="Ask the AI anything...">
        <button onclick="aiSend()">Send</button>
      </div>
    </div>`;
}

function aiSend() {
  const inp = document.getElementById("ai-input");
  const chat = document.getElementById("ai-chat");
  if (!inp || !chat || !inp.value.trim()) return;
  chat.innerHTML += `<div class="chat-msg user">You: ${inp.value}</div>`;
  chat.innerHTML += `<div class="chat-msg ai">AI: (AI integration coming soon — backend handles this)</div>`;
  inp.value = "";
  chat.scrollTop = chat.scrollHeight;
}

// ---- Settings panel -------------------------------------------
function renderSettingsPanel() {
  return `
    <div class="panel-header">Settings</div>
    <div style="padding:8px">
      <label style="color:#88a;font-size:11px">MQTT Broker</label>
      <input type="text" value="localhost:1883" style="margin-bottom:8px">
      <label style="color:#88a;font-size:11px">MQTT Topic Prefix</label>
      <input type="text" value="ezsdr" style="margin-bottom:8px">
      <label><input type="checkbox"> Web Remote</label>
      <div style="margin-top:8px">
        <button>Save Settings</button>
      </div>
    </div>`;
}

// ---- Golden Layout setup --------------------------------------
function initLayout() {
  const config = {
    settings: {
      showMaximiseIcon: true,
      showCloseIcon: true,
      constrainDragToContainer: true,
    },
    dimensions: {
      borderWidth: 4,
      barHeight: 28,
    },
    content: [
      {
        type: "row",
        content: [
          {
            type: "column",
            width: 22,
            content: [
              {
                type: "stack",
                content: [
                  { type: "component", componentName: "Bookmarks", componentState: {}, title: "Bookmarks" },
                  { type: "component", componentName: "Scheduler", componentState: {}, title: "Scheduler" },
                  { type: "component", componentName: "Settings", componentState: {}, title: "Settings" },
                ],
              },
            ],
          },
          {
            type: "column",
            content: [
              {
                type: "row",
                height: 65,
                content: [
                  { type: "component", componentName: "Spectrum", componentState: {}, title: "Spectrum" },
                ],
              },
              {
                type: "row",
                height: 35,
                content: [
                  {
                    type: "stack",
                    content: [
                      { type: "component", componentName: "SDR", componentState: {}, title: "SDR" },
                      { type: "component", componentName: "Satellite", componentState: {}, title: "Satellite" },
                      { type: "component", componentName: "ADS-B", componentState: {}, title: "ADS-B" },
                      { type: "component", componentName: "Recorder", componentState: {}, title: "Recorder" },
                      { type: "component", componentName: "AI Agent", componentState: {}, title: "AI Agent" },
                    ],
                  },
                ],
              },
            ],
          },
        ],
      },
    ],
  };

  layout = new GoldenLayout(config, document.getElementById("golden-container"));

  layout.registerComponent("Spectrum", (container) => {
    container.getElement().html(`
      <div style="width:100%;height:100%;display:flex;flex-direction:column">
        <canvas id="spectrum-canvas" style="flex:0.35"></canvas>
        <canvas id="waterfall-canvas" style="flex:0.65"></canvas>
      </div>`);
    container.on("open", () => setTimeout(initSpectrumCanvas, 100));
    container.on("resize", () => setTimeout(resizeSpectrum, 50));
  });

  layout.registerComponent("SDR", (container) => {
    container.getElement().html('<div id="sdr-panel" class="panel-content"></div>');
  });

  layout.registerComponent("Satellite", (container) => {
    container.getElement().html('<div id="sat-panel" class="panel-content"></div>');
  });

  layout.registerComponent("ADS-B", (container) => {
    container.getElement().html('<div id="adsb-panel" class="panel-content"></div>');
  });

  layout.registerComponent("Recorder", (container) => {
    container.getElement().html('<div id="rec-panel" class="panel-content"></div>');
  });

  layout.registerComponent("AI Agent", (container) => {
    container.getElement().html(renderAiPanel());
  });

  layout.registerComponent("Bookmarks", (container) => {
    container.getElement().html('<div id="bm-panel" class="panel-content"></div>');
    container.on("open", () => setTimeout(renderBookmarksPanel, 200));
  });

  layout.registerComponent("Scheduler", (container) => {
    container.getElement().html(`
      <div class="panel-header">Scheduler</div>
      <div class="panel-content">
        <button onclick="sendCmd({action:'refresh_tles'})">Refresh TLEs</button>
        <div style="margin-top:8px;color:#667;font-size:11px">
          Pass prediction runs automatically
        </div>
      </div>`);
  });

  layout.registerComponent("Settings", (container) => {
    container.getElement().html(renderSettingsPanel());
  });

  layout.init();

  window.addEventListener("resize", () => layout.updateSize());
}

// ---- Init -----------------------------------------------------
document.addEventListener("DOMContentLoaded", () => {
  initLayout();
  connectWs();

  // Re-render bookmarks when state arrives
  setInterval(() => {
    if (lastState) renderBookmarksPanel();
  }, 2000);
});
