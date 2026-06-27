//! DSP core — translated from `legacy_src/core/src/dsp/`
//!
//! Basic types (`Complex`, `Stereo`), double-buffered `Stream<T>`, the `Block`
//! trait, execution helpers (`BlockRunner`), plus `Chain` and `HierBlock`.

pub mod audio;
pub mod demod;
pub mod fft;
pub mod filter;
pub mod resampling;

use std::f32::consts::FRAC_PI_4;
use std::ops::{Add, AddAssign, Deref, DerefMut, Div, Mul, MulAssign, Sub, SubAssign};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

// ---------------------------------------------------------------------------
//  Typed sample buffers / basic DSP types
// ---------------------------------------------------------------------------

/// Complex float (f32) — equivalent of `dsp::complex_t`.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub fn conj(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    pub fn phase(self) -> f32 {
        self.im.atan2(self.re)
    }

    /// Fast approximation of `atan2(im, re)`.
    pub fn fast_phase(self) -> f32 {
        if self.re == 0.0 && self.im == 0.0 {
            return 0.0;
        }
        let abs_im = self.im.abs();
        let (r, angle) = if self.re >= 0.0 {
            let r = (self.re - abs_im) / (self.re + abs_im);
            (r, FRAC_PI_4 * (1.0 - r))
        } else {
            let r = (self.re + abs_im) / (abs_im - self.re);
            (r, FRAC_PI_4 * (3.0 - r))
        };
        let _ = r;
        if self.im < 0.0 {
            -angle
        } else {
            angle
        }
    }

    pub fn amplitude(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }

    /// Fast amplitude approximation (`alpha`-max + beta-min).
    pub fn fast_amplitude(self) -> f32 {
        let re_abs = self.re.abs();
        let im_abs = self.im.abs();
        if re_abs > im_abs {
            re_abs + 0.4 * im_abs
        } else {
            im_abs + 0.4 * re_abs
        }
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.im * rhs.re + self.re * rhs.im,
        }
    }
}

impl Mul<f32> for Complex {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            re: self.re * rhs,
            im: self.im * rhs,
        }
    }
}

impl Mul<f64> for Complex {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        let rhs = rhs as f32;
        Self {
            re: self.re * rhs,
            im: self.im * rhs,
        }
    }
}

impl Div<f32> for Complex {
    type Output = Self;
    fn div(self, rhs: f32) -> Self::Output {
        Self {
            re: self.re / rhs,
            im: self.im / rhs,
        }
    }
}

impl Div<f64> for Complex {
    type Output = Self;
    fn div(self, rhs: f64) -> Self::Output {
        let rhs = rhs as f32;
        Self {
            re: self.re / rhs,
            im: self.im / rhs,
        }
    }
}

impl AddAssign for Complex {
    fn add_assign(&mut self, rhs: Self) {
        self.re += rhs.re;
        self.im += rhs.im;
    }
}

impl SubAssign for Complex {
    fn sub_assign(&mut self, rhs: Self) {
        self.re -= rhs.re;
        self.im -= rhs.im;
    }
}

impl MulAssign<f32> for Complex {
    fn mul_assign(&mut self, rhs: f32) {
        self.re *= rhs;
        self.im *= rhs;
    }
}

/// Stereo float pair — equivalent of `dsp::stereo_t`.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Stereo {
    pub l: f32,
    pub r: f32,
}

impl Stereo {
    pub const fn new(l: f32, r: f32) -> Self {
        Self { l, r }
    }
}

impl Add for Stereo {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            l: self.l + rhs.l,
            r: self.r + rhs.r,
        }
    }
}

impl Sub for Stereo {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            l: self.l - rhs.l,
            r: self.r - rhs.r,
        }
    }
}

impl Mul<f32> for Stereo {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            l: self.l * rhs,
            r: self.r * rhs,
        }
    }
}

