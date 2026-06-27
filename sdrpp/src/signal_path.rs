//! Signal path / VFO manager - translated from core/src/signal_path/

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_channel::{bounded, Receiver, Sender};
use num_complex::Complex32 as Sample;
use rustfft::FftPlanner;

// ---------------------------------------------------------------------------
//  Shared types
// ---------------------------------------------------------------------------
pub type StereoSample = [f32; 2];

/// Simple event emitter used throughout the signal path.
pub struct Event<T> {
    handlers: Vec<Box<dyn FnMut(&T) + Send>>,
}

impl<T> Event<T> {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    pub fn bind_handler<F: FnMut(&T) + Send + 'static>(&mut self, handler: F) {
        self.handlers.push(Box::new(handler));
    }

    pub fn emit(&mut self, val: &T) {
        for h in &mut self.handlers {
            h(val);
        }
    }
}

impl<T> Default for Event<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
//  DSP helpers
// ---------------------------------------------------------------------------
fn blackman(i: usize, n: usize) -> f32 {
    if n <= 1 {
        return 1.0;
    }
    let a0 = 0.42_f32;
    let a1 = 0.5_f32;
    let a2 = 0.08_f32;
    let x = 2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0);
    a0 - a1 * x.cos() + a2 * (2.0 * x).cos()
}

fn nuttall(i: usize, n: usize) -> f32 {
    if n <= 1 {
        return 1.0;
    }
    let a0 = 0.355768_f32;
    let a1 = 0.487396_f32;
    let a2 = 0.144232_f32;
    let a3 = 0.012604_f32;
    let x = 2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0);
    a0 - a1 * x.cos() + a2 * (2.0 * x).cos() - a3 * (3.0 * x).cos()
}

