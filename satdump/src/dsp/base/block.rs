//! Block I/O types, base trait, runner, and simplified block wrappers.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;

use super::buffer::DspBuffer;
use super::stream::DspStream;
use anyhow::{bail, Result};

// ---------------------------------------------------------------------------
// I/O types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockIoType {
    Cf32,
    F32,
    S16,
    S8,
    U8,
}

impl BlockIoType {
    pub fn size_of(&self) -> usize {
        match self {
            BlockIoType::Cf32 => 8,
            BlockIoType::F32 => 4,
            BlockIoType::S16 => 2,
            BlockIoType::S8 => 1,
            BlockIoType::U8 => 1,
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            BlockIoType::Cf32 => "c",
            BlockIoType::F32 => "f",
            BlockIoType::S16 => "s",
            BlockIoType::S8 => "h",
            BlockIoType::U8 => "b",
        }
    }
}

/// I/O port descriptor.
#[derive(Clone)]
pub struct BlockIo {
    pub name: String,
    pub io_type: BlockIoType,
    pub stream: Option<Arc<DspStream>>,
    pub samplerate: u64,
    pub frequency: f64,
}

impl BlockIo {
    pub fn new(name: &str, io_type: BlockIoType) -> Self {
        Self {
            name: name.to_string(),
            io_type,
            stream: None,
            samplerate: 0,
            frequency: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Base trait & helpers
// ---------------------------------------------------------------------------

/// Core trait for every DSP block (source, sink, or transform).
///
/// Replaces `satdump::ndsp::Block`.  `work` is called in a loop on a
/// dedicated thread by a [`BlockRunner`].
pub trait Block: Send {
    fn id(&self) -> &str;
    fn inputs(&self) -> &[BlockIo];
    fn outputs(&self) -> &[BlockIo];
    fn inputs_mut(&mut self) -> &mut [BlockIo];
    fn outputs_mut(&mut self) -> &mut [BlockIo];

    fn init(&mut self) {}

    /// Main processing function.
    ///
    /// Return `true` when the block should exit (e.g. after receiving a
    /// propagating terminator).  `should_exit` is set by the runner when
    /// `stop(stop_now=true)` is called.
    fn work(&mut self, should_exit: &AtomicBool) -> bool;

    fn is_async(&self) -> bool {
        self.inputs().is_empty()
    }
}

pub fn set_input(block: &mut dyn Block, io: BlockIo, index: usize) -> Result<()> {
    if index >= block.inputs().len() {
        bail!("Input index {} does not exist for {}!", index, block.id());
    }
    if block.inputs()[index].io_type != io.io_type {
        bail!(
            "Input type mismatch for {} (expected {:?}, got {:?})",
            block.id(),
            block.inputs()[index].io_type,
            io.io_type
        );
    }
    block.inputs_mut()[index].stream = io.stream;
    Ok(())
}

pub fn get_output(block: &mut dyn Block, index: usize, nbuf: usize) -> Result<BlockIo> {
    if index >= block.outputs().len() {
        bail!("Output index {} does not exist for {}!", index, block.id());
    }
    if nbuf > 0 && block.outputs()[index].stream.is_none() {
        block.outputs_mut()[index].stream = Some(Arc::new(DspStream::new(nbuf)));
    }
    Ok(block.outputs()[index].clone())
}

pub fn link(
    input_block: &mut dyn Block,
    output_block: &mut dyn Block,
    output_index: usize,
    input_index: usize,
    nbuf: usize,
) -> Result<()> {
    get_output(output_block, output_index, nbuf)?;
    let out = output_block.outputs()[output_index].clone();
    set_input(input_block, out, input_index)
}

// ---------------------------------------------------------------------------
// BlockRunner
// ---------------------------------------------------------------------------

/// Threading harness for a [`Block`].
///
/// Replaces the thread-management portion of `satdump::ndsp::Block`.
pub struct BlockRunner {
    running: Arc<AtomicBool>,
    should_exit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl BlockRunner {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            should_exit: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub fn start(&mut self, mut block: Box<dyn Block>) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            bail!("Block runner already active");
        }
        block.init();
        self.running.store(true, Ordering::SeqCst);
        self.should_exit.store(false, Ordering::SeqCst);
        let running = self.running.clone();
        let should_exit = self.should_exit.clone();

        self.thread = Some(std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                if block.work(&should_exit) {
                    break;
                }
            }
        }));
        Ok(())
    }

    pub fn stop(&mut self, stop_now: bool, force: bool) {
        if stop_now {
            self.should_exit.store(true, Ordering::SeqCst);
        }
        if force {
            self.running.store(false, Ordering::SeqCst);
        }
        if let Some(th) = self.thread.take() {
            th.join().ok();
        }
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

pub trait SourceProcessor<To>: Send {
    fn process(&mut self, output: &mut [To]) -> usize;
    fn is_done(&self) -> bool { false }
}

/// Source block (0 inputs, 1 output).
pub struct SourceBlock<P, To> {
    id: String,
    inputs: Vec<BlockIo>,
    outputs: Vec<BlockIo>,
    processor: P,
    output_buffer_size: u32,
    _phantom: std::marker::PhantomData<To>,
}

impl<P, To> SourceBlock<P, To> {
    pub fn new(
        id: &str,
        processor: P,
        output_type: BlockIoType,
        output_buffer_size: u32,
    ) -> Self {
        Self {
            id: id.to_string(),
            inputs: vec![],
            outputs: vec![BlockIo::new("out", output_type)],
            processor,
            output_buffer_size,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<P, To> Block for SourceBlock<P, To>
where
    P: SourceProcessor<To> + Send,
    To: Copy + Send + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }
    fn inputs(&self) -> &[BlockIo] {
        &self.inputs
    }
    fn outputs(&self) -> &[BlockIo] {
        &self.outputs
    }
    fn inputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.inputs
    }
    fn outputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.outputs
    }

    fn work(&mut self, should_exit: &AtomicBool) -> bool {
        if should_exit.load(Ordering::Relaxed) {
            return true;
        }
        if self.processor.is_done() {
            let out_fifo = self.outputs[0].stream.as_ref().unwrap();
            out_fifo.enqueue(out_fifo.new_buffer_terminator(true));
            return true;
        }
        let out_fifo = self.outputs[0].stream.as_ref().unwrap();
        let mut oblk =
            out_fifo.new_buffer_samples(self.output_buffer_size, std::mem::size_of::<To>() as u8);
        let out_slice = oblk.as_mut_slice::<To>();
        let produced = self.processor.process(out_slice);
        oblk.size = produced as u32;
        if produced > 0 {
            out_fifo.enqueue(oblk);
        } else {
            out_fifo.free(oblk);
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Sink
// ---------------------------------------------------------------------------

pub trait SinkProcessor<Ti>: Send {
    /// Return `true` to signal termination.
    fn process(&mut self, input: &[Ti]) -> bool;
    fn finish(&mut self) {}
}

/// Sink block (1 input, 0 outputs).
pub struct SinkBlock<P, Ti> {
    id: String,
    inputs: Vec<BlockIo>,
    outputs: Vec<BlockIo>,
    processor: P,
    _phantom: std::marker::PhantomData<Ti>,
}

impl<P, Ti> SinkBlock<P, Ti> {
    pub fn new(id: &str, processor: P, input_type: BlockIoType) -> Self {
        Self {
            id: id.to_string(),
            inputs: vec![BlockIo::new("in", input_type)],
            outputs: vec![],
            processor,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<P, Ti> Block for SinkBlock<P, Ti>
where
    P: SinkProcessor<Ti> + Send,
    Ti: Copy + Send + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }
    fn inputs(&self) -> &[BlockIo] {
        &self.inputs
    }
    fn outputs(&self) -> &[BlockIo] {
        &self.outputs
    }
    fn inputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.inputs
    }
    fn outputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.outputs
    }

    fn work(&mut self, _should_exit: &AtomicBool) -> bool {
        let in_fifo = self.inputs[0].stream.as_ref().unwrap();
        let iblk = in_fifo.dequeue();

        if iblk.is_terminator() {
            self.processor.finish();
            in_fifo.free(iblk);
            return true;
        }

        let nsamples = iblk.size as usize;
        let in_slice = iblk.as_slice::<Ti>();
        let terminate = self.processor.process(&in_slice[..nsamples]);
        in_fifo.free(iblk);
        terminate
    }
}

// ---------------------------------------------------------------------------
// BlockSimple
// ---------------------------------------------------------------------------

pub trait SimpleProcessor<Ti, To>: Send {
    fn process(&mut self, input: &[Ti], output: &mut [To]) -> usize;
}

/// Single-input / single-output synchronous block.
///
/// Replaces `satdump::ndsp::BlockSimple<Ti, To>`.
pub struct BlockSimple<P, Ti, To> {
    id: String,
    inputs: Vec<BlockIo>,
    outputs: Vec<BlockIo>,
    processor: P,
    output_buffer_size_ratio: f32,
    _phantom: std::marker::PhantomData<(Ti, To)>,
}

impl<P, Ti, To> BlockSimple<P, Ti, To> {
    pub fn new(
        id: &str,
        processor: P,
        input_type: BlockIoType,
        output_type: BlockIoType,
    ) -> Self {
        Self {
            id: id.to_string(),
            inputs: vec![BlockIo::new("in", input_type)],
            outputs: vec![BlockIo::new("out", output_type)],
            processor,
            output_buffer_size_ratio: 1.0,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_ratio(mut self, ratio: f32) -> Self {
        self.output_buffer_size_ratio = ratio;
        self
    }
}

impl<P, Ti, To> Block for BlockSimple<P, Ti, To>
where
    P: SimpleProcessor<Ti, To> + Send,
    Ti: Copy + Send + 'static,
    To: Copy + Send + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }
    fn inputs(&self) -> &[BlockIo] {
        &self.inputs
    }
    fn outputs(&self) -> &[BlockIo] {
        &self.outputs
    }
    fn inputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.inputs
    }
    fn outputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.outputs
    }

    fn work(&mut self, _should_exit: &AtomicBool) -> bool {
        let in_fifo = self.inputs[0].stream.as_ref().unwrap();
        let iblk = in_fifo.dequeue();

        if iblk.is_terminator() {
            if iblk.terminator_should_propagate() {
                let out_fifo = self.outputs[0].stream.as_ref().unwrap();
                out_fifo.enqueue(out_fifo.new_buffer_terminator(true));
            }
            in_fifo.free(iblk);
            return true;
        }

        let nsamples = iblk.size as usize;
        let in_slice = iblk.as_slice::<Ti>();

        let out_max =
            ((iblk.max_size as f32) * self.output_buffer_size_ratio).ceil() as u32;
        let out_fifo = self.outputs[0].stream.as_ref().unwrap();
        let mut oblk = out_fifo.new_buffer_samples(out_max, std::mem::size_of::<To>() as u8);
        let out_slice = oblk.as_mut_slice::<To>();

        let produced = self.processor.process(&in_slice[..nsamples], out_slice);
        oblk.size = produced as u32;

        if oblk.size > 0 {
            out_fifo.enqueue(oblk);
        } else {
            out_fifo.free(oblk);
        }
        in_fifo.free(iblk);

        false
    }
}

