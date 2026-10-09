//! Wideband -> narrowband channelizer: digital down-conversion (DDC) of the shared wideband
//! [`SampleBus`] into any number of independently tuned virtual channels, each its own
//! [`SampleBus`] that pipelines (spectrum, audio, packet, telemetry) subscribe to.
//!
//! Each virtual channel is a plain, hand-rolled DDC chain — NCO mix to baseband, windowed-
//! sinc FIR low-pass, decimate — driven directly by [`Channelizer::tick`]/[`Channelizer::run`].
//! No generic pipeline/stage abstraction: one struct per concept, one function per step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use num_complex::Complex32;

use crate::bus::{OverflowPolicy, SampleBlock, SampleBus, SampleBusHandle};

/// A numerically controlled oscillator: generates `e^{-j*2*pi*f*n/fs}` incrementally so a
/// wideband block can be mixed to baseband one sample at a time, with phase carried
/// correctly across successive blocks.
struct Nco {
    phase: f64,
    phase_increment: f64,
    offset_hz: f64,
}

impl Nco {
    fn new(freq_offset_hz: f64, sample_rate_hz: f64) -> Self {
        let phase_increment = -2.0 * std::f64::consts::PI * freq_offset_hz / sample_rate_hz;
        Self {
            phase: 0.0,
            phase_increment,
            offset_hz: freq_offset_hz,
        }
    }

    fn seek(&mut self, sample_index: u64, source_rate_hz: f64) {
        // Integer-Hz capture clocks retain phase even at large absolute indices.
        let cycles = if self.offset_hz.fract() == 0.0
            && source_rate_hz.fract() == 0.0
            && source_rate_hz > 0.0
        {
            let rate = source_rate_hz as i128;
            (-(self.offset_hz as i128) * i128::from(sample_index)).rem_euclid(rate) as f64
                / rate as f64
        } else {
            (-self.offset_hz / source_rate_hz * sample_index as f64).rem_euclid(1.0)
        };
        self.phase = cycles * std::f64::consts::TAU;
        if self.phase > std::f64::consts::PI {
            self.phase -= std::f64::consts::TAU;
        }
    }

    /// Mixes `samples` down (or up) by this NCO's tuned offset, in place order preserved as
    /// a new `Vec` (the input itself, e.g. straight off the wideband bus, is never mutated).
    fn mix(&mut self, samples: &[Complex32]) -> Vec<Complex32> {
        samples
            .iter()
            .map(|&s| {
                let osc = Complex32::from_polar(1.0, self.phase as f32);
                self.phase += self.phase_increment;
                if self.phase > std::f64::consts::PI {
                    self.phase -= 2.0 * std::f64::consts::PI;
                } else if self.phase < -std::f64::consts::PI {
                    self.phase += 2.0 * std::f64::consts::PI;
                }
                s * osc
            })
            .collect()
    }
}

/// Minimum four-term Blackman-Harris window, evaluated in double precision.
fn blackman_harris(position: f64) -> f64 {
    let angle = std::f64::consts::TAU * position;
    0.35875 - 0.48829 * angle.cos() + 0.14128 * (2.0 * angle).cos() - 0.01168 * (3.0 * angle).cos()
}