fn gen_reshape_params(sample_rate: f64, fft_size: usize, fft_rate: f64) -> (usize, usize) {
    let fft_interval = (sample_rate / fft_rate).round() as usize;
    let nz_samp_count = fft_interval.min(fft_size);
    let skip = fft_interval - nz_samp_count;
    (skip, nz_samp_count)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FftWindow {
    Rectangular,
    Blackman,
    Nuttall,
}

/// Simple first-order IIR DC blocker.
///  y[n] = x[n] - x[n-1] + alpha * y[n-1]
struct DcBlocker {
    alpha: f32,
    xm1: Sample,
    ym1: Sample,
}

impl DcBlocker {
    fn new(rate: f64) -> Self {
        let alpha = (1.0 - 2.0 * std::f64::consts::PI * rate).clamp(0.0, 1.0) as f32;
        Self {
            alpha,
            xm1: Sample::new(0.0, 0.0),
            ym1: Sample::new(0.0, 0.0),
        }
    }

    fn reset(&mut self) {
        self.xm1 = Sample::new(0.0, 0.0);
        self.ym1 = Sample::new(0.0, 0.0);
    }

    fn process(&mut self, block: &mut [Sample]) {
        for s in block.iter_mut() {
            let y = *s - self.xm1 + self.alpha * self.ym1;
            self.xm1 = *s;
            self.ym1 = y;
            *s = y;
        }
    }
}

/// Averaging decimator.
struct PowerDecimator {
    ratio: usize,
    accum: Sample,
    count: usize,
}

impl PowerDecimator {
    fn new(ratio: usize) -> Self {
        Self {
            ratio,
            accum: Sample::new(0.0, 0.0),
            count: 0,
        }
    }

    fn set_ratio(&mut self, ratio: usize) {
        self.ratio = ratio;
        self.accum = Sample::new(0.0, 0.0);
        self.count = 0;
    }

    fn process(&mut self, input: &[Sample], output: &mut Vec<Sample>) {
        if self.ratio <= 1 {
            output.extend_from_slice(input);
            return;
        }
        for s in input {
            self.accum += *s;
            self.count += 1;
            if self.count >= self.ratio {
                output.push(self.accum / self.ratio as f32);
                self.count = 0;
                self.accum = Sample::new(0.0, 0.0);
            }
        }
    }

    fn flush(&mut self) {
        self.accum = Sample::new(0.0, 0.0);
        self.count = 0;
    }
}

// ---------------------------------------------------------------------------
//  FFT Buffer Provider
// ---------------------------------------------------------------------------
pub trait FftBufferProvider: Send {
    fn acquire(&mut self) -> Option<&mut [f32]>;
    fn release(&mut self);
}

// ---------------------------------------------------------------------------
//  VFO DSP (structural equivalent of dsp::channel::RxVFO)
// ---------------------------------------------------------------------------
pub struct VfoDsp {
    pub input_rate: f64,
    pub output_rate: f64,
    pub bandwidth: f64,
    pub offset: f64,
    pub running: bool,
}

impl VfoDsp {
    pub fn new(input_rate: f64, output_rate: f64, bandwidth: f64, offset: f64) -> Self {
        Self {
            input_rate,
            output_rate,
            bandwidth,
            offset,
            running: false,
        }
    }

    pub fn set_in_samplerate(&mut self, sr: f64) {
        self.input_rate = sr;
    }

    pub fn set_out_samplerate(&mut self, sr: f64, bandwidth: f64) {
        self.output_rate = sr;
        self.bandwidth = bandwidth;
    }

    pub fn set_offset(&mut self, offset: f64) {
        self.offset = offset;
    }

    pub fn set_bandwidth(&mut self, bw: f64) {
        self.bandwidth = bw;
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }
}

// ---------------------------------------------------------------------------
//  IQ Frontend
// ---------------------------------------------------------------------------
pub struct IqFrontend {
    sample_rate: f64,
    decim_ratio: usize,
    buffering: bool,
    dc_blocking: bool,
    invert_iq: bool,
    fft_size: usize,
    fft_rate: f64,
    fft_window_type: FftWindow,
    effective_sr: f64,

    // Processing blocks
    dc_blocker: DcBlocker,
    decimator: PowerDecimator,
    temp_decimated: Vec<Sample>,

    // Splitting
    vfo_inputs: HashMap<String, Sender<Arc<Vec<Sample>>>>,
    next_stream_id: AtomicUsize,
    bound_streams: Vec<(usize, Sender<Arc<Vec<Sample>>>)>,

    // FFT state
    nz_fft_size: usize,
    fft_skip: usize,
    fft_window: Vec<f32>,
    fft_in_buf: Vec<Sample>,
    fft_plan: Arc<dyn rustfft::Fft<f32>>,
    fft_provider: Box<dyn FftBufferProvider>,
    fft_accum: Vec<Sample>,
    fft_skip_left: usize,

    // Misc
    running: bool,
}

impl IqFrontend {
    pub fn new(
        sample_rate: f64,
        buffering: bool,
        decim_ratio: usize,
        dc_blocking: bool,
        fft_size: usize,
        fft_rate: f64,
        fft_window: FftWindow,
        fft_provider: Box<dyn FftBufferProvider>,
    ) -> Self {
        let effective_sr = sample_rate / decim_ratio.max(1) as f64;
        let mut planner = FftPlanner::new();
        let fft_plan = planner.plan_fft_forward(fft_size);
        let (skip, nz_fft_size) = gen_reshape_params(effective_sr, fft_size, fft_rate);
        let win_vec = Self::build_window(fft_window, nz_fft_size);

        let mut fft_in_buf = vec![Sample::new(0.0, 0.0); fft_size];
        for i in nz_fft_size..fft_size {
            fft_in_buf[i] = Sample::new(0.0, 0.0);
        }

        Self {
            sample_rate,
            decim_ratio,
            buffering,
            dc_blocking,
            invert_iq: false,
            fft_size,
            fft_rate,
            fft_window_type: fft_window,
            effective_sr,
            dc_blocker: DcBlocker::new(50.0 / effective_sr),
            decimator: PowerDecimator::new(decim_ratio),
            temp_decimated: Vec::new(),
            vfo_inputs: HashMap::new(),
            next_stream_id: AtomicUsize::new(1),
            bound_streams: Vec::new(),
            nz_fft_size,
            fft_skip: skip,
            fft_window: win_vec,
            fft_in_buf,
            fft_plan,
            fft_provider,
            fft_accum: Vec::new(),
            fft_skip_left: 0,
            running: false,
        }
    }

    fn build_window(typ: FftWindow, size: usize) -> Vec<f32> {
        let mut w = Vec::with_capacity(size);
        for i in 0..size {
            let base = match typ {
                FftWindow::Rectangular => 1.0,
                FftWindow::Blackman => blackman(i, size),
                FftWindow::Nuttall => nuttall(i, size),
            };
            // Spectrum centering shift: multiply by (-1)^i
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            w.push(base * sign);
        }
        w
    }

    // --- Public API ---

    pub fn set_buffering(&mut self, enabled: bool) {
        self.buffering = enabled;
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.effective_sr = sample_rate / self.decim_ratio.max(1) as f64;
        self.dc_blocker = DcBlocker::new(50.0 / self.effective_sr);
        self.update_fft_path(false);
    }

    pub fn set_decimation(&mut self, ratio: usize) {
        self.decim_ratio = ratio;
        self.decimator.set_ratio(ratio);
        self.set_sample_rate(self.sample_rate);
    }

    pub fn set_invert_iq(&mut self, enabled: bool) {
        self.invert_iq = enabled;
    }

    pub fn set_dc_blocking(&mut self, enabled: bool) {
        self.dc_blocking = enabled;
    }

    pub fn bind_iq_stream(&mut self, tx: Sender<Arc<Vec<Sample>>>) -> usize {
        let id = self.next_stream_id.fetch_add(1, Ordering::SeqCst);
        self.bound_streams.push((id, tx));
        id
    }

    pub fn unbind_iq_stream(&mut self, id: usize) {
        self.bound_streams.retain(|(i, _)| *i != id);
    }

    pub fn add_vfo(
        &mut self,
        name: &str,
        sample_rate: f64,
        bandwidth: f64,
        offset: f64,
    ) -> Option<(VfoDsp, Receiver<Arc<Vec<Sample>>>)> {
        if self.vfo_inputs.contains_key(name) {
            return None;
        }
        let (tx, rx) = bounded(1024);
        self.vfo_inputs.insert(name.to_string(), tx);
        let mut dsp = VfoDsp::new(self.effective_sr, sample_rate, bandwidth, offset);
        dsp.start();
        Some((dsp, rx))
    }

    pub fn remove_vfo(&mut self, name: &str) -> bool {
        self.vfo_inputs.remove(name).is_some()
    }

    pub fn set_fft_size(&mut self, size: usize) {
        self.fft_size = size;
        let mut planner = FftPlanner::new();
        self.fft_plan = planner.plan_fft_forward(size);
        self.update_fft_path(true);
    }

    pub fn set_fft_rate(&mut self, rate: f64) {
        self.fft_rate = rate;
        self.update_fft_path(false);
    }

    pub fn set_fft_window(&mut self, fft_window: FftWindow) {
        self.fft_window_type = fft_window;
        self.update_fft_path(false);
    }

    pub fn flush_input_buffer(&mut self) {
        self.fft_accum.clear();
        self.fft_skip_left = 0;
        self.decimator.flush();
        self.dc_blocker.reset();
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn get_effective_samplerate(&self) -> f64 {
        self.effective_sr
    }

    // --- Processing ---

    pub fn process_block(&mut self, input: &[Sample]) {
        if !self.running {
            return;
        }

        let mut stage: Vec<Sample> = input.to_vec();

        // Buffering is modelled by copying into a local Vec; in a zero-copy
        // pipeline this would be a ring buffer.  The temp copy keeps the logic
        // straightforward.
        //
        // Decimation
        if self.decim_ratio > 1 {
            self.temp_decimated.clear();
            self.decimator.process(&stage, &mut self.temp_decimated);
            stage = self.temp_decimated.clone();
        }

        // DC Blocking
        if self.dc_blocking {
            self.dc_blocker.process(&mut stage);
        }

        // IQ Inversion
        if self.invert_iq {
            for s in stage.iter_mut() {
                *s = s.conj();
            }
        }

        // Split to all consumers
        let shared = Arc::new(stage);
        self.split_to_vfos(&shared);
        self.feed_fft(&shared);
    }

    fn split_to_vfos(&self, block: &Arc<Vec<Sample>>) {
        for tx in self.vfo_inputs.values() {
            let _ = tx.try_send(block.clone());
        }
        for (_, tx) in &self.bound_streams {
            let _ = tx.try_send(block.clone());
        }
    }

    fn feed_fft(&mut self, block: &Arc<Vec<Sample>>) {
        for s in block.iter() {
            if self.fft_skip_left > 0 {
                self.fft_skip_left -= 1;
                continue;
            }
            self.fft_accum.push(*s);
            if self.fft_accum.len() >= self.nz_fft_size {
                self.run_fft();
                self.fft_accum.clear();
                self.fft_skip_left = self.fft_skip;
            }
        }
    }

    fn run_fft(&mut self) {
        // Apply window to first nz_fft_size samples
        for i in 0..self.nz_fft_size {
            let w = self.fft_window[i];
            self.fft_in_buf[i] = self.fft_accum[i] * w;
        }
        for i in self.nz_fft_size..self.fft_size {
            self.fft_in_buf[i] = Sample::new(0.0, 0.0);
        }

        // Execute FFT (in-place on fft_in_buf)
        self.fft_plan.process(&mut self.fft_in_buf);

        // Acquire waterfall buffer
        if let Some(buf) = self.fft_provider.acquire() {
            if buf.len() >= self.fft_size {
                let scale = self.fft_size as f32;
                let scale_db = 20.0 * scale.log10();
                for i in 0..self.fft_size {
                    let z = self.fft_in_buf[i];
                    let mag_sq = z.re * z.re + z.im * z.im;
                    let db = 10.0 * mag_sq.max(1e-20).log10() - scale_db;
                    buf[i] = db;
                }
            }
            self.fft_provider.release();
        }
    }

    fn update_fft_path(&mut self, _update_waterfall: bool) {
        let (skip, nz) = gen_reshape_params(self.effective_sr, self.fft_size, self.fft_rate);
        self.fft_skip = skip;
        self.nz_fft_size = nz;
        self.fft_window = Self::build_window(self.fft_window_type, nz);
        self.fft_in_buf.resize(self.fft_size, Sample::new(0.0, 0.0));
        for i in nz..self.fft_size {
            self.fft_in_buf[i] = Sample::new(0.0, 0.0);
        }
        self.fft_accum.clear();
        self.fft_skip_left = 0;
    }
}

// ---------------------------------------------------------------------------
//  VFO Manager
// ---------------------------------------------------------------------------
pub struct Vfo {
    name: String,
    reference: i32,
    offset: f64,
    center_offset: f64,
    bandwidth: f64,
    sample_rate: f64,
    min_bandwidth: f64,
    max_bandwidth: f64,
    bandwidth_locked: bool,
    pub color: u32,
    bandwidth_changed: bool,
}

impl Vfo {
    fn new(
        name: String,
        reference: i32,
        offset: f64,
        bandwidth: f64,
        sample_rate: f64,
        min_bandwidth: f64,
        max_bandwidth: f64,
        bandwidth_locked: bool,
    ) -> Self {
        Self {
            name,
            reference,
            offset,
            center_offset: offset,
            bandwidth,
            sample_rate,
            min_bandwidth,
            max_bandwidth,
            bandwidth_locked,
            color: 0xFFFFFFFF,
            bandwidth_changed: false,
        }
    }

    pub fn set_offset(&mut self, offset: f64) {
        self.offset = offset;
        self.center_offset = offset;
    }

    pub fn get_offset(&self) -> f64 {
        self.offset
    }

    pub fn set_center_offset(&mut self, offset: f64) {
        self.center_offset = offset;
    }

    pub fn set_bandwidth(&mut self, bandwidth: f64) {
        if (self.bandwidth - bandwidth).abs() < f64::EPSILON {
            return;
        }
        self.bandwidth = bandwidth;
        self.bandwidth_changed = true;
    }

    pub fn set_sample_rate(&mut self, sample_rate: f64, bandwidth: f64) {
        self.sample_rate = sample_rate;
        self.bandwidth = bandwidth;
    }

    pub fn set_reference(&mut self, reference: i32) {
        self.reference = reference;
    }

    pub fn set_bandwidth_limits(&mut self, min_bw: f64, max_bw: f64, locked: bool) {
        self.min_bandwidth = min_bw;
        self.max_bandwidth = max_bw;
        self.bandwidth_locked = locked;
    }

    pub fn get_bandwidth_changed(&mut self, erase: bool) -> bool {
        let val = self.bandwidth_changed;
        if erase {
            self.bandwidth_changed = false;
        }
        val
    }

    pub fn get_bandwidth(&self) -> f64 {
        self.bandwidth
    }

    pub fn get_reference(&self) -> i32 {
        self.reference
    }

    pub fn set_color(&mut self, color: u32) {
        self.color = color;
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

pub struct VfoManager {
    vfos: HashMap<String, Vfo>,
    pub on_vfo_created: Event<String>,
    pub on_vfo_delete: Event<String>,
    pub on_vfo_deleted: Event<String>,
}

impl Default for VfoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VfoManager {
    pub fn new() -> Self {
        Self {
            vfos: HashMap::new(),
            on_vfo_created: Event::new(),
            on_vfo_delete: Event::new(),
            on_vfo_deleted: Event::new(),
        }
    }

    pub fn create_vfo(
        &mut self,
        name: &str,
        reference: i32,
        offset: f64,
        bandwidth: f64,
        sample_rate: f64,
        min_bandwidth: f64,
        max_bandwidth: f64,
        bandwidth_locked: bool,
    ) -> Option<&mut Vfo> {
        if name.is_empty() || self.vfos.contains_key(name) {
            return None;
        }
        let vfo = Vfo::new(
            name.to_string(),
            reference,
            offset,
            bandwidth,
            sample_rate,
            min_bandwidth,
            max_bandwidth,
            bandwidth_locked,
        );
        self.vfos.insert(name.to_string(), vfo);
        self.on_vfo_created.emit(&name.to_string());
        self.vfos.get_mut(name)
    }

    pub fn delete_vfo(&mut self, name: &str) {
        if self.vfos.remove(name).is_some() {
            self.on_vfo_delete.emit(&name.to_string());
            self.on_vfo_deleted.emit(&name.to_string());
        }
    }

    pub fn set_offset(&mut self, name: &str, offset: f64) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_offset(offset);
        }
    }

    pub fn get_offset(&self, name: &str) -> f64 {
        self.vfos.get(name).map(|v| v.get_offset()).unwrap_or(0.0)
    }

    pub fn set_center_offset(&mut self, name: &str, offset: f64) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_center_offset(offset);
        }
    }

    pub fn set_bandwidth(&mut self, name: &str, bandwidth: f64) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_bandwidth(bandwidth);
        }
    }

    pub fn set_sample_rate(&mut self, name: &str, sample_rate: f64, bandwidth: f64) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_sample_rate(sample_rate, bandwidth);
        }
    }

    pub fn set_reference(&mut self, name: &str, reference: i32) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_reference(reference);
        }
    }

    pub fn set_bandwidth_limits(&mut self, name: &str, min_bw: f64, max_bw: f64, locked: bool) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_bandwidth_limits(min_bw, max_bw, locked);
        }
    }

    pub fn get_bandwidth_changed(&mut self, name: &str, erase: bool) -> bool {
        self.vfos
            .get_mut(name)
            .map(|v| v.get_bandwidth_changed(erase))
            .unwrap_or(false)
    }

    pub fn get_bandwidth(&self, name: &str) -> f64 {
        self.vfos
            .get(name)
            .map(|v| v.get_bandwidth())
            .unwrap_or(f64::NAN)
    }

    pub fn get_reference(&self, name: &str) -> i32 {
        self.vfos.get(name).map(|v| v.get_reference()).unwrap_or(-1)
    }

    pub fn set_color(&mut self, name: &str, color: u32) {
        if let Some(vfo) = self.vfos.get_mut(name) {
            vfo.set_color(color);
        }
    }

    pub fn vfo_exists(&self, name: &str) -> bool {
        self.vfos.contains_key(name)
    }

    pub fn iter_vfos(&self) -> impl Iterator<Item = &Vfo> {
        self.vfos.values()
    }
}