impl AddAssign for Stereo {
    fn add_assign(&mut self, rhs: Self) {
        self.l += rhs.l;
        self.r += rhs.r;
    }
}

impl SubAssign for Stereo {
    fn sub_assign(&mut self, rhs: Self) {
        self.l -= rhs.l;
        self.r -= rhs.r;
    }
}

impl MulAssign<f32> for Stereo {
    fn mul_assign(&mut self, rhs: f32) {
        self.l *= rhs;
        self.r *= rhs;
    }
}

impl std::iter::Sum for Stereo {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Stereo::new(0.0, 0.0), |a, b| Stereo {
            l: a.l + b.l,
            r: a.r + b.r,
        })
    }
}

// ---------------------------------------------------------------------------
//  Stream<T> — double-buffered, thread-safe, C++ semantics via safe Rust
// ---------------------------------------------------------------------------

/// Object-safe view of a stream so runners can stop/clear readers and writers.
pub trait UntypedStream: Send + Sync {
    fn stop_writer(&self);
    fn clear_writer_stop(&self);
    fn stop_reader(&self);
    fn clear_reader_stop(&self);
}

/// Internal state for the *writer* side of the double buffer.
struct SwapState {
    can_swap: bool,
    writer_stop: bool,
}

/// Internal state for the *reader* side of the double buffer.
struct RdyState {
    data_ready: bool,
    reader_stop: bool,
    data_size: usize,
}

/// Double-buffered inter-thread sample pipe.
///
/// Two `Mutex<Vec<T>>` buffers are used; at any moment one is the *write*
/// buffer and the other is the *read* buffer.  Because the two threads always
/// touch *different* buffers in the steady state, the mutex acquisitions are
/// uncontended and the design is effectively lock-free for the actual sample
/// memory (mirroring the original C++ pointer-swap logic).
pub struct Stream<T> {
    buf_a: Mutex<Vec<T>>,
    buf_b: Mutex<Vec<T>>,
    write_is_a: AtomicBool,
    swap: Mutex<SwapState>,
    swap_cv: Condvar,
    rdy: Mutex<RdyState>,
    rdy_cv: Condvar,
}

/// RAII guard for the current write buffer.
pub struct WriteGuard<'a, T> {
    guard: std::sync::MutexGuard<'a, Vec<T>>,
}

impl<'a, T> Deref for WriteGuard<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.guard[..]
    }
}

impl<'a, T> DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard[..]
    }
}

/// RAII guard for the current read buffer.
pub struct ReadGuard<'a, T> {
    guard: std::sync::MutexGuard<'a, Vec<T>>,
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.guard[..]
    }
}

impl<'a, T> DerefMut for ReadGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard[..]
    }
}

impl<T: Clone + Default> Stream<T> {
    /// Default capacity matches the original C++ `STREAM_BUFFER_SIZE`.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf_a: Mutex::new(vec![T::default(); capacity]),
            buf_b: Mutex::new(vec![T::default(); capacity]),
            write_is_a: AtomicBool::new(true),
            swap: Mutex::new(SwapState {
                can_swap: true,
                writer_stop: false,
            }),
            swap_cv: Condvar::new(),
            rdy: Mutex::new(RdyState {
                data_ready: false,
                reader_stop: false,
                data_size: 0,
            }),
            rdy_cv: Condvar::new(),
        }
    }

    pub fn new() -> Self {
        Self::with_capacity(1_000_000)
    }
}