/// Causal fractional-delay low-pass. Cutoff is a fraction of input Nyquist.
/// Double-precision design and DC normalization precede coefficient storage.
fn design_lowpass(num_taps: usize, cutoff: f64, phase: f64) -> Vec<f32> {
    let middle = (num_taps - 1) as f64 / 2.0;
    let mut taps: Vec<f64> = (0..num_taps)
        .map(|n| {
            let x = n as f64 - middle - phase;
            let sinc = if x.abs() < 1e-12 {
                cutoff
            } else {
                (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            sinc * blackman_harris(n as f64 / (num_taps - 1) as f64)
        })
        .collect();
    let gain: f64 = taps.iter().sum();
    taps.iter_mut().for_each(|tap| *tap /= gain);
    taps.into_iter().map(|tap| tap as f32).collect()
}

const HALF_BAND_TAPS: usize = 63;
const FILTER_PHASES: usize = 256;
/// Pass through 0.4 * output Fs, stop from 0.5 * output Fs. The midpoint
/// cutoff is 0.45 * output Fs; 100 input taps per remaining rate ratio
/// leave five window bins on either side of that midpoint. Sweep tests
/// verify <=0.02 dB passband error and >=80 dB stopband rejection.
const TAPS_PER_RATIO: f64 = 100.0;

struct HalfBandStage {
    paired: [f32; 16],
    center: f32,
    history: [Complex32; 2 * HALF_BAND_TAPS],
    write: usize,
    emit: bool,
}

impl HalfBandStage {
    fn new() -> Self {
        let mut taps = design_lowpass(HALF_BAND_TAPS, 0.5, 0.0);
        // Half-band symmetry: every second coefficient except the center
        // is exactly zero. Keep these zeros exact, then normalize again.
        for n in (1..HALF_BAND_TAPS).step_by(2) {
            if n != HALF_BAND_TAPS / 2 {
                taps[n] = 0.0;
            }
        }
        let gain: f64 = taps.iter().map(|&tap| f64::from(tap)).sum();
        Self {
            paired: std::array::from_fn(|n| (f64::from(taps[2 * n]) / gain) as f32),
            center: (f64::from(taps[HALF_BAND_TAPS / 2]) / gain) as f32,
            history: [Complex32::new(0.0, 0.0); 2 * HALF_BAND_TAPS],
            write: 0,
            emit: true,
        }
    }

    fn reset(&mut self) {
        self.history.fill(Complex32::new(0.0, 0.0));
        self.write = 0;
        self.emit = true;
    }

    fn push(&mut self, sample: Complex32) -> Option<Complex32> {
        self.history[self.write] = sample;
        self.history[self.write + HALF_BAND_TAPS] = sample;
        self.write = (self.write + 1) % HALF_BAND_TAPS;
        let emit = self.emit;
        self.emit = !self.emit;
        if !emit {
            return None;
        }
        let window = &self.history[self.write..self.write + HALF_BAND_TAPS];
        let mut output = window[HALF_BAND_TAPS / 2] * self.center;
        for (n, &tap) in self.paired.iter().enumerate() {
            output += (window[2 * n] + window[HALF_BAND_TAPS - 1 - 2 * n]) * tap;
        }
        Some(output)
    }
}

struct PhaseFilter {
    banks: Vec<Vec<f32>>,
    history: Vec<Complex32>,
    write: usize,
    len: usize,
}

impl PhaseFilter {
    fn new(ratio: f64, phases: usize) -> Self {
        let len = (TAPS_PER_RATIO * ratio).ceil() as usize | 1;
        Self {
            banks: (0..=phases)
                .map(|n| design_lowpass(len, 0.9 / ratio, n as f64 / phases.max(1) as f64))
                .collect(),
            history: vec![Complex32::new(0.0, 0.0); 2 * len],
            write: 0,
            len,
        }
    }

    fn reset(&mut self) {
        self.history.fill(Complex32::new(0.0, 0.0));
        self.write = 0;
    }

    fn push(&mut self, sample: Complex32) {
        self.history[self.write] = sample;
        self.history[self.write + self.len] = sample;
        self.write = (self.write + 1) % self.len;
    }

    fn output(&self, phase: f64) -> Complex32 {
        let coordinate = phase * (self.banks.len() - 1) as f64;
        let bank = coordinate.floor() as usize;
        let blend = (coordinate - bank as f64) as f32;
        let window = &self.history[self.write..self.write + self.len];
        if blend == 0.0 {
            return window
                .iter()
                .zip(&self.banks[bank])
                .map(|(&x, &tap)| x * tap)
                .sum();
        }
        // Interpolate coefficients, never already-aliased input samples.
        // Exact phases are used through factor 256. At higher factors the
        // bank interpolation error is included in the measured response.
        window
            .iter()
            .zip(&self.banks[bank])
            .zip(&self.banks[bank + 1])
            .map(|((&x, &a), &b)| x * (a + (b - a) * blend))
            .sum()
    }
}

/// Streaming multistage/polyphase decimator. Half-band stages reduce the
/// working rate; the final phase-selected FIR emits at exact source offsets
/// n*D even when D is not divisible by the half-band product. This keeps
/// sample counts, source clock stride and caller-chunk independence intact.
/// Coefficient/history storage stays bounded for large and prime D.
struct FirDecimator {
    decimation: usize,
    skip: usize,
    stages: Vec<HalfBandStage>,
    stage_factor: usize,
    output_phase: usize,
    final_filter: Option<PhaseFilter>,
}

impl FirDecimator {
    fn new(decimation: usize) -> Self {
        assert!(decimation > 0);
        if decimation == 1 {
            return Self {
                decimation,
                skip: 0,
                stages: Vec::new(),
                stage_factor: 1,
                output_phase: 0,
                final_filter: None,
            };
        }
        let stages = if decimation < 8 {
            0
        } else {
            (decimation / 4).ilog2() as usize
        };
        let stage_factor = 1usize << stages;
        let phases = if stages == 0 { 0 } else { FILTER_PHASES };
        Self {
            decimation,
            skip: 0,
            stages: (0..stages).map(|_| HalfBandStage::new()).collect(),
            stage_factor,
            output_phase: 0,
            final_filter: Some(PhaseFilter::new(
                decimation as f64 / stage_factor as f64,
                phases,
            )),
        }
    }

    fn reset(&mut self, first_offset: usize) {
        self.skip = first_offset;
        self.output_phase = first_offset % self.stage_factor;
        self.stages.iter_mut().for_each(HalfBandStage::reset);
        if let Some(filter) = &mut self.final_filter {
            filter.reset();
        }
    }

    fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let Some(filter) = &mut self.final_filter else {
            return input.to_vec();
        };
        let mut output = Vec::with_capacity(input.len() / self.decimation + 1);
        let mut next = self.skip;
        for (n, &sample) in input.iter().enumerate() {
            let mut value = Some(sample);
            for stage in &mut self.stages {
                value = value.and_then(|sample| stage.push(sample));
                if value.is_none() {
                    break;
                }
            }
            if let Some(sample) = value {
                filter.push(sample);
            }
            if n == next {
                output.push(filter.output(self.output_phase as f64 / self.stage_factor as f64));
                self.output_phase =
                    (self.output_phase + self.decimation % self.stage_factor) % self.stage_factor;
                next += self.decimation;
            }
        }
        self.skip = next - input.len();
        output
    }
}

fn make_decimator(decimation: usize) -> FirDecimator {
    FirDecimator::new(decimation)
}

/// One tuned narrowband tap on the wideband stream: an independent DDC chain plus the
/// [`SampleBus`] its decimated output is published to.
#[derive(Clone, Copy)]
struct InputEnd {
    sample: u64,
    clock: ez_proto::CaptureClock,
    rate_hz: u32,
    center_hz: u64,
}

struct VirtualChannel {
    id: u64,
    center_freq_hz: u64,
    output_rate_hz: u32,
    nco: Nco,
    decimator: FirDecimator,
    last_input: Option<InputEnd>,
    output_epoch: u64,
    pending_discontinuity: bool,
    bus: SampleBus,
}

impl VirtualChannel {
    fn new(
        id: u64,
        wideband_center_hz: u64,
        wideband_rate_hz: u32,
        center_freq_hz: u64,
        decimation: usize,
    ) -> Self {
        let offset_hz = center_freq_hz as f64 - wideband_center_hz as f64;
        let nco = Nco::new(offset_hz, wideband_rate_hz as f64);
        let decimator = make_decimator(decimation);
        Self {
            id,
            center_freq_hz,
            output_rate_hz: (wideband_rate_hz as usize / decimation) as u32,
            nco,
            decimator,
            last_input: None,
            output_epoch: ez_proto::new_capture_epoch(),
            pending_discontinuity: true,
            bus: SampleBus::new(),
        }
    }

    /// Mixes and decimates one wideband block, returning this channel's next output block —
    /// or `None` if too few samples have accumulated yet to produce one (e.g. a short block
    /// against a large decimation factor).
    fn process(&mut self, block: &SampleBlock) -> Option<SampleBlock> {
        let clock = block.capture_clock();
        if clock.source_sample_rate_hz == 0 || clock.sample_stride == 0 || block.sample_rate_hz == 0
        {
            self.pending_discontinuity = true;
            self.last_input = None;
            return None;
        }
        let contiguous = !clock.discontinuous
            && self.last_input.is_some_and(|end| {
                end.sample == block.start_sample
                    && end.rate_hz == block.sample_rate_hz
                    && end.center_hz == block.center_freq_hz
                    && end.clock.epoch == clock.epoch
                    && end.clock.source_sample_index == clock.source_sample_index
                    && end.clock.source_sample_rate_hz == clock.source_sample_rate_hz
                    && end.clock.sample_stride == clock.sample_stride
                    && end.clock.origin_unix_ns == clock.origin_unix_ns
            });
        let decimation = self.decimator.decimation as u64;
        if !contiguous {
            self.output_epoch = ez_proto::new_capture_epoch();
            self.pending_discontinuity = true;
            let first_offset =
                ((decimation - block.start_sample % decimation) % decimation) as usize;
            self.decimator.reset(first_offset);
            self.nco = Nco::new(
                self.center_freq_hz as f64 - block.center_freq_hz as f64,
                clock.exact_sample_rate_hz(),
            );
            self.nco.seek(
                clock.source_sample_index,
                f64::from(clock.source_sample_rate_hz),
            );
        }
        let first_offset = self.decimator.skip as u64;
        let mixed = self.nco.mix(&block.samples);
        let decimated = self.decimator.process(&mixed);
        self.last_input = Some(InputEnd {
            sample: block.start_sample + block.samples.len() as u64,
            clock: clock.offset_samples(block.samples.len() as u64)?,
            rate_hz: block.sample_rate_hz,
            center_hz: block.center_freq_hz,
        });
        if decimated.is_empty() {
            return None;
        }
        let start_sample = (block.start_sample + first_offset) / decimation;
        let mut output_clock = clock.offset_samples(first_offset)?;
        output_clock.epoch = self.output_epoch;
        output_clock.sample_stride = output_clock.sample_stride.checked_mul(decimation as u32)?;
        output_clock.discontinuous = self.pending_discontinuity;
        self.pending_discontinuity = false;
        self.output_rate_hz = block.sample_rate_hz / decimation as u32;
        Some(SampleBlock {
            product_session: block.product_session,
            observed_at: block.observed_at,
            capture: Some(output_clock),
            start_sample,
            sample_rate_hz: self.output_rate_hz,
            center_freq_hz: self.center_freq_hz,
            samples: Arc::from(decimated),
        })
    }
}

/// Routes one wideband [`SampleBus`] into any number of independently tuned virtual
/// channels. Owns the wideband subscription and the channel registry; `add_channel`/
/// `remove_channel` are the only mutation points, so registration is as cheap and simple as
/// a `Vec` push/retain — no need for the bus's lock-free machinery here, since only the one
/// thread driving `tick`/`run` ever touches the registry.
pub struct Channelizer {
    wideband: SampleBusHandle,
    wideband_center_hz: u64,
    wideband_rate_hz: u32,
    channels: Vec<VirtualChannel>,
    next_id: u64,
}

impl Channelizer {
    #[must_use]
    pub fn new(wideband: SampleBusHandle, wideband_center_hz: u64, wideband_rate_hz: u32) -> Self {
        Self {
            wideband,
            wideband_center_hz,
            wideband_rate_hz,
            channels: Vec::new(),
            next_id: 0,
        }
    }

    /// Registers a new virtual channel tuned to `center_freq_hz`, decimated down to (at
    /// least) `min_output_rate_hz`. Returns the channel's id (for [`Self::remove_channel`])
    /// and a bus handle its consumer(s) receive decimated output from; dropping every handle
    /// subscribed to it does not remove the channel itself — call `remove_channel` for that.
    pub fn add_channel(
        &mut self,
        center_freq_hz: u64,
        min_output_rate_hz: u32,
    ) -> Result<(u64, SampleBusHandle)> {
        if min_output_rate_hz == 0 {
            return Err(anyhow!("virtual channel output rate must be non-zero"));
        }
        let offset_hz = center_freq_hz as f64 - self.wideband_center_hz as f64;
        let nyquist_hz = self.wideband_rate_hz as f64 / 2.0;
        if offset_hz.abs() > nyquist_hz {
            return Err(anyhow!(
                "requested channel {center_freq_hz} Hz is outside the wideband capture's {:.0}-{:.0} Hz span",
                self.wideband_center_hz as f64 - nyquist_hz,
                self.wideband_center_hz as f64 + nyquist_hz,
            ));
        }

        let decimation = (self.wideband_rate_hz / min_output_rate_hz).max(1) as usize;
        let id = self.next_id;
        self.next_id += 1;
        let channel = VirtualChannel::new(
            id,
            self.wideband_center_hz,
            self.wideband_rate_hz,
            center_freq_hz,
            decimation,
        );
        let handle = channel.bus.subscribe(64, OverflowPolicy::DropOldest);
        self.channels.push(channel);
        Ok((id, handle))
    }

    pub fn remove_channel(&mut self, id: u64) {
        self.channels.retain(|c| c.id != id);
    }

    /// Re-tunes an already-registered virtual channel in place: recomputes its NCO offset and
    /// rebuilds its FIR/decimation for the new `center_freq_hz`/`min_output_rate_hz` **while
    /// keeping the same `SampleBus`** the channel was originally created with. Preserving the
    /// bus means every consumer that already holds a subscription (a pipeline's input tap, a
    /// recording handle) keeps receiving output uninterrupted — this is exactly what makes a
    /// live `Retune` possible without tearing down and re-spawning the channel's pipeline.
    ///
    /// The decimator's filter history is reset (a fresh FIR for the new bandwidth); a brief
    /// transient at the instant of retune is expected and physically matches a hardware retune.
    pub fn retune_channel(
        &mut self,
        id: u64,
        center_freq_hz: u64,
        min_output_rate_hz: u32,
    ) -> Result<()> {
        if min_output_rate_hz == 0 {
            return Err(anyhow!("virtual channel output rate must be non-zero"));
        }
        let offset_hz = center_freq_hz as f64 - self.wideband_center_hz as f64;
        let nyquist_hz = self.wideband_rate_hz as f64 / 2.0;
        if offset_hz.abs() > nyquist_hz {
            return Err(anyhow!(
                "requested channel {center_freq_hz} Hz is outside the wideband capture's {:.0}-{:.0} Hz span",
                self.wideband_center_hz as f64 - nyquist_hz,
                self.wideband_center_hz as f64 + nyquist_hz,
            ));
        }
        let decimation = (self.wideband_rate_hz / min_output_rate_hz).max(1) as usize;
        let decimator = make_decimator(decimation);

        let Some(channel) = self.channels.iter_mut().find(|c| c.id == id) else {
            return Err(anyhow!("unknown virtual channel {id}"));
        };
        channel.center_freq_hz = center_freq_hz;
        channel.output_rate_hz = (self.wideband_rate_hz as usize / decimation) as u32;
        channel.nco = Nco::new(offset_hz, self.wideband_rate_hz as f64);
        channel.decimator = decimator;
        channel.last_input = None;
        channel.pending_discontinuity = true;
        Ok(())
    }

    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// The wideband capture's current center frequency, as last set by [`Self::new`] or
    /// [`Self::retune`]. Callers (e.g. `DaemonState`) need this to translate a client's
    /// `center_offset_hz`-relative-to-wideband-center channel request into the absolute
    /// `center_freq_hz` [`Self::add_channel`] expects.
    #[must_use]
    pub fn wideband_center_hz(&self) -> u64 {
        self.wideband_center_hz
    }

    /// Wideband input sample rate in Hz. Used to validate virtual-channel
    /// requests (bandwidth/offset must fit inside the wideband).
    pub fn wideband_rate_hz(&self) -> u32 {
        self.wideband_rate_hz
    }

    /// Updates the capture rate used for future virtual channels. Existing channels cannot
    /// be migrated safely because their NCO, FIR, and decimator state all depend on the old
    /// rate, so callers must remove them first. A wideband-only spectrum pipeline is not a
    /// channelizer channel and therefore does not prevent this update.
    pub fn set_wideband_rate_hz(&mut self, hz: u32) -> Result<()> {
        if hz == 0 {
            return Err(anyhow!("wideband sample rate must be non-zero"));
        }
        if !self.channels.is_empty() {
            return Err(anyhow!(
                "cannot change wideband sample rate while virtual channels are active"
            ));
        }
        self.wideband_rate_hz = hz;
        Ok(())
    }

    /// The actual decimated output sample rate of a live channel, or `None` if `id` doesn't
    /// name one. This can differ from the `min_output_rate_hz` originally requested via
    /// [`Self::add_channel`] (which only guarantees "at least this much"), so callers that
    /// need the real rate (e.g. `TelemetryPipeline`'s fixed-rate decoder) must read it back
    /// here rather than assuming their request was honored exactly.
    #[must_use]
    pub fn channel_output_rate_hz(&self, id: u64) -> Option<u32> {
        self.channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.output_rate_hz)
    }

    pub fn channel_center_hz(&self, id: u64) -> Option<u64> {
        self.channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.center_freq_hz)
    }

    /// Re-centers the wideband capture (e.g. the hardware source retuned), recomputing every
    /// active virtual channel's NCO offset against the new center. The next input
    /// resets NCO/FIR history and publishes a new discontinuous output epoch.
    /// Each block's actual frequency/rate determines its mix, including queued
    /// input from before the hardware retune.
    pub fn retune(&mut self, new_wideband_center_hz: u64) {
        self.wideband_center_hz = new_wideband_center_hz;
        for channel in &mut self.channels {
            let offset_hz = channel.center_freq_hz as f64 - new_wideband_center_hz as f64;
            channel.nco = Nco::new(offset_hz, self.wideband_rate_hz as f64);
            channel.last_input = None;
            channel.pending_discontinuity = true;
        }
    }

    /// Returns an additional, independent subscriber handle for an already-registered
    /// channel (e.g. so a recording can attach to a channel already feeding a demod
    /// pipeline, without disturbing it). `None` if `id` doesn't name a live channel.
    #[must_use]
    pub fn subscribe(&self, id: u64, capacity: usize) -> Option<SampleBusHandle> {
        self.channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.bus.subscribe(capacity, OverflowPolicy::DropOldest))
    }

    /// Waits up to `poll_timeout` for the next wideband block and, if one arrived, routes it
    /// through every active channel. Returns whether a block was actually processed —
    /// callers driving a shutdown-aware loop should check an external stop signal on `false`
    /// rather than treating it as "source gone" (see [`SampleBusHandle::recv_timeout`]).
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        let Some(block) = self.wideband.recv_timeout(poll_timeout) else {
            return false;
        };
        for channel in &mut self.channels {
            if let Some(out) = channel.process(&block) {
                channel.bus.publish(out);
            }
        }
        true
    }

    /// Drives `tick` in a loop until `running` is set to `false`.
    pub fn run(&mut self, running: &AtomicBool) {
        while running.load(Ordering::Relaxed) {
            self.tick(Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wideband_block(
        center_freq_hz: u64,
        sample_rate_hz: u32,
        samples: Vec<Complex32>,
    ) -> SampleBlock {
        SampleBlock {
            product_session: 0,
            observed_at: None,
            capture: None,
            start_sample: 0,
            sample_rate_hz,
            center_freq_hz,
            samples: Arc::from(samples),
        }
    }

    fn tone(n: usize, freq_hz: f64, sample_rate_hz: f64) -> Vec<Complex32> {
        (0..n)
            .map(|k| {
                let phase = 2.0 * std::f64::consts::PI * freq_hz * (k as f64) / sample_rate_hz;
                Complex32::from_polar(1.0, phase as f32)
            })
            .collect()
    }

    #[test]
    fn missing_input_remains_visible_after_decimation_and_filter_state_resets() {
        for decimation in [1, 4, 7] {
            let mut channel = VirtualChannel::new(1, 100_000, 2_048_003, 100_000, decimation);
            let mut first = wideband_block(100_000, 2_048_003, vec![Complex32::new(1.0, 0.0); 70]);
            first.product_session = 42;
            let previous = channel.process(&first).unwrap();
            let mut next = wideband_block(100_000, 2_048_003, vec![Complex32::new(0.0, 0.0); 70]);
            next.start_sample = 1000;
            next.product_session = 42;
            let current = channel.process(&next).unwrap();
            assert_eq!(current.start_sample, 1000_u64.div_ceil(decimation as u64));
            assert!(current.capture.unwrap().discontinuous);
            assert_eq!(previous.product_session, 42);
            assert_eq!(current.product_session, 42);
            assert_ne!(
                current.capture.unwrap().epoch,
                previous.capture.unwrap().epoch
            );
            assert!(
                current.samples.iter().all(|sample| sample.norm() == 0.0),
                "FIR history bridged the gap"
            );
            let capture = current.capture.unwrap();
            assert_eq!(
                capture.source_sample_index,
                current.start_sample * decimation as u64
            );
            assert_eq!(capture.sample_stride, decimation as u32);
            assert_eq!(
                capture.exact_sample_rate_hz(),
                2_048_003.0 / decimation as f64
            );
        }
    }

    #[test]
    fn sub_decimation_gap_has_an_explicit_marker_even_when_output_indices_are_adjacent() {
        let mut channel = VirtualChannel::new(1, 100_000, 1000, 100_000, 4);
        let before = channel
            .process(&wideband_block(
                100_000,
                1000,
                vec![Complex32::new(1.0, 0.0)],
            ))
            .unwrap();
        let mut gap = wideband_block(100_000, 1000, vec![Complex32::new(0.0, 0.0)]);
        gap.start_sample = 2;
        assert!(
            channel.process(&gap).is_none(),
            "this block precedes the next decimation grid point"
        );
        gap.start_sample = 3;
        gap.samples = Arc::from(vec![Complex32::new(0.0, 0.0); 3]);
        let after = channel.process(&gap).unwrap();
        assert_eq!(
            after.start_sample,
            before.start_sample + before.samples.len() as u64
        );
        assert!(
            after.capture.unwrap().discontinuous,
            "numeric adjacency must not hide missing acquisition samples"
        );
        assert_ne!(before.capture.unwrap().epoch, after.capture.unwrap().epoch);
        assert!(after.samples.iter().all(|sample| sample.norm() == 0.0));
    }

    #[test]
    fn channelizer_preserves_acquisition_origin_and_continuous_fractional_clock() {
        let mut channel = VirtualChannel::new(1, 100_000, 2_048_003, 100_000, 7);
        let mut input = wideband_block(100_000, 2_048_003, vec![Complex32::new(0.0, 0.0); 73]);
        input.start_sample = 13;
        input.observed_at = Some(std::time::Instant::now());
        input.capture = Some(ez_proto::CaptureClock::new(
            42,
            13,
            2_048_003,
            Some(1_000_000),
        ));
        let first = channel.process(&input).unwrap();
        assert_eq!(first.observed_at, input.observed_at);
        input.start_sample += 73;
        input.capture = input.capture.unwrap().offset_samples(73);
        let second = channel.process(&input).unwrap();
        let a = first.capture.unwrap();
        let b = second.capture.unwrap();
        assert_eq!(a.source_sample_index, 14);
        assert_eq!(a.epoch, b.epoch);
        assert!(!b.discontinuous);
        assert_eq!(
            second.start_sample,
            first.start_sample + first.samples.len() as u64
        );
        assert_eq!(b.source_sample_index, second.start_sample * 7);
        assert_eq!(
            b.unix_time_ns(0),
            Some(1_000_000 + b.source_sample_index * 1_000_000_000 / 2_048_003)
        );
    }

    #[test]
    fn nco_phase_follows_the_acquisition_index_after_a_gap() {
        let mut channel = VirtualChannel::new(1, 100_000, 1000, 100_100, 1);
        for start in [0_u64, 1003, 1_000_000_003] {
            let samples: Vec<_> = (start..start + 40)
                .map(|n| {
                    let cycles = (100_u128 * u128::from(n)) % 1000;
                    Complex32::from_polar(
                        1.0,
                        (cycles as f64 / 1000.0 * std::f64::consts::TAU) as f32,
                    )
                })
                .collect();
            let mut input = wideband_block(100_000, 1000, samples);
            input.start_sample = start;
            let output = channel.process(&input).unwrap();
            assert!(output
                .samples
                .iter()
                .all(|s| (*s - Complex32::new(1.0, 0.0)).norm() < 1e-5));
        }
    }

    #[test]
    fn actual_wideband_queue_overflow_reaches_every_channel_as_a_gap() {
        for decimation in [1, 4] {
            let bus = SampleBus::new();
            let input = bus.subscribe(1, OverflowPolicy::DropOldest);
            let mut channelizer = Channelizer::new(input, 100_000, 1000);
            let (_, output) = channelizer.add_channel(100_000, 1000 / decimation).unwrap();
            let mut block = wideband_block(100_000, 1000, vec![Complex32::new(1.0, 0.0); 8]);
            bus.publish(block.clone());
            assert!(channelizer.tick(Duration::ZERO));
            let before = output.try_recv().unwrap();
            block.start_sample = 8;
            bus.publish(block.clone());
            block.start_sample = 16;
            block.samples = Arc::from(vec![Complex32::new(0.0, 0.0); 8]);
            bus.publish(block);
            assert_eq!(channelizer.wideband.dropped_count(), 1);
            assert!(channelizer.tick(Duration::ZERO));
            let after = output.try_recv().unwrap();
            assert_eq!(after.start_sample, 16 / u64::from(decimation));
            assert!(after.start_sample > before.start_sample + before.samples.len() as u64);
            assert!(after.capture.unwrap().discontinuous);
            assert!(after.samples.iter().all(|s| s.norm() == 0.0));
        }
    }

    #[test]
    fn source_epoch_and_channel_retune_are_visible_even_with_adjacent_indices() {
        let bus = SampleBus::new();
        let input = bus.subscribe(8, OverflowPolicy::DropOldest);
        let mut channelizer = Channelizer::new(input, 100_000, 1000);
        let (id, output) = channelizer.add_channel(100_000, 250).unwrap();
        let mut block = wideband_block(100_000, 1000, vec![Complex32::new(0.0, 0.0); 16]);
        let mut epoch = None;
        for (index, source_epoch, retune) in [(0, 1, false), (16, 2, false), (32, 2, true)] {
            block.start_sample = index;
            block.capture = Some(ez_proto::CaptureClock::new(source_epoch, index, 1000, None));
            if retune {
                channelizer.retune_channel(id, 100_100, 250).unwrap();
            }
            bus.publish(block.clone());
            assert!(channelizer.tick(Duration::ZERO));
            let next = output.try_recv().unwrap();
            assert!(next.capture.unwrap().discontinuous);
            assert_ne!(Some(next.capture.unwrap().epoch), epoch);
            assert_eq!(next.start_sample, index / 4);
            epoch = Some(next.capture.unwrap().epoch);
        }
    }

    #[test]
    fn nco_mix_removes_matching_offset_tone_to_near_dc() {
        let sample_rate = 1_000.0;
        let offset = 100.0;
        let input = tone(256, offset, sample_rate);

        let mut nco = Nco::new(offset, sample_rate);
        let mixed = nco.mix(&input);

        for w in mixed.windows(2) {
            let dphi = (w[1] * w[0].conj()).arg();
            assert!(dphi.abs() < 1e-3, "residual phase step {dphi}");
        }
    }

    #[test]
    fn nco_mix_preserves_magnitude_over_many_samples() {
        let mut nco = Nco::new(12_345.678, 2_048_000.0);
        let input = vec![Complex32::new(1.0, 0.0); 100_000];
        let mixed = nco.mix(&input);
        for s in mixed.iter().step_by(997) {
            assert!(
                (s.norm() - 1.0).abs() < 1e-4,
                "magnitude drifted: {}",
                s.norm()
            );
        }
    }

    #[test]
    fn staged_and_prime_decimations_preserve_counts_partitions_and_reset_alignment() {
        for d in [1, 2, 7, 10, 17, 160, 163, 4093] {
            let input: Vec<_> = (0..d * 8 + 97)
                .map(|n| {
                    let angle = std::f64::consts::TAU * n as f64 * 0.08 / d as f64;
                    Complex32::new(angle.cos() as f32, angle.sin() as f32)
                })
                .collect();
            let mut whole = make_decimator(d);
            let expected = whole.process(&input);
            assert_eq!(expected.len(), input.len().div_ceil(d));
            if d == 1 {
                assert_eq!(expected, input);
            }
            for chunk in [1, 37, 997] {
                let mut divided = make_decimator(d);
                let actual: Vec<_> = input
                    .chunks(chunk)
                    .flat_map(|chunk| divided.process(chunk))
                    .collect();
                assert_eq!(actual, expected, "d={d} chunk={chunk}");
            }
            // A discontinuity may start off the previous decimation grid.
            let first = (d - 3 % d) % d;
            whole.reset(first);
            let reset = whole.process(&input);
            let mut fresh = make_decimator(d);
            fresh.reset(first);
            assert_eq!(reset, fresh.process(&input));
            assert_eq!(reset.len(), input.len().saturating_sub(first).div_ceil(d));
        }
    }

    #[test]
    fn extreme_decimation_does_not_allocate_a_source_rate_sized_fir() {
        for d in [2_048_003, u32::MAX as usize] {
            let mut filter = make_decimator(d);
            let final_filter = filter.final_filter.as_ref().unwrap();
            assert!(final_filter.len <= 801);
            assert!(final_filter.banks.len() <= FILTER_PHASES + 1);
            let bytes = final_filter.len * final_filter.banks.len() * size_of::<f32>();
            assert!(
                bytes < 1024 * 1024,
                "unbounded coefficient allocation {bytes}"
            );
            assert_eq!(
                filter.process(&vec![Complex32::new(0.1, 0.0); 1000]).len(),
                1
            );
        }
    }

    fn streamed_response(d: usize, components: &[(f64, f64)], chunk: usize) -> Vec<Complex32> {
        let rate = 2_048_003;
        let offset = 128_000;
        let out_rate = f64::from(rate) / d as f64;
        let count = 256 * d;
        let mut channel = VirtualChannel::new(1, 100_000_000, rate, 100_128_000, d);
        let mut output = Vec::new();
        for first in (0..count).step_by(chunk) {
            let samples = (first..(first + chunk).min(count))
                .map(|n| {
                    components.iter().fold(
                        Complex32::new(0.0, 0.0),
                        |sum, &(relative, amplitude)| {
                            let angle = std::f64::consts::TAU
                                * (relative * out_rate + f64::from(offset))
                                * n as f64
                                / f64::from(rate);
                            sum + Complex32::new(
                                (angle.cos() * amplitude) as f32,
                                (angle.sin() * amplitude) as f32,
                            )
                        },
                    )
                })
                .collect();
            let mut block = wideband_block(100_000_000, rate, samples);
            block.start_sample = first as u64;
            block.product_session = 42;
            if let Some(block) = channel.process(&block) {
                assert_eq!(
                    block.capture_clock().source_sample_index,
                    output.len() as u64 * d as u64
                );
                assert_eq!(block.capture_clock().sample_stride, d as u32);
                output.extend_from_slice(&block.samples);
            }
        }
        assert_eq!(output.len(), 256);
        output
    }

    fn response_rms(samples: &[Complex32]) -> f64 {
        (samples.iter().map(|x| f64::from(x.norm_sqr())).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn rational_output_phases_follow_the_source_clock_with_constant_filter_delay() {
        for d in [2, 7, 17, 163, 4093] {
            let stages = if d < 8 { 0 } else { (d / 4usize).ilog2() };
            let factor = 1usize << stages;
            let taps = (100.0 * d as f64 / factor as f64).ceil() as usize | 1;
            // Half-band stages contribute 31 input samples each. The final
            // FIR contributes its midpoint at the reduced rate. Fractional
            // output phases must preserve this single physical delay.
            let delay = 31 * (factor - 1) + (taps - 1) / 2 * factor;
            for relative in [-0.4, -0.08, 0.08, 0.4] {
                let output = streamed_response(d, &[(relative, 0.5)], 997);
                for (n, actual) in output.iter().enumerate().skip(128) {
                    let angle =
                        std::f64::consts::TAU * relative * (n as f64 - delay as f64 / d as f64);
                    let expected =
                        Complex32::new((0.5 * angle.cos()) as f32, (0.5 * angle.sin()) as f32);
                    assert!((actual - expected).norm() < 0.0001,
                        "d={d} frequency={relative} sample={n} actual={actual:?} expected={expected:?}");
                }
            }
        }
    }

    #[test]
    fn streamed_filters_reject_aliases_and_strong_mixtures_including_interpolated_prime_phases() {
        for d in [2, 10, 163, 4093] {
            for fraction in [0.0, 0.4, 0.5, 1.08] {
                // D=2 cannot physically contain the beyond-Nyquist 1.08*Fsout tone.
                if fraction > 1.0 && d == 2 {
                    continue;
                }
                let output = streamed_response(d, &[(fraction, 0.5)], 4093);
                let db = 20.0 * (response_rms(&output[128..]) / 0.5).log10();
                if fraction <= 0.4 {
                    assert!(db.abs() < 0.02, "d={d} pass={db}");
                } else {
                    assert!(db < -80.0, "d={d} rejection={db}");
                }
            }
            if d == 2 {
                continue;
            }
            let reference = streamed_response(d, &[(0.08, 0.01)], 997);
            let actual = streamed_response(
                d,
                &[
                    (0.08, 0.01),
                    (1.08, 0.24),
                    (-0.92, 0.24),
                    (2.08, 0.24),
                    (-1.92, 0.24),
                ],
                4093,
            );
            let difference: Vec<_> = actual[128..]
                .iter()
                .zip(&reference[128..])
                .map(|(&a, &b)| a - b)
                .collect();
            let error = response_rms(&difference) / response_rms(&reference[128..]);
            assert!(error < 0.005, "d={d} relative mixture error={error}");
        }
    }

    #[test]
    fn design_lowpass_has_unity_dc_gain() {
        let taps = design_lowpass(63, 0.2, 0.0);
        let dc_gain: f32 = taps.iter().sum();
        assert!((dc_gain - 1.0).abs() < 1e-5, "DC gain was {dc_gain}");
    }

    #[test]
    fn fir_decimator_downsamples_by_expected_factor() {
        let mut dec = make_decimator(4);
        let input = vec![Complex32::new(1.0, 0.0); 4_000];
        let out = dec.process(&input);
        // 4000 / 4 = 1000, +/-1 for warm-up/decimation-phase rounding at the boundary.
        assert!(
            (out.len() as i64 - 1000).abs() <= 1,
            "got {} outputs",
            out.len()
        );
    }

    #[test]
    fn fir_decimator_is_continuous_across_chunk_boundaries() {
        let full_input: Vec<Complex32> = tone(2_000, 10.0, 1_000.0);

        let mut whole = make_decimator(5);
        let out_whole = whole.process(&full_input);

        let mut chunked = make_decimator(5);
        let mut out_chunks = Vec::new();
        for chunk in full_input.chunks(37) {
            out_chunks.extend(chunked.process(chunk));
        }

        assert_eq!(out_whole.len(), out_chunks.len());
        for (a, b) in out_whole.iter().zip(out_chunks.iter()) {
            assert!((a - b).norm() < 1e-4, "diverged: {a} vs {b}");
        }
    }

    #[test]
    fn fir_lowpass_attenuates_out_of_band_tone() {
        let sample_rate = 1_000.0;
        let mut dec = make_decimator(10); // output Nyquist 50 Hz

        let in_band = tone(2_000, 10.0, sample_rate);
        let out_of_band = tone(2_000, 400.0, sample_rate);

        let passed = dec.process(&in_band);
        let mut dec2 = make_decimator(10);
        let rejected = dec2.process(&out_of_band);

        let tail_rms = |v: &[Complex32]| {
            let tail = &v[v.len() / 2..];
            (tail.iter().map(|c| c.norm_sqr()).sum::<f32>() / tail.len() as f32).sqrt()
        };
        let passed_rms = tail_rms(&passed);
        let rejected_rms = tail_rms(&rejected);
        assert!(
            passed_rms > 0.7,
            "in-band tone over-attenuated: {passed_rms}"
        );
        assert!(
            rejected_rms < 0.1,
            "out-of-band tone insufficiently attenuated: {rejected_rms}"
        );
    }

    #[test]
    fn virtual_channel_process_shifts_and_decimates() {
        let mut ch = VirtualChannel::new(0, 100_000_000, 2_000_000, 100_050_000, 10);
        let block = wideband_block(100_000_000, 2_000_000, tone(2_000, 50_000.0, 2_000_000.0));

        let out = ch.process(&block).expect("expected decimated output");
        assert_eq!(out.center_freq_hz, 100_050_000);
        assert_eq!(out.sample_rate_hz, 200_000);
        assert!(
            (out.samples.len() as i64 - 200).abs() <= 2,
            "got {}",
            out.samples.len()
        );
    }

    #[test]
    fn channelizer_add_channel_rejects_out_of_nyquist_frequency() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let err = match chan.add_channel(150_000_000, 48_000) {
            Err(e) => e,
            Ok(_) => panic!("expected out-of-range channel to be rejected"),
        };
        assert!(err.to_string().contains("outside"));
    }

    #[test]
    fn channelizer_add_channel_rejects_zero_output_rate() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(chan.add_channel(100_000_000, 0).is_err());
    }

    #[test]
    fn channelizer_tick_routes_wideband_block_to_subscribed_channel() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (_, out_handle) = chan.add_channel(100_000_000, 100_000).unwrap();

        bus.publish(wideband_block(
            100_000_000,
            2_000_000,
            tone(4_000, 0.0, 2_000_000.0),
        ));
        let processed = chan.tick(Duration::from_millis(200));
        assert!(processed);

        let out = out_handle
            .recv_timeout(Duration::from_millis(200))
            .expect("expected decimated output on the channel bus");
        assert_eq!(out.center_freq_hz, 100_000_000);
    }

    #[test]
    fn channelizer_supports_multiple_independently_tuned_channels() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (_, a) = chan.add_channel(99_900_000, 200_000).unwrap();
        let (_, b) = chan.add_channel(100_100_000, 50_000).unwrap();
        assert_eq!(chan.channel_count(), 2);

        bus.publish(wideband_block(
            100_000_000,
            2_000_000,
            tone(8_000, 0.0, 2_000_000.0),
        ));
        assert!(chan.tick(Duration::from_millis(200)));

        let out_a = a
            .recv_timeout(Duration::from_millis(200))
            .expect("channel a output");
        let out_b = b
            .recv_timeout(Duration::from_millis(200))
            .expect("channel b output");
        assert_eq!(out_a.center_freq_hz, 99_900_000);
        assert_eq!(out_b.center_freq_hz, 100_100_000);
        assert_ne!(out_a.sample_rate_hz, out_b.sample_rate_hz);
    }

    #[test]
    fn channelizer_remove_channel_drops_it_from_registry() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        let (id, _out_handle) = chan.add_channel(100_000_000, 100_000).unwrap();
        assert_eq!(chan.channel_count(), 1);
        chan.remove_channel(id);
        assert_eq!(chan.channel_count(), 0);
    }

    #[test]
    fn retune_channel_recenters_while_preserving_the_same_bus_subscribers() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);

        // Subscribe BEFORE retune so we prove the existing bus handle survives the retune.
        let (id, out_handle) = chan.add_channel(100_000_000, 200_000).unwrap();
        let second = chan.subscribe(id, 4).expect("channel is live");

        chan.retune_channel(id, 100_050_000, 100_000).unwrap();
        assert_eq!(chan.channel_output_rate_hz(id), Some(100_000));
        // The second handle taken before retune must still receive output afterwards.
        let _ = second;

        bus.publish(wideband_block(
            100_000_000,
            2_000_000,
            tone(8_000, 0.0, 2_000_000.0),
        ));
        assert!(chan.tick(Duration::from_millis(200)));
        let out = out_handle
            .recv_timeout(Duration::from_millis(200))
            .expect("subscriber on the retained bus still gets output after retune");
        assert_eq!(out.center_freq_hz, 100_050_000);
    }

    #[test]
    fn retune_channel_rejects_unknown_id() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(chan.retune_channel(99, 100_000_000, 100_000).is_err());
    }

    #[test]
    fn subscribe_gives_an_additional_independent_handle_to_a_live_channel() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        let (id, first) = chan.add_channel(100_000_000, 200_000).unwrap();

        let second = chan.subscribe(id, 4).expect("channel is live");

        bus.publish(wideband_block(
            100_000_000,
            2_000_000,
            tone(8_000, 0.0, 2_000_000.0),
        ));
        assert!(chan.tick(Duration::from_millis(200)));

        let out_first = first
            .recv_timeout(Duration::from_millis(200))
            .expect("first subscriber output");
        let out_second = second
            .recv_timeout(Duration::from_millis(200))
            .expect("second subscriber output");
        assert_eq!(out_first.center_freq_hz, out_second.center_freq_hz);
    }

    #[test]
    fn subscribe_returns_none_for_an_unknown_channel_id() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(chan.subscribe(42, 4).is_none());
    }

    #[test]
    fn channelizer_tick_times_out_cleanly_when_idle() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert!(!chan.tick(Duration::from_millis(20)));
    }

    #[test]
    fn channelizer_run_exits_promptly_when_running_flag_clears() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        let running = AtomicBool::new(false);
        chan.run(&running); // must return immediately, not hang
    }

    #[test]
    fn wideband_center_hz_reflects_construction_and_retune() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert_eq!(chan.wideband_center_hz(), 100_000_000);
        chan.retune(101_000_000);
        assert_eq!(chan.wideband_center_hz(), 101_000_000);
    }

    #[test]
    fn channel_output_rate_hz_reports_actual_decimated_rate() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        // 2_000_000 / 100_000 requested -> decimation factor 20 -> exact 100_000 Hz actual.
        let (id, _out) = chan.add_channel(100_000_000, 100_000).unwrap();
        assert_eq!(chan.channel_output_rate_hz(id), Some(100_000));
    }

    #[test]
    fn channel_output_rate_hz_returns_none_for_unknown_channel() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        assert_eq!(chan.channel_output_rate_hz(42), None);
    }

    #[test]
    fn retune_updates_nco_offset_so_channel_still_tracks_its_absolute_frequency() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        // Channel is absolute 100_050_000 Hz, i.e. +50kHz offset while wideband center is
        // 100_000_000. After retuning the wideband center to 100_050_000, the channel's
        // offset should become ~0 (it *is* the new center), so a DC tone injected at the
        // wideband level should now survive the channel's low-pass essentially undamped.
        let (id, out_handle) = chan.add_channel(100_050_000, 200_000).unwrap();
        chan.retune(100_050_000);
        let _ = id;

        bus.publish(wideband_block(
            100_050_000,
            2_000_000,
            tone(4_000, 0.0, 2_000_000.0),
        ));
        assert!(chan.tick(Duration::from_millis(200)));
        let out = out_handle
            .recv_timeout(Duration::from_millis(200))
            .expect("expected decimated output after retune");
        let tail = &out.samples[out.samples.len() / 2..];
        let rms = (tail.iter().map(|c| c.norm_sqr()).sum::<f32>() / tail.len() as f32).sqrt();
        assert!(
            rms > 0.7,
            "expected near-undamped DC tone after retune, got rms {rms}"
        );
    }

    #[test]
    fn retune_preserves_channel_count_and_registry() {
        let bus = SampleBus::new();
        let handle = bus.subscribe(4, OverflowPolicy::DropIncoming);
        let mut chan = Channelizer::new(handle, 100_000_000, 2_000_000);
        chan.add_channel(99_900_000, 200_000).unwrap();
        chan.add_channel(100_100_000, 200_000).unwrap();
        assert_eq!(chan.channel_count(), 2);
        chan.retune(100_010_000);
        assert_eq!(chan.channel_count(), 2);
    }
}
