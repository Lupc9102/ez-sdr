'use strict';

// These tests execute the shipped daemon script and desktop Web Remote page.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const appSource = fs.readFileSync(path.join(__dirname, '../src/web/frontend/app.js'), 'utf8');
const remoteHtml = fs.readFileSync(path.join(__dirname, '../../ez-gui/src/web_remote.html'), 'utf8');

class Element {
  constructor(id = '') {
    this.id = id;
    this.value = '';
    this.textContent = '';
    this.hidden = false;
    this.disabled = false;
    this.className = '';
    this.style = {};
    this.dataset = {};
    this.children = [];
    this.listeners = new Map();
    this.classList = { toggle() {} };
  }
  contains() { return false; }
  replaceChildren(...children) { this.children = children; }
  append(...children) { this.children.push(...children); }
  appendChild(child) { this.children.push(child); return child; }
  addEventListener(name, callback) {
    const listeners = this.listeners.get(name) || [];
    listeners.push(callback);
    this.listeners.set(name, listeners);
  }
  dispatch(name) { for (const callback of this.listeners.get(name) || []) callback({ target: this }); }
}

class MockSocket {
  static OPEN = 1;
  constructor(url, protocols) {
    this.url = url;
    this.protocols = protocols;
    this.readyState = MockSocket.OPEN;
    this.listeners = new Map();
    this.sent = [];
    this.closed = false;
    MockSocket.sockets.push(this);
  }
  addEventListener(name, callback) { this.listeners.set(name, callback); }
  send(value) { this.sent.push(value); }
  close() { this.closed = true; }
  open() { this.onopen?.(); }
  message(value) { return this.onmessage?.({ data: JSON.stringify(value) }); }
}

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

function response(value) {
  return { ok: true, text: async () => JSON.stringify(value) };
}

function receiverStatus(kind) {
  return { hardware: { frequency_hz: kind === 'new' ? 105_000_000 : 100_000_000,
    sample_rate_hz: 1_000_000, gain_db: 20, source_kind: kind } };
}

function makeDaemonContext(fetchImpl) {
  const elements = new Map();
  const sockets = [];
  const document = {
    activeElement: new Element(),
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
    querySelectorAll() { return []; },
    createElement(tag) { return new Element(tag); },
  };
  const context = {
    document,
    location: { protocol: 'http:', host: '127.0.0.1:7891' },
    WebSocket: class extends MockSocket { constructor(url, protocols) { super(url, protocols); } },
    TextEncoder,
    Map,
    Uint8Array,
    DataView,
    ArrayBuffer,
    window: { addEventListener() {} },
    setInterval() { return 1; },
    clearInterval() {},
    setTimeout() {},
    fetch: fetchImpl,
  };
  // Keep per-context sockets isolated while retaining the shared mock class.
  MockSocket.sockets = sockets;
  vm.createContext(context);
  vm.runInContext(appSource, context, { filename: 'ez-daemon/src/web/frontend/app.js' });
  return { context, elements, sockets };
}

function makeDesktopContext() {
  const elements = new Map();
  const sockets = [];
  const document = {
    activeElement: null,
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
    createElement(tag) { return new Element(tag); },
  };
  elements.set('freqInput', new Element('freqInput'));
  elements.get('freqInput').value = '100000000';
  const context = {
    document,
    location: { host: '127.0.0.1:5259' },
    WebSocket: class extends MockSocket { constructor(url, protocols) { super(url, protocols); } },
    setTimeout() {},
    Date,
    Number,
    Math,
    JSON,
  };
  MockSocket.sockets = sockets;
  vm.createContext(context);
  const script = remoteHtml.match(/<script>([\s\S]*?)<\/script>/)?.[1];
  assert.ok(script, 'desktop Web Remote inline script exists');
  vm.runInContext(script, context, { filename: 'ez-gui/src/web_remote.html' });
  return { context, elements, sockets, document };
}

