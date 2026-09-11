// App bootstrap: wires the control-plane socket, REST client, and UI panels together.
// No framework runtime — each ui/* module is a plain class owning a DOM subtree; this
// file just threads callbacks between them and the api/* clients.

import { ControlSocket } from "./api/control";
import * as rest from "./api/rest";
import { AudioMonitor } from "./audio/monitor";
import { SpectrumView } from "./ui/spectrum-view";
import { HardwarePanel } from "./ui/hardware-panel";
import { DisplayPanel } from "./ui/display-panel";
import { ChannelList } from "./ui/channel-list";
import { ChannelControls } from "./ui/channel-controls";
import { AircraftPanel } from "./ui/aircraft-panel";
import type { ChannelMetrics } from "./types";

function el<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing #${id}`);
  return found as T;
}

const connStatus = el<HTMLElement>("conn-status");
const errorBanner = el<HTMLElement>("error-banner");

const hardwarePanel = new HardwarePanel(el<HTMLElement>("hardware-panel"));
const displayPanel = new DisplayPanel(el<HTMLElement>("display-panel"));
const channelList = new ChannelList(el<HTMLElement>("channel-list"));
const channelControls = new ChannelControls(el<HTMLElement>("channel-controls"));
const aircraftPanel = new AircraftPanel(el<HTMLElement>("aircraft-panel"));
const spectrumView = new SpectrumView(
  el<HTMLElement>("spectrum-container"),
  el<HTMLCanvasElement>("spectrum-canvas"),
  el<HTMLCanvasElement>("spectrum-overlay"),
);
const audioMonitor = new AudioMonitor();
const control = new ControlSocket();

let channels: ChannelMetrics[] = [];
let selectedId: number | null = null;
let spectrumChannelId: number | null = null;
let errorTimer: number | undefined;

function showError(message: string): void {
  console.error(message);
  errorBanner.textContent = message;
  errorBanner.classList.add("error-banner--visible");
  window.clearTimeout(errorTimer);
  errorTimer = window.setTimeout(() => errorBanner.classList.remove("error-banner--visible"), 5000);
}

function selectChannel(id: number | null): void {
  selectedId = id;
  channelList.setSelected(id);
  spectrumView.setSelected(id);
  channelControls.setChannel(channels.find((c) => c.spec.id === id)?.spec ?? null);
  if (id !== null) {
    rest
      .getRecording(id)
      .then((status) => channelControls.setRecording(status))
      .catch(() => channelControls.setRecording(null));
  } else {
    channelControls.setRecording(null);
  }
}

async function refreshChannels(): Promise<void> {
  channels = await rest.listChannels();
  channelList.setChannels(channels);
  spectrumView.setChannels(channels.map((c) => c.spec));

  const spectrumChannel = channels.find((c) => c.spec.kind === "Spectrum");
  if (spectrumChannel && spectrumChannel.spec.id !== spectrumChannelId) {
    spectrumChannelId = spectrumChannel.spec.id;
    spectrumView.attachStream(spectrumChannelId);
  }

  const adsb = channels.find((c) => c.spec.kind === "AdsbPackets");
  aircraftPanel.setChannelId(adsb?.spec.id ?? null);

  if (selectedId !== null && !channels.some((c) => c.spec.id === selectedId)) {
    selectChannel(null);
  }
}

/** Ensures a wideband Spectrum channel exists (self-healing after a fresh daemon start —
 * pipelines don't survive a restart, unlike everything else here which just re-syncs), then
 * refreshes the whole dashboard from it. Safe to call repeatedly: a no-op create skip when
 * a Spectrum channel is already present. */
async function ensureDefaultsAndRefresh(): Promise<void> {
  const list = await rest.listChannels();
  if (!list.some((c) => c.spec.kind === "Spectrum")) {
    const hw = await rest.getHardware().catch(() => null);
    await rest
      .createChannel({
        id: 0,
        center_offset_hz: 0,
        bandwidth_hz: hw?.sample_rate_hz ?? 2_000_000,
        kind: "Spectrum",
        demod_mode: null,
      })
      .catch((err) => showError(`failed to create default spectrum channel: ${err.message}`));
  }
  await refreshChannels();
}

