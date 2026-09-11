// AudioWorklet processor: pulls f32 mono samples from a lock-free-ish ring buffer
// filled by the main thread via port.postMessage. Kept as plain JS in public/ so it
// can be loaded directly with audioWorklet.addModule() — worklet modules load by URL,
// outside the app bundle.
//
// The ring absorbs network jitter; target fill is ~150 ms. On underrun we emit
// silence rather than stretching, and on overrun (slow tab, long GC pause) we drop
// the oldest samples so live monitoring stays near-real-time instead of drifting
// ever further behind the daemon.

class RingPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.capacity = 65536; // samples (~1.4 s at 48 kHz) — hard jitter ceiling
    this.buf = new Float32Array(this.capacity);
    this.readPos = 0;
    this.writePos = 0;
    this.size = 0;
    this.port.onmessage = (e) => {
      const samples = e.data;
      if (!(samples instanceof Float32Array)) return;
      for (let i = 0; i < samples.length; i++) {
        if (this.size === this.capacity) {
          // Overrun: drop oldest to stay live.
          this.readPos = (this.readPos + 1) % this.capacity;
          this.size -= 1;
        }
        this.buf[this.writePos] = samples[i];
        this.writePos = (this.writePos + 1) % this.capacity;
        this.size += 1;
      }
    };
  }

  process(_inputs, outputs) {
    const out = outputs[0][0];
    if (!out) return true;
    for (let i = 0; i < out.length; i++) {
      if (this.size > 0) {
        out[i] = this.buf[this.readPos];
        this.readPos = (this.readPos + 1) % this.capacity;
        this.size -= 1;
      } else {
        out[i] = 0; // underrun -> silence
      }
    }
    return true;
  }
}

registerProcessor("ring-player", RingPlayerProcessor);
