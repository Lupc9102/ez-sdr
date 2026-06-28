const PRESETS = {
    spectrum: {
        color: '#2ecc71',
        lineWidth: 1.5,
        gridColor: '#444',
        labelColor: '#888',
        padding: { top: 20, right: 20, bottom: 25, left: 45 },
    },
};

let ws;
let state = null;
let waterfallCanvas, waterfallCtx;
let spectrumCanvas, spectrumCtx;
let peakHold = false;
let average = 0.5;
let waterfallRows = [];

// ——— Golden Layout ———
const config = {
    settings: { showPopoutIcon: false, showMaximiseIcon: false },
    content: [
        {
            type: 'row',
            content: [
                { type: 'column', width: 60, content: [
                    { type: 'component', componentName: 'Spectrum', componentState: { id: 'spectrum' }, height: 70 },
                    { type: 'component', componentName: 'Waterfall', componentState: { id: 'waterfall' }, height: 30 },
                ]},
                { type: 'column', width: 40, content: [
                    { type: 'row', content: [
                        { type: 'component', componentName: 'SDR Controls', componentState: { id: 'sdr' }, width: 50 },
                        { type: 'component', componentName: 'Source', componentState: { id: 'source' }, width: 50 },
                    ]},
                    { type: 'row', content: [
                        { type: 'component', componentName: 'Satellite', componentState: { id: 'satellite' }, width: 50 },
                        { type: 'component', componentName: 'ADS-B', componentState: { id: 'adsb' }, width: 50 },
                    ]},
                ]},
            ],
        },
        {
            type: 'column',
            content: [
                { type: 'row', content: [
                    { type: 'component', componentName: 'Recorder', componentState: { id: 'recorder' }, width: 33 },
                    { type: 'component', componentName: 'AI Agent', componentState: { id: 'ai' }, width: 34 },
                    { type: 'component', componentName: 'Bookmarks', componentState: { id: 'bookmarks' }, width: 33 },
                ]},
                { type: 'row', content: [
                    { type: 'component', componentName: 'Scheduler', componentState: { id: 'scheduler' }, width: 50 },
                    { type: 'component', componentName: 'Settings', componentState: { id: 'settings' }, width: 50 },
                ]},
            ],
        },
    ],
};