channelList.onSelect = (id) => selectChannel(id);
channelList.onCreate = (spec) => {
  rest
    .createChannel(spec)
    .then(() => refreshChannels())
    .catch((err) => showError(`create channel failed: ${err.message}`));
};

channelControls.onSetDemodMode = (id, mode) => {
  rest
    .setDemodMode(id, mode)
    // The daemon now mirrors the mode onto the channel's ChannelSpec, so a refresh shows the
    // real running mode; fall back to a local patch if the refetch races or fails.
    .then(() => refreshChannels())
    .catch((err) => showError(`set demod mode failed: ${err.message}`));
};
channelControls.onSetVolume = (id, level) => {
  rest.setVolume(id, level).catch((err) => showError(`set volume failed: ${err.message}`));
};
channelControls.onSetSquelch = (id, db) => {
  rest.setSquelch(id, db).catch((err) => showError(`set squelch failed: ${err.message}`));
};
channelControls.onStartRecording = (id, format) => {
  rest
    .startRecording(id, format)
    .then((status) => channelControls.setRecording(status))
    .catch((err) => showError(`start recording failed: ${err.message}`));
};
channelControls.onStopRecording = (id) => {
  rest
    .stopRecording(id)
    .then((status) => channelControls.setRecording(status))
    .catch((err) => showError(`stop recording failed: ${err.message}`));
};
channelControls.onToggleMonitor = (id) => {
  if (audioMonitor.monitoring === id) {
    audioMonitor.stop();
    channelControls.setMonitoring(null);
  } else {
    void audioMonitor.start(id);
    channelControls.setMonitoring(id);
  }
};

aircraftPanel.onCreateChannel = () => {
  const id = channels.reduce((max, c) => Math.max(max, c.spec.id), 0) + 1;
  rest
    .createChannel({ id, center_offset_hz: 0, bandwidth_hz: 2_000_000, kind: "AdsbPackets", demod_mode: null })
    .then(() => refreshChannels())
    .catch((err) => showError(`create ADS-B channel failed: ${err.message}`));
};

hardwarePanel.onSetFrequency = (hz) =>
  rest.setFrequency(hz).catch((err) => showError(`set frequency failed: ${err.message}`));
hardwarePanel.onSetSampleRate = (hz) =>
  rest.setSampleRate(hz).catch((err) => showError(`set sample rate failed: ${err.message}`));
hardwarePanel.onSetGain = (db) =>
  rest.setGain(db).catch((err) => showError(`set gain failed: ${err.message}`));

displayPanel.onSetPalette = (name) => spectrumView.setPalette(name);

spectrumView.onChannelSelect = (id) => selectChannel(id);
spectrumView.onChannelEdit = (edit) => {
  // Passband drag-editing in spectrum-view.ts updates its own local display live; on release
  // we push the new center/bandwidth to the daemon, which re-tunes the channel in place
  // (preserving the running pipeline's subscribers) and we re-sync the channel table.
  rest
    .retuneChannel(edit.id, edit.center_offset_hz, edit.bandwidth_hz)
    .then(() => refreshChannels())
    .catch((err) => showError(`retune failed: ${err.message}`));
};

channelControls.onDelete = (id) => {
  rest
    .deleteChannel(id)
    .then(() => {
      if (selectedId === id) selectChannel(null);
      refreshChannels();
    })
    .catch((err) => showError(`delete channel failed: ${err.message}`));
};

control.onConnection((connected) => {
  connStatus.textContent = connected ? "connected" : "reconnecting…";
  connStatus.classList.toggle("conn-status--up", connected);
  connStatus.classList.toggle("conn-status--down", !connected);
  hardwarePanel.setConnected(connected);
});

control.onEvent((event) => {
  if ("Welcome" in event) {
    void ensureDefaultsAndRefresh();
  } else if ("Hardware" in event) {
    hardwarePanel.setHardware(event.Hardware);
  } else if ("Error" in event) {
    showError(`server: ${event.Error.message}`);
  }
});

control.connect();
void ensureDefaultsAndRefresh();