// ---------------------------------------------------------------------------
//  Sink Router
// ---------------------------------------------------------------------------
pub trait Sink: Send {
    fn start(&mut self);
    fn stop(&mut self);
}

struct NullSink;
impl Sink for NullSink {
    fn start(&mut self) {}
    fn stop(&mut self) {}
}

#[derive(Clone)]
pub struct SinkProvider {
    pub create: Arc<dyn Fn(&mut Stream) -> Box<dyn Sink> + Send + Sync>,
}

impl SinkProvider {
    pub fn null() -> Self {
        Self {
            create: Arc::new(|_stream: &mut Stream| -> Box<dyn Sink> {
                Box::new(NullSink)
            }),
        }
    }
}

pub struct Stream {
    pub name: String,
    input: Sender<Arc<Vec<StereoSample>>>,
    volume: f32,
    muted: bool,
    sample_rate: f32,
    pub provider_name: String,
    pub provider_id: usize,
    running: bool,
    gui_volume: f32,
    sink: Box<dyn Sink>,
    bound_sends: Vec<Sender<Arc<Vec<StereoSample>>>>,
    pub sr_change: Event<f32>,
}

impl Stream {
    pub fn new(name: &str, sample_rate: f32, provider: &SinkProvider) -> (Self, Receiver<Arc<Vec<StereoSample>>>) {
        let (tx, rx) = bounded(1024);
        let mut s = Self {
            name: name.to_string(),
            input: tx,
            volume: 1.0,
            muted: false,
            sample_rate,
            provider_name: "None".to_string(),
            provider_id: 0,
            running: false,
            gui_volume: 1.0,
            sink: Box::new(NullSink),
            bound_sends: Vec::new(),
            sr_change: Event::new(),
        };
        s.sink = (provider.create)(&mut s);
        (s, rx)
    }

