// Live audio monitoring: decodes AudioFrame binary WS frames and feeds them into an
// AudioWorklet ring buffer for low-latency playback, fully isolated from canvas work.
//
// The daemon's audio pipeline emits at its own rate (typically 48 kHz); the
// AudioContext is created with a matching sampleRate when the first frame arrives so
// no client-side resampling is needed in the common case (browsers resample internally
// if the hardware disagrees).

import { StreamSocket } from "../api/stream";
import { decodeAudio } from "../api/wire";

export class AudioMonitor {
  private socket: StreamSocket | null = null;
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  private gain: GainNode | null = null;
  private pendingSampleRate: number | null = null;
  private starting = false;
  private channelId: number | null = null;

  /** Which channel is currently being monitored, or null when muted/stopped. */
  get monitoring(): number | null {
    return this.channelId;
  }

  async start(channelId: number): Promise<void> {
    this.stop();
    this.channelId = channelId;
    this.socket = new StreamSocket("audio", channelId, (buf) => this.onFrame(buf));
    this.socket.connect();
  }

  private onFrame(buf: ArrayBuffer): void {
    const frame = decodeAudio(buf);
    if (!this.ctx && !this.starting) {
      this.pendingSampleRate = frame.sample_rate_hz;
      void this.initAudio();
    }
    this.node?.port.postMessage(frame.samples, [frame.samples.buffer]);
  }

  private async initAudio(): Promise<void> {
    this.starting = true;
    try {
      const ctx = new AudioContext({
        sampleRate: this.pendingSampleRate ?? 48_000,
        latencyHint: "interactive",
      });
      await ctx.audioWorklet.addModule("/audio-processor.js");
      const node = new AudioWorkletNode(ctx, "ring-player", {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [1],
      });
      const gain = ctx.createGain();
      node.connect(gain);
      gain.connect(ctx.destination);
      // Browsers may start the context suspended until a user gesture; monitoring is
      // always user-initiated (a click), so resume() succeeds here.
      await ctx.resume();
      this.ctx = ctx;
      this.node = node;
      this.gain = gain;
    } finally {
      this.starting = false;
    }
  }

  /** Local monitor gain (independent of the daemon-side per-channel volume). */
  setLocalGain(level: number): void {
    if (this.gain) this.gain.gain.value = level;
  }

  stop(): void {
    this.socket?.close();
    this.socket = null;
    this.node?.disconnect();
    this.gain?.disconnect();
    void this.ctx?.close();
    this.ctx = null;
    this.node = null;
    this.gain = null;
    this.channelId = null;
  }
}