// ---------------------------------------------------------------------------
// BlockSimpleMulti
// ---------------------------------------------------------------------------

pub trait SimpleMultiProcessor<Ti, To, const NI: usize, const NO: usize>: Send {
    fn process(
        &mut self,
        input: &[&[Ti]; NI],
        output: &mut [&mut [To]; NO],
        produced: &mut [usize; NO],
    );
}

/// Multi-input / multi-output synchronous block.
///
/// Replaces `satdump::ndsp::BlockSimpleMulti<Ti, To, Ni, No>`.
pub struct BlockSimpleMulti<P, Ti, To, const NI: usize, const NO: usize> {
    id: String,
    inputs: Vec<BlockIo>,
    outputs: Vec<BlockIo>,
    processor: P,
    output_buffer_size_ratio: [f32; NO],
    _phantom: std::marker::PhantomData<(Ti, To)>,
}

impl<P, Ti, To, const NI: usize, const NO: usize> BlockSimpleMulti<P, Ti, To, NI, NO> {
    pub fn new(
        id: &str,
        processor: P,
        input_types: [BlockIoType; NI],
        output_types: [BlockIoType; NO],
    ) -> Self {
        let inputs = input_types
            .iter()
            .enumerate()
            .map(|(i, t)| BlockIo::new(&format!("in{}", i), *t))
            .collect();
        let outputs = output_types
            .iter()
            .enumerate()
            .map(|(i, t)| BlockIo::new(&format!("out{}", i), *t))
            .collect();
        Self {
            id: id.to_string(),
            inputs,
            outputs,
            processor,
            output_buffer_size_ratio: [1.0; NO],
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_ratio(mut self, ratios: [f32; NO]) -> Self {
        self.output_buffer_size_ratio = ratios;
        self
    }
}

impl<P, Ti, To, const NI: usize, const NO: usize> Block for BlockSimpleMulti<P, Ti, To, NI, NO>
where
    P: SimpleMultiProcessor<Ti, To, NI, NO> + Send,
    Ti: Copy + Send + 'static,
    To: Copy + Send + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }
    fn inputs(&self) -> &[BlockIo] {
        &self.inputs
    }
    fn outputs(&self) -> &[BlockIo] {
        &self.outputs
    }
    fn inputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.inputs
    }
    fn outputs_mut(&mut self) -> &mut [BlockIo] {
        &mut self.outputs
    }

    fn work(&mut self, _should_exit: &AtomicBool) -> bool {
        let mut iblks: [DspBuffer; NI] = std::array::from_fn(|_| DspBuffer::new());
        let mut max_buf_size = 0u32;

        for i in 0..NI {
            let in_fifo = self.inputs[i].stream.as_ref().unwrap();
            iblks[i] = in_fifo.dequeue();

            if iblks[i].is_terminator() {
                if iblks[i].terminator_should_propagate() {
                    for o in 0..NO {
                        let out_fifo = self.outputs[o].stream.as_ref().unwrap();
                        out_fifo.enqueue(out_fifo.new_buffer_terminator(true));
                    }
                }
                // Free all already-dequeued buffers (C++ only freed the terminator).
                for j in 0..=i {
                    let f = self.inputs[j].stream.as_ref().unwrap();
                    f.free(std::mem::replace(&mut iblks[j], DspBuffer::new()));
                }
                return true;
            }

            if iblks[i].max_size > max_buf_size {
                max_buf_size = iblks[i].max_size;
            }
        }

        let mut oblks: [DspBuffer; NO] = std::array::from_fn(|_| DspBuffer::new());
        for o in 0..NO {
            let out_max =
                ((max_buf_size as f32) * self.output_buffer_size_ratio[o]).ceil() as u32;
            let out_fifo = self.outputs[o].stream.as_ref().unwrap();
            oblks[o] = out_fifo.new_buffer_samples(out_max, std::mem::size_of::<To>() as u8);
        }

        // Build slice arrays via raw pointers to avoid borrow-checker fights
        // with the mutable `oblks` array.
        let in_slices: [&[Ti]; NI] = std::array::from_fn(|i| unsafe {
            std::slice::from_raw_parts(
                iblks[i].as_slice::<Ti>().as_ptr(),
                iblks[i].size as usize,
            )
        });
        let mut out_slices: [&mut [To]; NO] = std::array::from_fn(|o| unsafe {
            std::slice::from_raw_parts_mut(
                oblks[o].as_mut_slice::<To>().as_mut_ptr(),
                oblks[o].max_size as usize,
            )
        });
        let mut produced = [0usize; NO];
        self.processor.process(&in_slices, &mut out_slices, &mut produced);

        for o in 0..NO {
            let mut oblk = std::mem::replace(&mut oblks[o], DspBuffer::new());
            oblk.size = produced[o] as u32;
            let out_fifo = self.outputs[o].stream.as_ref().unwrap();
            out_fifo.enqueue(oblk);
        }

        for i in 0..NI {
            let in_fifo = self.inputs[i].stream.as_ref().unwrap();
            in_fifo.free(std::mem::replace(&mut iblks[i], DspBuffer::new()));
        }

        false
    }
}