    pub fn start(&mut self) {
        if self.running {
            return;
        }
        self.sink.start();
        self.running = true;
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        self.sink.stop();
        self.running = false;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.gui_volume = volume.clamp(0.0, 1.0);
        self.volume = if self.muted { 0.0 } else { self.gui_volume };
    }

    pub fn get_volume(&self) -> f32 {
        self.gui_volume
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.volume = if muted { 0.0 } else { self.gui_volume };
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.sr_change.emit(&sample_rate);
    }

    pub fn get_sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn bind_stream(&mut self) -> Receiver<Arc<Vec<StereoSample>>> {
        let (tx, rx) = bounded(1024);
        self.bound_sends.push(tx);
        rx
    }

    pub fn unbind_stream(&mut self, _rx: &Receiver<Arc<Vec<StereoSample>>>) {
        // crossbeam Receiver has no identity; in a production app you'd
        // wrap it in an Arc to enable ptr_eq removal.
    }

    pub fn feed_block(&mut self, block: Arc<Vec<StereoSample>>) {
        // Forward raw copies to bound streams (before volume).
        for tx in &self.bound_sends {
            let _ = tx.try_send(block.clone());
        }

        // Apply volume for the sink path
        let adjusted: Arc<Vec<StereoSample>> = if self.volume != 1.0 {
            let mut v = Vec::with_capacity(block.len());
            for [l, r] in block.iter() {
                v.push([l * self.volume, r * self.volume]);
            }
            Arc::new(v)
        } else {
            block
        };

        let _ = self.input.try_send(adjusted);
    }
}