const panelHTML = {
    spectrum: () => `
        <div class="panel-controls">
            <label>FFT <select id="fftSize"><option>512</option><option>1024</option><option selected>2048</option><option>4096</option></select></label>
            <label>Window <select id="windowFunc"><option>Hann</option><option>Hamming</option><option>Blackman</option><option>Blackman-Harris</option></select></label>
            <label><input type="checkbox" id="peakHold"> Peak Hold</label>
            <label>Avg <input type="range" id="avgAlpha" min="0.05" max="0.95" step="0.05" value="0.5" style="width:60px"></label>
        </div>
        <div class="panel-body"><canvas id="spectrumCanvas" class="spectrum-canvas"></canvas></div>
    `,
    waterfall: () => `
        <div class="panel-body"><canvas id="waterfallCanvas" class="waterfall-canvas"></canvas></div>
    `,
    sdr: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Demodulation</div>
                <div class="mode-grid">
                    <button class="mode-btn" data-mode="RAW">RAW</button>
                    <button class="mode-btn active" data-mode="FM">FM</button>
                    <button class="mode-btn" data-mode="WFM">WFM</button>
                    <button class="mode-btn" data-mode="AM">AM</button>
                    <button class="mode-btn" data-mode="LSB">LSB</button>
                    <button class="mode-btn" data-mode="USB">USB</button>
                </div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Filter Bandwidth <span id="bwValue">12.0 kHz</span></div>
                <input type="range" id="filterBw" min="100" max="250000" step="100" value="12000">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Squelch <span id="sqlValue">-50 dB</span></div>
                <input type="range" id="squelch" min="-120" max="0" step="1" value="-50">
            </div>
        </div>
    `,
    source: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Frequency</div>
                <div class="freq-input-group">
                    <input type="number" id="freqMHz" class="freq-input" value="100.0" step="0.1" min="24" max="1766">
                    <span class="freq-unit">MHz</span>
                </div>
                <div class="freq-input-group" style="margin-top:4px">
                    <input type="number" id="freqKHz" class="freq-input freq-small" value="0" step="1" min="0" max="999">
                    <span class="freq-unit">kHz</span>
                </div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Sample Rate <span id="srValue">2.048 MS/s</span></div>
                <select id="sampleRate">
                    <option value="250000">250 kS/s</option>
                    <option value="1024000">1.024 MS/s</option>
                    <option value="1536000">1.536 MS/s</option>
                    <option value="1792000">1.792 MS/s</option>
                    <option value="1920000">1.920 MS/s</option>
                    <option value="2048000" selected>2.048 MS/s</option>
                    <option value="2160000">2.160 MS/s</option>
                    <option value="2400000">2.400 MS/s</option>
                    <option value="2560000">2.560 MS/s</option>
                </select>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Gain <span id="gainValue">40.0 dB</span></div>
                <input type="range" id="gainSlider" min="0" max="49.6" step="0.1" value="40">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">PPM Correction <span id="ppmValue">0</span></div>
                <input type="range" id="ppmSlider" min="-100" max="100" step="1" value="0">
            </div>
            <div class="sdr-section toggle-row">
                <label><input type="checkbox" id="biasTee"> Bias Tee (V)</label>
                <label><input type="checkbox" id="directSampling"> Direct Sampling</label>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Temperature</div>
                <div class="temp-display" id="tempDisplay">—</div>
            </div>
            <button id="applySource" class="btn-apply">Apply Settings</button>
        </div>
    `,
    satellite: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Observer Location</div>
                <div class="obs-row">
                    <label>Lat <input type="number" id="obsLat" value="51.5" step="0.01" style="width:70px"></label>
                    <label>Lon <input type="number" id="obsLon" value="-0.1" step="0.01" style="width:70px"></label>
                </div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Signal Strength</div>
                <div class="signal-bar-wrap"><div class="signal-bar" id="signalBar"></div></div>
                <span class="signal-dB" id="signalDb">-120.0 dB</span>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Doppler Shift</div>
                <div class="doppler-display" id="dopplerDisplay">0 Hz</div>
            </div>
            <div class="sdr-section toggle-row">
                <label><input type="checkbox" id="autoRecord" checked> Auto-record</label>
                <label><input type="checkbox" id="autoTune" checked> Auto-tune</label>
                <label><input type="checkbox" id="liveDecode"> Live-decode</label>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Upcoming Passes</div>
                <table class="pass-table" id="passTable">
                    <thead><tr><th>SAT</th><th>AOS</th><th>LOS</th><th>Max El</th><th></th></tr></thead>
                    <tbody id="passBody"></tbody>
                </table>
            </div>
        </div>
    `,
    adsb: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-row-between">
                    <div>
                        <div class="sdr-label">Messages</div>
                        <span id="adsbMsgCount" class="big-number">0</span>
                    </div>
                    <div>
                        <div class="sdr-label">Rate</div>
                        <span id="adsbMsgRate" class="big-number">0</span><span class="unit">/s</span>
                    </div>
                    <div>
                        <div class="sdr-label">Aircraft</div>
                        <span id="adsbAircraftCount" class="big-number">0</span>
                    </div>
                </div>
            </div>
            <div class="sdr-section">
                <button id="adsbToggle" class="btn-apply" style="width:100%">Start ADS-B</button>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Aircraft</div>
                <div class="aircraft-table-wrap">
                    <table class="aircraft-table" id="aircraftTable">
                        <thead><tr><th>ICAO</th><th>Call</th><th>Alt</th><th>Spd</th><th>HDG</th><th>Lat</th><th>Lon</th><th>Age</th></tr></thead>
                        <tbody id="aircraftBody"></tbody>
                    </table>
                </div>
            </div>
            <div class="sdr-section">
                <div id="adsbMap" class="adsb-map"></div>
            </div>
        </div>
    `,
    recorder: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Status</div>
                <div id="recStatus" class="rec-indicator stopped">STOPPED</div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Format</div>
                <div class="mode-grid">
                    <button class="mode-btn active" data-fmt="IQ">IQ</button>
                    <button class="mode-btn" data-fmt="WAV">WAV</button>
                </div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Output Directory</div>
                <div id="recDir" class="path-display">./recordings</div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Disk Space</div>
                <div class="progress-bar-wrap"><div class="progress-bar" id="recDiskBar" style="width:0%"></div></div>
                <span id="recDiskText">— free</span>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Duration</div>
                <div id="recDuration" class="time-display">00:00</div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Size</div>
                <span id="recSize" class="big-number">0</span><span class="unit"> KB</span>
            </div>
            <div class="sdr-section">
                <button id="recToggle" class="btn-apply" style="width:100%">Start Recording</button>
            </div>
        </div>
    `,
    ai: () => `
        <div class="panel-body ai-panel">
            <div class="sdr-section">
                <label class="sdr-label">OpenRouter API Key</label>
                <input type="password" id="apiKey" class="api-key-input" placeholder="sk-or-...">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Model: <span id="aiModelDisplay">claude-3-haiku-20240307</span></div>
            </div>
            <div class="ai-chat" id="aiChat">
                <div class="ai-msg system">Ready. Ask me about signal analysis, pass predictions, or SDR settings.</div>
            </div>
            <div class="ai-input-row">
                <input type="text" id="aiInput" class="ai-input" placeholder="Ask about signals...">
                <button id="aiSend" class="btn-apply">Send</button>
            </div>
        </div>
    `,
    bookmarks: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <input type="text" id="bmSearch" placeholder="Search bookmarks..." style="width:100%;padding:6px;background:#1e1e2e;color:#fff;border:1px solid #444;border-radius:4px">
            </div>
            <div class="bookmarks-list" id="bmList"></div>
            <div class="sdr-section" style="margin-top:8px">
                <button id="bmAddCustom" class="btn-apply" style="width:100%">+ Add Custom Bookmark</button>
            </div>
        </div>
    `,
    scheduler: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Active Jobs</div>
                <div id="schedulerJobs" class="scheduler-jobs">No active scheduled tasks</div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Next Pass</div>
                <div id="nextPassInfo" class="next-pass">—</div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">System</div>
                <div class="system-stats">
                    <div>Uptime: <span id="sysUptime">—</span></div>
                    <div>Memory: <span id="sysMem">—</span></div>
                    <div>Backend: <span id="sysBackend">Disconnected</span></div>
                </div>
            </div>
        </div>
    `,
    settings: () => `
        <div class="panel-body">
            <div class="sdr-section">
                <div class="sdr-label">Display</div>
                <div class="toggle-row">
                    <label><input type="checkbox" id="cfgPeakHold"> Peak Hold</label>
                    <label><input type="checkbox" id="cfgWaterfall" checked> Waterfall</label>
                    <label><input type="checkbox" id="cfgMinimap"> Minimap</label>
                </div>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Audio Output</div>
                <select id="cfgAudioOut"><option>Default</option></select>
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Default Save Path</div>
                <input type="text" id="cfgSavePath" value="./recordings" style="width:100%;padding:4px;background:#1e1e2e;color:#fff;border:1px solid #444;border-radius:4px">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">MQTT Broker</div>
                <input type="text" id="cfgMqtt" placeholder="mqtt://localhost:1883" style="width:100%;padding:4px;background:#1e1e2e;color:#fff;border:1px solid #444;border-radius:4px">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">MQTT Topic</div>
                <input type="text" id="cfgMqttTopic" value="ez-sdr/spectrum" style="width:100%;padding:4px;background:#1e1e2e;color:#fff;border:1px solid #444;border-radius:4px">
            </div>
            <div class="sdr-section">
                <div class="sdr-label">Web Remote Port</div>
                <input type="number" id="cfgWebPort" value="8080" style="width:100%;padding:4px;background:#1e1e2e;color:#fff;border:1px solid #444;border-radius:4px">
            </div>
            <div class="sdr-section">
                <button id="cfgReset" class="btn-apply" style="width:100%">Reset Defaults</button>
            </div>
        </div>
    `,
};

const panelRenderers = {};

function registerRenderers() {
    panelRenderers.spectrum = (container, state) => {
        container.innerHTML = panelHTML.spectrum();
        spectrumCanvas = container.querySelector('#spectrumCanvas');
        if (!spectrumCanvas) return;
        spectrumCtx = spectrumCanvas.getContext('2d');
        const resize = () => {
            const rect = container.querySelector('.panel-body').getBoundingClientRect();
            spectrumCanvas.width = rect.width;
            spectrumCanvas.height = rect.height;
        };
        resize();
        new ResizeObserver(resize).observe(container.getElement());
        const pe = container.getElement();
        if (pe) pe.addEventListener('resize', resize);

        container.getElement().querySelector('#peakHold')?.addEventListener('change', e => { peakHold = e.target.checked; });
        container.getElement().querySelector('#avgAlpha')?.addEventListener('input', e => { average = parseFloat(e.target.value); });
    };

    panelRenderers.waterfall = (container, state) => {
        container.innerHTML = panelHTML.waterfall();
        waterfallCanvas = container.querySelector('#waterfallCanvas');
        if (!waterfallCanvas) return;
        waterfallCtx = waterfallCanvas.getContext('2d');
        waterfallRows = [];
        const resize = () => {
            const rect = container.querySelector('.panel-body').getBoundingClientRect();
            waterfallCanvas.width = rect.width;
            waterfallCanvas.height = rect.height;
        };
        resize();
        new ResizeObserver(resize).observe(container.getElement());
        const pe = container.getElement();
        if (pe) pe.addEventListener('resize', resize);
    };

    panelRenderers.sdr = (container, state) => {
        container.innerHTML = panelHTML.sdr();
        const el = container.getElement();
        el.querySelectorAll('.mode-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                el.querySelectorAll('.mode-btn').forEach(b => b.classList.remove('active'));
                btn.classList.add('active');
                send({ cmd: 'set_demod', mode: btn.dataset.mode });
            });
        });
        el.querySelector('#filterBw')?.addEventListener('input', e => {
            el.querySelector('#bwValue').textContent = (e.target.value / 1000).toFixed(1) + ' kHz';
        });
        el.querySelector('#filterBw')?.addEventListener('change', e => {
            send({ cmd: 'set_filter_bw', bw: parseInt(e.target.value) });
        });
        el.querySelector('#squelch')?.addEventListener('input', e => {
            el.querySelector('#sqlValue').textContent = e.target.value + ' dB';
        });
        el.querySelector('#squelch')?.addEventListener('change', e => {
            send({ cmd: 'set_squelch', level: parseFloat(e.target.value) });
        });
    };

    panelRenderers.source = (container, state) => {
        container.innerHTML = panelHTML.source();
        const el = container.getElement();
        el.querySelector('#applySource')?.addEventListener('click', () => {
            const mhz = parseFloat(el.querySelector('#freqMHz').value) || 100;
            const khz = parseFloat(el.querySelector('#freqKHz').value) || 0;
            const freqHz = Math.round((mhz + khz / 1000) * 1e6);
            send({
                cmd: 'set_source',
                frequency_hz: freqHz,
                sample_rate_hz: parseInt(el.querySelector('#sampleRate').value),
                gain_db: parseFloat(el.querySelector('#gainSlider').value),
                bias_tee: el.querySelector('#biasTee').checked,
                ppm_correction: parseInt(el.querySelector('#ppmSlider').value),
                direct_sampling: el.querySelector('#directSampling').checked,
            });
        });
        el.querySelector('#gainSlider')?.addEventListener('input', e => {
            el.querySelector('#gainValue').textContent = parseFloat(e.target.value).toFixed(1) + ' dB';
        });
        el.querySelector('#ppmSlider')?.addEventListener('input', e => {
            el.querySelector('#ppmValue').textContent = e.target.value;
        });
        el.querySelector('#sampleRate')?.addEventListener('change', e => {
            el.querySelector('#srValue').textContent = (parseInt(e.target.value) / 1e6).toFixed(3) + ' MS/s';
        });
    };

    panelRenderers.satellite = (container, state) => {
        container.innerHTML = panelHTML.satellite();
        const el = container.getElement();
        el.querySelector('#autoRecord')?.addEventListener('change', e => {
            send({ cmd: 'set_auto_record', value: e.target.checked });
        });
        el.querySelector('#autoTune')?.addEventListener('change', e => {
            send({ cmd: 'set_auto_tune', value: e.target.checked });
        });
        el.querySelector('#liveDecode')?.addEventListener('change', e => {
            send({ cmd: 'set_live_decode', value: e.target.checked });
        });
        el.querySelector('#obsLat')?.addEventListener('change', e => {
            send({ cmd: 'set_observer', lat: parseFloat(e.target.value), lon: parseFloat(el.querySelector('#obsLon').value) });
        });
        el.querySelector('#obsLon')?.addEventListener('change', e => {
            send({ cmd: 'set_observer', lat: parseFloat(el.querySelector('#obsLat').value), lon: parseFloat(e.target.value) });
        });
    };

    panelRenderers.adsb = (container, state) => {
        container.innerHTML = panelHTML.adsb();
        const el = container.getElement();
        el.querySelector('#adsbToggle')?.addEventListener('click', () => {
            send({ cmd: state?.adsb_running ? 'stop_adsb' : 'start_adsb' });
        });
    };

    panelRenderers.recorder = (container, state) => {
        container.innerHTML = panelHTML.recorder();
        const el = container.getElement();
        el.querySelector('#recToggle')?.addEventListener('click', () => {
            send({ cmd: state?.recording ? 'stop_recording' : 'start_recording' });
        });
        el.querySelectorAll('[data-fmt]').forEach(btn => {
            btn.addEventListener('click', () => {
                el.querySelectorAll('[data-fmt]').forEach(b => b.classList.remove('active'));
                btn.classList.add('active');
            });
        });
    };

    panelRenderers.ai = (container, state) => {
        container.innerHTML = panelHTML.ai();
        const el = container.getElement();
        el.querySelector('#aiSend')?.addEventListener('click', () => {
            const input = el.querySelector('#aiInput');
            if (!input?.value.trim()) return;
            const chat = el.querySelector('#aiChat');
            chat.innerHTML += `<div class="ai-msg user">${escHtml(input.value)}</div>`;
            chat.scrollTop = chat.scrollHeight;
            send({ cmd: 'ai_query', prompt: input.value, api_key: el.querySelector('#apiKey')?.value || '' });
            input.value = '';
        });
        el.querySelector('#aiInput')?.addEventListener('keydown', e => {
            if (e.key === 'Enter') el.querySelector('#aiSend')?.click();
        });
    };

    panelRenderers.bookmarks = (container, state) => {
        container.innerHTML = panelHTML.bookmarks();
        renderBookmarks(container.getElement(), state);
    };

    panelRenderers.scheduler = (container, state) => {
        container.innerHTML = panelHTML.scheduler();
    };

    panelRenderers.settings = (container, state) => {
        container.innerHTML = panelHTML.settings();
        const el = container.getElement();
        el.querySelector('#cfgReset')?.addEventListener('click', () => {
            send({ cmd: 'reset_config' });
        });
    };
}

function renderBookmarks(el, state) {
    const list = el.querySelector('#bmList');
    const search = el.querySelector('#bmSearch')?.value?.toLowerCase() || '';
    const bookmarks = state?.bookmarks || [];
    const grouped = {};
    bookmarks.forEach(bm => {
        if (search && !bm.name.toLowerCase().includes(search) && !bm.category.toLowerCase().includes(search)) return;
        if (!grouped[bm.category]) grouped[bm.category] = [];
        grouped[bm.category].push(bm);
    });
    let html = '';
    Object.keys(grouped).sort().forEach(cat => {
        html += `<div class="bm-category" data-cat="${escAttr(cat)}"><span class="bm-cat-icon">▸</span> <strong>${escHtml(cat)}</strong> <span class="bm-count">${grouped[cat].length}</span></div>`;
        html += `<div class="bm-items" id="bm-${cat.replace(/[^a-z0-9]/gi,'_')}">`;
        grouped[cat].forEach(bm => {
            const freqStr = bm.frequency_hz >= 1e9 ? (bm.frequency_hz/1e9).toFixed(3)+' GHz' : bm.frequency_hz >= 1e6 ? (bm.frequency_hz/1e6).toFixed(3)+' MHz' : (bm.frequency_hz/1e3).toFixed(1)+' kHz';
            html += `<div class="bm-item" data-freq="${bm.frequency_hz}">
                <div class="bm-name">${escHtml(bm.name)}</div>
                <div class="bm-details">${freqStr} ${bm.mode} ${bm.notes ? '· '+escHtml(bm.notes) : ''}</div>
                <button class="btn-tune" data-freq="${bm.frequency_hz}" data-mode="${bm.mode}">Tune</button>
            </div>`;
        });
        html += '</div>';
    });
    list.innerHTML = html;
    list.querySelectorAll('.bm-category').forEach(cat => {
        cat.addEventListener('click', () => {
            cat.classList.toggle('collapsed');
            const items = cat.nextElementSibling;
            if (items) items.style.display = cat.classList.contains('collapsed') ? 'none' : '';
            cat.querySelector('.bm-cat-icon').textContent = cat.classList.contains('collapsed') ? '▸' : '▾';
        });
    });
    list.querySelectorAll('.btn-tune').forEach(btn => {
        btn.addEventListener('click', () => {
            send({ cmd: 'set_source', frequency_hz: parseInt(btn.dataset.freq) });
            send({ cmd: 'set_demod', mode: btn.dataset.mode || 'FM' });
        });
    });
}

function updateUI(s) {
    state = s;
    if (!s) return;

    // Spectrum
    drawSpectrum(s.spectrum, s.peak_hold);

    // Waterfall
    if (s.waterfall && s.waterfall.length > 0) drawWaterfall(s.waterfall, s.fft_size);

    // SDR
    document.querySelectorAll('.mode-btn[data-mode]').forEach(b => {
        b.classList.toggle('active', b.dataset.mode === s.demod_mode);
    });
    const bwEl = document.getElementById('bwValue');
    if (bwEl) bwEl.textContent = (s.filter_bw / 1000).toFixed(1) + ' kHz';
    const sqlEl = document.getElementById('sqlValue');
    if (sqlEl) sqlEl.textContent = s.squelch + ' dB';
    const bwSlider = document.getElementById('filterBw');
    if (bwSlider) bwSlider.value = s.filter_bw;
    const sqlSlider = document.getElementById('squelch');
    if (sqlSlider) sqlSlider.value = s.squelch;

    // Source
    const freqMHz = document.getElementById('freqMHz');
    if (freqMHz) freqMHz.value = (s.source?.frequency_hz / 1e6).toFixed(1);
    const srEl = document.getElementById('srValue');
    if (srEl && s.source) srEl.textContent = (s.source.sample_rate_hz / 1e6).toFixed(3) + ' MS/s';
    const gainVal = document.getElementById('gainValue');
    if (gainVal && s.source) gainVal.textContent = s.source.gain_db.toFixed(1) + ' dB';
    const gainSlider = document.getElementById('gainSlider');
    if (gainSlider && s.source) gainSlider.value = s.source.gain_db;
    const ppmVal = document.getElementById('ppmValue');
    if (ppmVal && s.source) ppmVal.textContent = s.source.ppm_correction;
    const ppmSlider = document.getElementById('ppmSlider');
    if (ppmSlider && s.source) ppmSlider.value = s.source.ppm_correction;
    const biasCb = document.getElementById('biasTee');
    if (biasCb && s.source) biasCb.checked = s.source.bias_tee;
    const dsCb = document.getElementById('directSampling');
    if (dsCb && s.source) dsCb.checked = s.source.direct_sampling;
    const tempDisp = document.getElementById('tempDisplay');
    if (tempDisp && s.source) tempDisp.textContent = s.source.temperature > 0 ? s.source.temperature.toFixed(1) + '°C' : '—';

    // Satellite
    const sigBar = document.getElementById('signalBar');
    if (sigBar) {
        const pct = Math.max(0, Math.min(100, (s.signal_strength + 120) / 80 * 100));
        sigBar.style.width = pct + '%';
        sigBar.style.background = pct > 60 ? '#2ecc71' : pct > 30 ? '#f39c12' : '#e74c3c';
    }
    const sigDb = document.getElementById('signalDb');
    if (sigDb) sigDb.textContent = s.signal_strength.toFixed(1) + ' dB';
    const dopplerDisp = document.getElementById('dopplerDisplay');
    if (dopplerDisp) dopplerDisp.textContent = (s.doppler_hz >= 0 ? '+' : '') + s.doppler_hz.toFixed(0) + ' Hz';
    const autoRecCb = document.getElementById('autoRecord');
    if (autoRecCb) autoRecCb.checked = s.auto_record;
    const autoTuneCb = document.getElementById('autoTune');
    if (autoTuneCb) autoTuneCb.checked = s.auto_tune;
    const liveDecCb = document.getElementById('liveDecode');
    if (liveDecCb) liveDecCb.checked = s.live_decode;
    const obsLatInput = document.getElementById('obsLat');
    if (obsLatInput) obsLatInput.value = s.observer_lat;
    const obsLonInput = document.getElementById('obsLon');
    if (obsLonInput) obsLonInput.value = s.observer_lon;

    // Passes
    const passBody = document.getElementById('passBody');
    if (passBody && s.passes) {
        passBody.innerHTML = s.passes.map(p => `<tr><td>${escHtml(p.satellite)}</td><td>${escHtml(p.aos)}</td><td>${escHtml(p.los)}</td><td>${p.max_elevation.toFixed(0)}°</td><td><button class="btn-tune-pass" data-freq="${p.frequency_hz}">Tune</button></td></tr>`).join('');
        passBody.querySelectorAll('.btn-tune-pass').forEach(b => {
            b.addEventListener('click', () => send({ cmd: 'set_source', frequency_hz: parseInt(b.dataset.freq) }));
        });
    }

    // ADS-B
    const adsbCount = document.getElementById('adsbMsgCount');
    if (adsbCount) adsbCount.textContent = s.total_adsb_messages.toLocaleString();
    const adsbRate = document.getElementById('adsbMsgRate');
    if (adsbRate) adsbRate.textContent = s.msg_rate.toFixed(1);
    const adsbAircraft = document.getElementById('adsbAircraftCount');
    if (adsbAircraft) adsbAircraft.textContent = (s.aircraft || []).length;
    const adsbBtn = document.getElementById('adsbToggle');
    if (adsbBtn) { adsbBtn.textContent = s.adsb_running ? 'Stop ADS-B' : 'Start ADS-B'; adsbBtn.classList.toggle('btn-stop', s.adsb_running); }
    const acBody = document.getElementById('aircraftBody');
    if (acBody && s.aircraft) {
        acBody.innerHTML = s.aircraft.map(a => `<tr><td>${a.icao.toString(16).toUpperCase()}</td><td>${escHtml(a.callsign)}</td><td>${a.altitude}</td><td>${a.speed}</td><td>${a.heading}°</td><td>${a.lat.toFixed(4)}</td><td>${a.lon.toFixed(4)}</td><td>${a.age_secs}s</td></tr>`).join('');
    }

    // Recorder
    const recStatus = document.getElementById('recStatus');
    if (recStatus) {
        recStatus.textContent = s.recording ? 'RECORDING' : 'STOPPED';
        recStatus.className = 'rec-indicator ' + (s.recording ? 'recording' : 'stopped');
    }
    const recDur = document.getElementById('recDuration');
    if (recDur) { const m = Math.floor(s.record_secs/60), sec = s.record_secs%60; recDur.textContent = String(m).padStart(2,'0')+':'+String(sec).padStart(2,'0'); }
    const recSize = document.getElementById('recSize');
    if (recSize) recSize.textContent = (s.record_bytes/1024).toFixed(0);
    const recToggle = document.getElementById('recToggle');
    if (recToggle) recToggle.textContent = s.recording ? 'Stop Recording' : 'Start Recording';

    // Scheduler
    const nextPassEl = document.getElementById('nextPassInfo');
    if (nextPassEl && s.passes && s.passes.length > 0) {
        const next = s.passes[0];
        nextPassEl.textContent = `${next.satellite} @ ${next.aos} (${next.max_elevation.toFixed(0)}°)`;
    }

    // System
    const backendEl = document.getElementById('sysBackend');
    if (backendEl) backendEl.textContent = 'Connected';

    // Bookmarks (re-render if panel exists)
    const bmList = document.getElementById('bmList');
    if (bmList) {
        const container = bmList.closest('.panel-body')?.parentElement;
        if (container) renderBookmarks(container, s);
    }
}

function drawSpectrum(data, peakData) {
    if (!spectrumCanvas || !spectrumCtx || !data || data.length === 0) return;
    const W = spectrumCanvas.width, H = spectrumCanvas.height;
    if (W <= 0 || H <= 0) return;
    const ctx = spectrumCtx;
    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = '#1a1a2e';
    ctx.fillRect(0, 0, W, H);

    const pad = PRESETS.spectrum.padding;
    const plotW = W - pad.left - pad.right;
    const plotH = H - pad.top - pad.bottom;
    const minDb = -120, maxDb = 0;
    const range = maxDb - minDb;

    // Grid lines
    ctx.strokeStyle = PRESETS.spectrum.gridColor;
    ctx.lineWidth = 0.5;
    ctx.font = '10px monospace';
    ctx.fillStyle = PRESETS.spectrum.labelColor;
    ctx.textAlign = 'right';
    for (let db = minDb; db <= maxDb; db += 20) {
        const y = pad.top + plotH * (1 - (db - minDb) / range);
        ctx.beginPath(); ctx.moveTo(pad.left, y); ctx.lineTo(W - pad.right, y); ctx.stroke();
        ctx.fillText(db + ' dB', pad.left - 4, y + 3);
    }

    // X-axis labels
    ctx.textAlign = 'center';
    const binCount = data.length;
    const centerIdx = Math.floor(binCount / 2);
    for (let i = 0; i <= 4; i++) {
        const x = pad.left + (i / 4) * plotW;
        const binIdx = Math.floor(i / 4 * binCount);
        const offset = (binIdx - centerIdx) / binCount;
        const freqKhz = (offset * parseInt(document.getElementById('sampleRate')?.value || 2048000) / 1000);
        ctx.fillText((freqKhz >= 0 ? '+' : '') + freqKhz.toFixed(0) + ' kHz', x, H - 4);
    }

    // Peak hold
    if (peakHold && peakData) {
        ctx.strokeStyle = 'rgba(255,80,80,0.5)';
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (let i = 0; i < binCount; i++) {
            const x = pad.left + (i / binCount) * plotW;
            const y = pad.top + plotH * (1 - (peakData[i] - minDb) / range);
            i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
        }
        ctx.stroke();
    }

    // Spectrum line
    ctx.strokeStyle = PRESETS.spectrum.color;
    ctx.lineWidth = PRESETS.spectrum.lineWidth;
    ctx.beginPath();
    for (let i = 0; i < binCount; i++) {
        const x = pad.left + (i / binCount) * plotW;
        const db = minDb + Math.max(0, Math.min(1, (data[i] - minDb) / range)) * range;
        const y = pad.top + plotH * (1 - (db - minDb) / range);
        i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill under
    ctx.lineTo(pad.left + plotW, pad.top + plotH);
    ctx.lineTo(pad.left, pad.top + plotH);
    ctx.closePath();
    ctx.fillStyle = 'rgba(46,204,113,0.1)';
    ctx.fill();
}

function drawWaterfall(row, fftSize) {
    if (!waterfallCanvas || !waterfallCtx || !row || row.length === 0) return;
    const W = waterfallCanvas.width, H = waterfallCanvas.height;
    if (W <= 0 || H <= 0) return;
    const ctx = waterfallCtx;
    waterfallRows.push(row);
    const maxRows = H;
    if (waterfallRows.length > maxRows) waterfallRows.shift();
    ctx.clearRect(0, 0, W, H);
    const rowH = Math.max(1, H / maxRows);
    const startRow = Math.max(0, maxRows - waterfallRows.length);
    for (let r = 0; r < waterfallRows.length; r++) {
        const rowData = waterfallRows[r];
        const binCount = rowData.length / 4;
        const imageData = ctx.createImageData(W, 1);
        for (let x = 0; x < W; x++) {
            const bin = Math.min(binCount - 1, Math.floor((x / W) * binCount));
            const idx = bin * 4;
            imageData.data[x*4+0] = rowData[idx+0];
            imageData.data[x*4+1] = rowData[idx+1];
            imageData.data[x*4+2] = rowData[idx+2];
            imageData.data[x*4+3] = rowData[idx+3] ?? 255;
        }
        ctx.putImageData(imageData, 0, Math.floor((startRow + r) * rowH));
    }
}

function escHtml(s) { const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }
function escAttr(s) { return s.replace(/[^a-z0-9]/gi,'_'); }

function send(obj) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
}

function connect() {
    const url = 'ws://127.0.0.1:5347';
    ws = new WebSocket(url);
    ws.onopen = () => {
        console.log('Connected to ez-backend');
        document.title = 'ez-sdr';
    };
    ws.onmessage = (e) => {
        try {
            const msg = JSON.parse(e.data);
            if (msg.type === 'state') updateUI(msg.state);
            if (msg.type === 'ai_response') {
                const chat = document.getElementById('aiChat');
                if (chat) {
                    chat.innerHTML += `<div class="ai-msg assistant">${escHtml(msg.text || msg.error || 'No response')}</div>`;
                    chat.scrollTop = chat.scrollHeight;
                }
            }
        } catch (err) { console.error('WS parse error', err); }
    };
    ws.onclose = () => {
        console.log('Disconnected, reconnecting in 2s...');
        setTimeout(connect, 2000);
    };
    ws.onerror = () => ws.close();
}

registerRenderers();
const myLayout = new GoldenLayout(config);
Object.keys(panelRenderers).forEach(name => {
    myLayout.registerComponent(name, function(container, state) {
        panelRenderers[name](container, state);
    });
});
myLayout.init();
connect();