impl<T> Stream<T> {
    /// Lock and return a mutable view of the current **write** buffer.
    ///
    /// Callers are expected to fill the buffer and then call [`Stream::swap`].
    pub fn write_buf(&self) -> WriteGuard<'_, T> {
        let guard = if self.write_is_a.load(Ordering::Relaxed) {
            self.buf_a.lock().unwrap()
        } else {
            self.buf_b.lock().unwrap()
        };
        WriteGuard { guard }
    }

    /// Lock and return a mutable view of the current **read** buffer.
    ///
    /// Only valid after a successful [`Stream::read`] and before [`Stream::flush`].
    pub fn read_buf(&self) -> ReadGuard<'_, T> {
        let guard = if self.write_is_a.load(Ordering::Relaxed) {
            self.buf_b.lock().unwrap()
        } else {
            self.buf_a.lock().unwrap()
        };
        ReadGuard { guard }
    }

    /// Writer call: hand the filled write buffer to the reader and flip buffers.
    ///
    /// Returns `false` if the writer has been stopped.
    pub fn swap(&self, size: usize) -> bool {
        let state = self.swap.lock().unwrap();
        let mut state = self
            .swap_cv
            .wait_while(state, |s| !s.can_swap && !s.writer_stop)
            .unwrap();
        if state.writer_stop {
            return false;
        }
        state.can_swap = false;
        self.write_is_a.fetch_xor(true, Ordering::Relaxed);
        drop(state);

        let mut rdy = self.rdy.lock().unwrap();
        rdy.data_size = size;
        rdy.data_ready = true;
        drop(rdy);
        self.rdy_cv.notify_all();
        true
    }

    /// Reader call: block until data is ready (or reader is stopped).
    ///
    /// Returns `None` on reader stop.
    pub fn read(&self) -> Option<usize> {
        let rdy = self.rdy.lock().unwrap();
        let rdy = self
            .rdy_cv
            .wait_while(rdy, |s| !s.data_ready && !s.reader_stop)
            .unwrap();
        if rdy.reader_stop {
            return None;
        }
        Some(rdy.data_size)
    }

    /// Reader call: release the read buffer and allow the writer to swap again.
    pub fn flush(&self) {
        let mut rdy = self.rdy.lock().unwrap();
        rdy.data_ready = false;
        drop(rdy);
        self.rdy_cv.notify_all();

        let mut state = self.swap.lock().unwrap();
        state.can_swap = true;
        drop(state);
        self.swap_cv.notify_all();
    }
}

impl<T: Send> UntypedStream for Stream<T> {
    fn stop_writer(&self) {
        let mut state = self.swap.lock().unwrap();
        state.writer_stop = true;
        drop(state);
        self.swap_cv.notify_all();
    }

    fn clear_writer_stop(&self) {
        let mut state = self.swap.lock().unwrap();
        state.writer_stop = false;
        drop(state);
        self.swap_cv.notify_all();
    }

    fn stop_reader(&self) {
        let mut rdy = self.rdy.lock().unwrap();
        rdy.reader_stop = true;
        drop(rdy);
        self.rdy_cv.notify_all();
    }

    fn clear_reader_stop(&self) {
        let mut rdy = self.rdy.lock().unwrap();
        rdy.reader_stop = false;
        drop(rdy);
        self.rdy_cv.notify_all();
    }
}

// ---------------------------------------------------------------------------
//  Block trait + execution helpers
// ---------------------------------------------------------------------------

/// Error returned from a block's run loop to signal termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    Stopped,
}

/// Core DSP block trait.
///
/// In C++ this was `generic_block` / `block`.  In Rust the *execution* side
/// (thread spawning) is split out into [`BlockRunner`]; the trait only
/// describes the sample-processing logic.
pub trait Block: Send {
    /// Single pass of the block's work function.
    ///
    /// Returning `Err(BlockError::Stopped)` causes the worker thread to exit.
    fn run(&mut self) -> Result<usize, BlockError>;

    /// All input streams this block reads from (used by runners to stop readers).
    fn inputs(&self) -> Vec<Arc<dyn UntypedStream>>;
    /// All output streams this block writes to (used by runners to stop writers).
    fn outputs(&self) -> Vec<Arc<dyn UntypedStream>>;
}

/// Idiomatic Rust equivalent of the C++ `block::doStart` / `doStop` machinery.
///
/// Wraps an `Arc<Mutex<dyn Block>>`, starts a thread that calls `run()` in a
/// loop, and cleanly tears everything down on `stop()`.
pub struct BlockRunner {
    running: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    inputs: Vec<Arc<dyn UntypedStream>>,
    outputs: Vec<Arc<dyn UntypedStream>>,
}