pub struct SinkRouter {
    providers: HashMap<String, SinkProvider>,
    provider_names: Vec<String>,
    streams: HashMap<String, Stream>,
    stream_names: Vec<String>,

    pub on_sink_provider_registered: Event<String>,
    pub on_sink_provider_unregister: Event<String>,
    pub on_sink_provider_unregistered: Event<String>,
    pub on_stream_registered: Event<String>,
    pub on_stream_unregister: Event<String>,
    pub on_stream_unregistered: Event<String>,
}

impl Default for SinkRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl SinkRouter {
    pub fn new() -> Self {
        let mut router = Self {
            providers: HashMap::new(),
            provider_names: Vec::new(),
            streams: HashMap::new(),
            stream_names: Vec::new(),
            on_sink_provider_registered: Event::new(),
            on_sink_provider_unregister: Event::new(),
            on_sink_provider_unregistered: Event::new(),
            on_stream_registered: Event::new(),
            on_stream_unregister: Event::new(),
            on_stream_unregistered: Event::new(),
        };
        router.register_sink_provider("None", SinkProvider::null());
        router
    }

    pub fn register_sink_provider(&mut self, name: &str, provider: SinkProvider) {
        if self.providers.contains_key(name) {
            return;
        }
        self.providers.insert(name.to_string(), provider);
        self.provider_names.push(name.to_string());
        for stream in self.streams.values_mut() {
            stream.provider_id = self
                .provider_names
                .iter()
                .position(|n| n == &stream.provider_name)
                .unwrap_or(0);
        }
        self.on_sink_provider_registered.emit(&name.to_string());
    }