function welcome() {
  return { Welcome: { client_channels: { spectrum: 100 } } };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

async function testOldCloseCannotRetireReplacementSession() {
  const { context, sockets } = makeDaemonContext(async url => {
    if (url.endsWith('/status')) return response(receiverStatus('synthetic'));
    return response([]);
  });
  await vm.runInContext('connect()', context);
  const oldControl = sockets.find(socket => socket.url.endsWith('/ws/control'));
  oldControl.open();
  await oldControl.message(welcome());
  assert.equal(vm.runInContext('connected', context), true);

  await vm.runInContext('connect()', context);
  const replacement = sockets.filter(socket => socket.url.endsWith('/ws/control')).at(-1);
  replacement.open();
  await replacement.message(welcome());
  oldControl.onclose();

  assert.equal(vm.runInContext('connected', context), true);
  assert.equal(replacement.closed, false);
  const replacementStream = sockets.filter(socket => socket.url.includes('/ws/stream/')).at(-1);
  assert.ok(replacementStream, 'the replacement session creates its spectrum stream');
  assert.equal(replacementStream.closed, false);
}

async function testDelayedInitialStatusAndWelcomeCannotReviveDisconnect() {
  const pendingStatus = deferred();
  const { context, sockets, elements } = makeDaemonContext(() => pendingStatus.promise);
  const connect = vm.runInContext('connect()', context);
  vm.runInContext('disconnect()', context);
  pendingStatus.resolve(response(receiverStatus('obsolete')));
  await connect;
  assert.equal(sockets.length, 0, 'a stale initial status response must not open a socket');
  assert.equal(elements.get('connection').textContent, 'Disconnected');

  const second = makeDaemonContext(async url => response(url.endsWith('/status') ? receiverStatus('synthetic') : []));
  await vm.runInContext('connect()', second.context);
  const oldControl = second.sockets.find(socket => socket.url.endsWith('/ws/control'));
  vm.runInContext('disconnect()', second.context);
  await oldControl.message(welcome());
  assert.equal(vm.runInContext('connected', second.context), false);
  assert.equal(vm.runInContext('streams.size', second.context), 0);
  assert.equal(second.sockets.length, 1, 'a stale Welcome must not create a data stream');
}

async function testObsoleteRefreshCannotOverwriteCurrentSession() {
  const oldStatus = deferred(), oldChannels = deferred(), oldRecordings = deferred();
  let statusCalls = 0, channelCalls = 0, recordingCalls = 0;
  const { context, sockets, elements } = makeDaemonContext(url => {
    if (url.endsWith('/status')) {
      statusCalls++;
      if (statusCalls === 1) return response(receiverStatus('old-initial'));
      if (statusCalls === 2) return oldStatus.promise;
      return response(receiverStatus('new'));
    }
    if (url.endsWith('/channels')) {
      channelCalls++;
      return channelCalls === 1 ? oldChannels.promise : response([]);
    }
    recordingCalls++;
    return recordingCalls === 1 ? oldRecordings.promise : response([]);
  });
  await vm.runInContext('connect()', context);
  const oldControl = sockets.find(socket => socket.url.endsWith('/ws/control'));
  const oldRefresh = oldControl.message(welcome());
  await flush();

  await vm.runInContext('connect()', context);
  const newControl = sockets.filter(socket => socket.url.endsWith('/ws/control')).at(-1);
  await newControl.message(welcome());
  const currentSource = elements.get('source').textContent;
  assert.match(currentSource, /^new ·/);

  oldStatus.resolve(response(receiverStatus('old-refresh')));
  oldChannels.resolve(response([]));
  oldRecordings.resolve(response([]));
  await oldRefresh;
  assert.equal(elements.get('source').textContent, currentSource);
  assert.equal(vm.runInContext("status.hardware.source_kind", context), 'new');
}

function testDesktopFrequencyDraftAndValidation() {
  const { context, elements, sockets, document } = makeDesktopContext();
  const input = elements.get('freqInput');
  const socket = sockets[0];
  const telemetry = frequency => ({ frequency_hz: frequency, gain_db: 10, demod_mode: 'NFM',
    aircraft_count: 0, snr_db: 20, squelch_db: -60, volume: 0.8, recording: false,
    scanner_active: false, upcoming_passes: [] });

  input.value = '118700000';
  elements.get('freqInput').dispatch('input');
  document.activeElement = input;
  socket.onmessage({ data: JSON.stringify(telemetry(100000000)) });
  assert.equal(input.value, '118700000', 'telemetry preserves the edited frequency buffer');
  assert.equal(elements.get('freqDisplay').textContent, '100.0000 MHz', 'readout follows acknowledged telemetry');

  vm.runInContext('tuneFreq()', context);
  assert.equal(JSON.parse(socket.sent.at(-1)).hz, 118700000);
  input.value = 'NaN';
  input.dispatch('input');
  const sentBeforeInvalid = socket.sent.length;
  vm.runInContext('tuneFreq()', context);
  assert.equal(socket.sent.length, sentBeforeInvalid, 'non-finite input is never sent');

  input.value = '118700000';
  input.dispatch('input');
  vm.runInContext('tuneFreq()', context);
  socket.onmessage({ data: JSON.stringify(telemetry(118700000)) });
  assert.equal(input.value, 118700000, 'the acknowledged request reconciles the editor');
  assert.equal(vm.runInContext('frequencyDirty', context), false);
}

(async () => {
  await testOldCloseCannotRetireReplacementSession();
  await testDelayedInitialStatusAndWelcomeCannotReviveDisconnect();
  await testObsoleteRefreshCannotOverwriteCurrentSession();
  testDesktopFrequencyDraftAndValidation();
  console.log('web frontend regressions passed: session ownership, stale refresh, frequency draft and validation');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