impl BlockRunner {
    pub fn new(inputs: Vec<Arc<dyn UntypedStream>>, outputs: Vec<Arc<dyn UntypedStream>>) -> Self {
        Self {
            running: AtomicBool::new(false),
            worker: Mutex::new(None),
            inputs,
            outputs,
        }
    }

    pub fn start(&self, block: Arc<Mutex<dyn Block>>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let b = Arc::clone(&block);
        *self.worker.lock().unwrap() = Some(thread::spawn(move || {
            loop {
                let res = { b.lock().unwrap().run() };
                if res.is_err() {
                    break;
                }
            }
        }));
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        for out in &self.outputs {
            out.stop_writer();
        }
        for input in &self.inputs {
            input.stop_reader();
        }
        if let Some(handle) = self.worker.lock().unwrap().take() {
            let _ = handle.join();
        }
        for input in &self.inputs {
            input.clear_reader_stop();
        }
        for out in &self.outputs {
            out.clear_writer_stop();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
//  Default run-loop helpers (C++ macro replacements)
// ---------------------------------------------------------------------------

/// Standard single-rate processor run loop.
///
/// Translates `OVERRIDE_PROC_RUN(process(count, in, out))`.
pub fn default_proc_run<I, O>(
    input: &Arc<Stream<I>>,
    output: &Arc<Stream<O>>,
    mut process: impl FnMut(usize, &[I], &mut [O]),
) -> Result<usize, BlockError> {
    let count = input.read().ok_or(BlockError::Stopped)?;
    {
        let in_buf = input.read_buf();
        let mut out_buf = output.write_buf();
        process(count, &in_buf[..count], &mut out_buf[..count]);
    }
    input.flush();
    if !output.swap(count) {
        return Err(BlockError::Stopped);
    }
    Ok(count)
}

/// Multirate processor run loop.
///
/// Translates `OVERRIDE_MULTIRATE_PROC_RUN(process(count, in, out))`.
pub fn default_multirate_proc_run<I, O>(
    input: &Arc<Stream<I>>,
    output: &Arc<Stream<O>>,
    mut process: impl FnMut(usize, &[I], &mut [O]) -> usize,
) -> Result<usize, BlockError> {
    let count = input.read().ok_or(BlockError::Stopped)?;
    let out_count = {
        let in_buf = input.read_buf();
        let mut out_buf = output.write_buf();
        process(count, &in_buf[..count], &mut out_buf[..count])
    };
    input.flush();
    if out_count > 0 {
        if !output.swap(out_count) {
            return Err(BlockError::Stopped);
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
//  Processor / Source / Sink helpers
// ---------------------------------------------------------------------------

/// Convenience struct wrapping the classic 1-input → 1-output DSP block.
pub struct Processor<I, O> {
    pub input: Arc<Stream<I>>,
    pub output: Arc<Stream<O>>,
}

impl<I: Clone + Default + Send + 'static, O: Clone + Default + Send + 'static> Processor<I, O> {
    pub fn new(input: Arc<Stream<I>>) -> Self {
        Self {
            input,
            output: Arc::new(Stream::new()),
        }
    }

    pub fn set_input(&mut self, input: Arc<Stream<I>>) {
        self.input = input;
    }

    pub fn runner(&self) -> BlockRunner {
        BlockRunner::new(
            vec![Arc::clone(&self.input) as Arc<dyn UntypedStream>],
            vec![Arc::clone(&self.output) as Arc<dyn UntypedStream>],
        )
    }
}

/// Convenience struct for a source (0-input → 1-output).
pub struct Source<T> {
    pub output: Arc<Stream<T>>,
}

impl<T: Clone + Default + Send + 'static> Source<T> {
    pub fn new() -> Self {
        Self {
            output: Arc::new(Stream::new()),
        }
    }

    pub fn runner(&self) -> BlockRunner {
        BlockRunner::new(
            vec![],
            vec![Arc::clone(&self.output) as Arc<dyn UntypedStream>],
        )
    }
}

/// Convenience struct for a sink (1-input → 0-output).
pub struct Sink<T> {
    pub input: Arc<Stream<T>>,
}

impl<T: Clone + Default + Send + 'static> Sink<T> {
    pub fn new(input: Arc<Stream<T>>) -> Self {
        Self { input }
    }

    pub fn set_input(&mut self, input: Arc<Stream<T>>) {
        self.input = input;
    }

    pub fn runner(&self) -> BlockRunner {
        BlockRunner::new(
            vec![Arc::clone(&self.input) as Arc<dyn UntypedStream>],
            vec![],
        )
    }
}

// ---------------------------------------------------------------------------
//  Chain<T>
// ---------------------------------------------------------------------------

/// Trait required for blocks that participate in a [`Chain`].
///
/// A chainable block has exactly one input and one output stream of the same
/// sample type, and its input can be rewired at runtime.
pub trait ChainableBlock<T>: Block {
    fn input_stream(&self) -> &Arc<Stream<T>>;
    fn output_stream(&self) -> &Arc<Stream<T>>;
    fn set_input(&mut self, input: Arc<Stream<T>>);
}

/// Linear chain of `ChainableBlock`s — equivalent of `dsp::chain`.
pub struct Chain<T> {
    input: Option<Arc<Stream<T>>>,
    pub output: Option<Arc<Stream<T>>>,
    links: Vec<Box<dyn ChainableBlock<T>>>,
    enabled: Vec<bool>,
    running: bool,
    runners: Vec<Option<BlockRunner>>,
}

impl<T: Send + 'static> Chain<T> {
    pub fn new() -> Self {
        Self {
            input: None,
            output: None,
            links: Vec::new(),
            enabled: Vec::new(),
            running: false,
            runners: Vec::new(),
        }
    }

    pub fn init(&mut self, input: Arc<Stream<T>>) {
        self.input = Some(Arc::clone(&input));
        self.output = Some(input);
    }

    pub fn add_block(&mut self, mut block: Box<dyn ChainableBlock<T>>, enabled: bool) {
        if let Some(prev_out) = self.output.clone() {
            block.set_input(prev_out);
        }
        if enabled {
            self.output = Some(Arc::clone(block.output_stream()));
            if self.running {
                let runner = BlockRunner::new(
                    vec![Arc::clone(block.input_stream()) as Arc<dyn UntypedStream>],
                    vec![Arc::clone(block.output_stream()) as Arc<dyn UntypedStream>],
                );
                self.runners.push(Some(runner));
            } else {
                self.runners.push(None);
            }
        } else {
            self.runners.push(None);
        }
        self.links.push(block);
        self.enabled.push(enabled);
    }

    pub fn remove_block(&mut self, idx: usize) {
        if idx >= self.links.len() {
            return;
        }
        if self.enabled[idx] {
            self.disable_block(idx);
        }
        self.links.remove(idx);
        self.enabled.remove(idx);
        self.runners.remove(idx);
    }

    pub fn enable_block(&mut self, idx: usize) {
        if idx >= self.links.len() || self.enabled[idx] {
            return;
        }
        let before = self.block_before(idx);
        let after = self.block_after(idx);
        let new_input = if let Some(before_idx) = before {
            Arc::clone(self.links[before_idx].output_stream())
        } else {
            self.input.clone().unwrap()
        };
        let out_stream = Arc::clone(self.links[idx].output_stream());
        self.links[idx].set_input(new_input);
        if let Some(after_idx) = after {
            self.links[after_idx].set_input(Arc::clone(&out_stream));
        } else {
            self.output = Some(Arc::clone(&out_stream));
        }
        if self.running {
            let runner = BlockRunner::new(
                vec![Arc::clone(self.links[idx].input_stream()) as Arc<dyn UntypedStream>],
                vec![Arc::clone(self.links[idx].output_stream()) as Arc<dyn UntypedStream>],
            );
            self.runners[idx] = Some(runner);
        }
        self.enabled[idx] = true;
    }

    pub fn disable_block(&mut self, idx: usize) {
        if idx >= self.links.len() || !self.enabled[idx] {
            return;
        }
        let before = self.block_before(idx);
        let after = self.block_after(idx);

        if let Some(after_idx) = after {
            let new_input = if let Some(before_idx) = before {
                Arc::clone(self.links[before_idx].output_stream())
            } else {
                self.input.clone().unwrap()
            };
            self.links[after_idx].set_input(new_input);
        } else {
            self.output = if let Some(before_idx) = before {
                Some(Arc::clone(self.links[before_idx].output_stream()))
            } else {
                self.input.clone()
            };
        }

        if let Some(runner) = self.runners[idx].take() {
            runner.stop();
        }
        self.enabled[idx] = false;
    }

    pub fn set_block_enabled(&mut self, idx: usize, enabled: bool) {
        if enabled {
            self.enable_block(idx);
        } else {
            self.disable_block(idx);
        }
    }

    pub fn enable_all_blocks(&mut self) {
        for i in 0..self.links.len() {
            self.enable_block(i);
        }
    }

    pub fn disable_all_blocks(&mut self) {
        for i in 0..self.links.len() {
            self.disable_block(i);
        }
    }

    pub fn start(&mut self) {
        if self.running {
            return;
        }
        for (i, block) in self.links.iter().enumerate() {
            if !self.enabled[i] {
                continue;
            }
            let runner = BlockRunner::new(
                vec![Arc::clone(block.input_stream()) as Arc<dyn UntypedStream>],
                vec![Arc::clone(block.output_stream()) as Arc<dyn UntypedStream>],
            );
            self.runners[i] = Some(runner);
        }
        self.running = true;
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        for runner in &mut self.runners {
            if let Some(r) = runner.take() {
                r.stop();
            }
        }
        self.running = false;
    }

    fn block_before(&self, idx: usize) -> Option<usize> {
        let mut prev = None;
        for (i, _) in self.links.iter().enumerate() {
            if i == idx {
                return prev;
            }
            if self.enabled[i] {
                prev = Some(i);
            }
        }
        None
    }

    fn block_after(&self, idx: usize) -> Option<usize> {
        let mut found = false;
        for (i, _) in self.links.iter().enumerate() {
            if i == idx {
                found = true;
                continue;
            }
            if found && self.enabled[i] {
                return Some(i);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
//  HierBlock
// ---------------------------------------------------------------------------

/// Equivalent of `dsp::hier_block` — a block that is itself composed of sub-blocks.
pub struct HierBlock {
    blocks: Vec<Arc<Mutex<dyn Block>>>,
    runners: Vec<BlockRunner>,
    running: bool,
}

impl HierBlock {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            runners: Vec::new(),
            running: false,
        }
    }

    pub fn register_block(&mut self, block: Arc<Mutex<dyn Block>>) {
        let b = Arc::clone(&block);
        let guard = b.lock().unwrap();
        let runner = BlockRunner::new(guard.inputs(), guard.outputs());
        drop(guard);
        self.blocks.push(block);
        self.runners.push(runner);
    }

    pub fn unregister_block(&mut self, idx: usize) {
        if idx < self.blocks.len() {
            self.blocks.remove(idx);
            self.runners.remove(idx);
        }
    }

    pub fn start(&mut self) {
        if self.running {
            return;
        }
        for (block, runner) in self.blocks.iter().zip(&self.runners) {
            runner.start(Arc::clone(block));
        }
        self.running = true;
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        for runner in &self.runners {
            runner.stop();
        }
        self.running = false;
    }
}