    pub fn unregister_sink_provider(&mut self, name: &str) {
        if name == "None" || !self.providers.contains_key(name) {
            return;
        }
        self.on_sink_provider_unregister.emit(&name.to_string());

        let affected: Vec<String> = self
            .streams
            .iter()
            .filter(|(_, s)| s.provider_name == name)
            .map(|(n, _)| n.clone())
            .collect();
        for stream_name in affected {
            self.set_stream_sink(&stream_name, "None");
        }

        self.providers.remove(name);
        self.provider_names.retain(|n| n != name);
        for stream in self.streams.values_mut() {
            stream.provider_id = self
                .provider_names
                .iter()
                .position(|n| n == &stream.provider_name)
                .unwrap_or(0);
        }
        self.on_sink_provider_unregistered.emit(&name.to_string());
    }

    pub fn register_stream(&mut self, name: &str, mut stream: Stream) {
        if self.streams.contains_key(name) {
            return;
        }
        let prov = self.providers.get("None").cloned().unwrap_or_else(SinkProvider::null);
        stream.sink = (prov.create)(&mut stream);
        stream.provider_id = self.provider_names.iter().position(|n| n == "None").unwrap_or(0);
        stream.provider_name = "None".to_string();
        self.streams.insert(name.to_string(), stream);
        self.stream_names.push(name.to_string());
        self.on_stream_registered.emit(&name.to_string());
    }

    pub fn unregister_stream(&mut self, name: &str) {
        if let Some(mut stream) = self.streams.remove(name) {
            self.on_stream_unregister.emit(&name.to_string());
            stream.stop();
            self.stream_names.retain(|n| n != name);
            self.on_stream_unregistered.emit(&name.to_string());
        }
    }

    pub fn start_stream(&mut self, name: &str) {
        if let Some(stream) = self.streams.get_mut(name) {
            stream.start();
        }
    }

    pub fn stop_stream(&mut self, name: &str) {
        if let Some(stream) = self.streams.get_mut(name) {
            stream.stop();
        }
    }

    pub fn get_stream_sample_rate(&self, name: &str) -> Option<f32> {
        self.streams.get(name).map(|s| s.get_sample_rate())
    }

    pub fn set_stream_sink(&mut self, name: &str, provider_name: &str) {
        let provider = match self.providers.get(provider_name).cloned() {
            Some(p) => p,
            None => return,
        };
        let stream = match self.streams.get_mut(name) {
            Some(s) => s,
            None => return,
        };

        stream.stop();
        stream.sink = (provider.create)(stream);
        stream.provider_name = provider_name.to_string();
        stream.provider_id = self
            .provider_names
            .iter()
            .position(|n| n == provider_name)
            .unwrap_or(0);
        if stream.running {
            stream.sink.start();
        }
    }

    pub fn get_stream(&mut self, name: &str) -> Option<&mut Stream> {
        self.streams.get_mut(name)
    }

    pub fn get_stream_names(&self) -> &[String] {
        &self.stream_names
    }

    pub fn bind_stream(&mut self, name: &str) -> Option<Receiver<Arc<Vec<StereoSample>>>> {
        self.streams.get_mut(name).map(|s| s.bind_stream())
    }

    pub fn unbind_stream(&mut self, name: &str, rx: &Receiver<Arc<Vec<StereoSample>>>) {
        if let Some(s) = self.streams.get_mut(name) {
            s.unbind_stream(rx);
        }
    }
}

// ---------------------------------------------------------------------------
//  Source Manager
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TuningMode {
    Normal,
    Panadapter,
}

pub trait SourceHandler: Send {
    fn menu_handler(&mut self);
    fn select_handler(&mut self);
    fn deselect_handler(&mut self);
    fn start_handler(&mut self);
    fn stop_handler(&mut self);
    fn tune_handler(&mut self, freq: f64);
}

pub struct SourceManager {
    sources: HashMap<String, Box<dyn SourceHandler>>,
    selected_name: Option<String>,
    tune_offset: f64,
    current_freq: f64,
    if_freq: f64,
    tune_mode: TuningMode,

    pub on_source_registered: Event<String>,
    pub on_source_unregister: Event<String>,
    pub on_source_unregistered: Event<String>,
    pub on_retune: Event<f64>,
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            selected_name: None,
            tune_offset: 0.0,
            current_freq: 0.0,
            if_freq: 0.0,
            tune_mode: TuningMode::Normal,
            on_source_registered: Event::new(),
            on_source_unregister: Event::new(),
            on_source_unregistered: Event::new(),
            on_retune: Event::new(),
        }
    }

    pub fn register_source(&mut self, name: &str, handler: Box<dyn SourceHandler>) {
        if self.sources.contains_key(name) {
            return;
        }
        self.sources.insert(name.to_string(), handler);
        self.on_source_registered.emit(&name.to_string());
    }

    pub fn unregister_source(&mut self, name: &str) {
        if let Some(mut handler) = self.sources.remove(name) {
            self.on_source_unregister.emit(&name.to_string());
            if self.selected_name.as_deref() == Some(name) {
                handler.deselect_handler();
                self.selected_name = None;
            }
            self.on_source_unregistered.emit(&name.to_string());
        }
    }

    pub fn get_source_names(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }

    pub fn select_source(&mut self, name: &str) {
        if !self.sources.contains_key(name) {
            return;
        }
        if let Some(prev) = &self.selected_name {
            if let Some(h) = self.sources.get_mut(prev) {
                h.deselect_handler();
            }
        }
        if let Some(h) = self.sources.get_mut(name) {
            h.select_handler();
            self.selected_name = Some(name.to_string());
        }
    }

    pub fn show_selected_menu(&mut self) {
        if let Some(name) = &self.selected_name {
            if let Some(h) = self.sources.get_mut(name) {
                h.menu_handler();
            }
        }
    }

    pub fn start(&mut self) {
        if let Some(name) = &self.selected_name {
            if let Some(h) = self.sources.get_mut(name) {
                h.start_handler();
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(name) = &self.selected_name {
            if let Some(h) = self.sources.get_mut(name) {
                h.stop_handler();
            }
        }
    }

    pub fn tune(&mut self, freq: f64) {
        if let Some(name) = &self.selected_name {
            if let Some(h) = self.sources.get_mut(name) {
                let target = match self.tune_mode {
                    TuningMode::Normal => (freq + self.tune_offset).abs(),
                    TuningMode::Panadapter => self.if_freq.abs(),
                };
                h.tune_handler(target);
                self.on_retune.emit(&(freq + self.tune_offset));
                self.current_freq = freq;
            }
        }
    }

    pub fn set_tuning_offset(&mut self, offset: f64) {
        self.tune_offset = offset;
        self.tune(self.current_freq);
    }

    pub fn set_tuning_mode(&mut self, mode: TuningMode) {
        self.tune_mode = mode;
        self.tune(self.current_freq);
    }

    pub fn set_panadapter_if(&mut self, freq: f64) {
        self.if_freq = freq;
        self.tune(self.current_freq);
    }
}
