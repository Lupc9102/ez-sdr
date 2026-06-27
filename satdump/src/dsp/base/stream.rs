//! Blocking SPSC circular buffer and `DSPStream` replacement.

use std::sync::{Condvar, Mutex};

use super::buffer::{BufferType, DspBuffer};

// ---------------------------------------------------------------------------
// BlockingCircularBuffer
// ---------------------------------------------------------------------------

struct RingBuffer<T> {
    data: Vec<Option<T>>,
    head: usize,
    tail: usize,
    count: usize,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    fn new(capacity: usize) -> Self {
        let mut data = Vec::with_capacity(capacity);
        data.resize_with(capacity, || None);
        Self {
            data,
            head: 0,
            tail: 0,
            count: 0,
            capacity,
        }
    }

    fn push(&mut self, item: T) {
        self.data[self.tail] = Some(item);
        self.tail = (self.tail + 1) % self.capacity;
        self.count += 1;
    }

    fn pop(&mut self) -> Option<T> {
        if self.count == 0 {
            return None;
        }
        let item = self.data[self.head].take();
        self.head = (self.head + 1) % self.capacity;
        self.count -= 1;
        item
    }

    fn len(&self) -> usize {
        self.count
    }
}

/// Mutex + condvar backed circular queue.
///
/// Replaces `moodycamel::BlockingReaderWriterCircularBuffer`.
pub struct BlockingCircularBuffer<T> {
    inner: Mutex<RingBuffer<T>>,
    not_empty: Condvar,
    not_full: Condvar,
}

impl<T> BlockingCircularBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(RingBuffer::new(capacity)),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }
    }

    pub fn enqueue(&self, item: T) {
        let mut guard = self.inner.lock().unwrap();
        while guard.len() == guard.capacity {
            guard = self.not_full.wait(guard).unwrap();
        }
        guard.push(item);
        self.not_empty.notify_one();
    }

    pub fn try_enqueue(&self, item: T) -> Result<(), T> {
        let mut guard = self.inner.lock().unwrap();
        if guard.len() == guard.capacity {
            return Err(item);
        }
        guard.push(item);
        self.not_empty.notify_one();
        Ok(())
    }

    pub fn dequeue(&self) -> T {
        let mut guard = self.inner.lock().unwrap();
        while guard.len() == 0 {
            guard = self.not_empty.wait(guard).unwrap();
        }
        let item = guard.pop().unwrap();
        self.not_full.notify_one();
        item
    }

    pub fn try_dequeue(&self) -> Option<T> {
        let mut guard = self.inner.lock().unwrap();
        if guard.len() == 0 {
            return None;
        }
        let item = guard.pop();
        self.not_full.notify_one();
        item
    }

    pub fn size_approx(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn max_capacity(&self) -> usize {
        self.inner.lock().unwrap().capacity
    }
}

// ---------------------------------------------------------------------------
// DspStream
// ---------------------------------------------------------------------------

/// Bidirectional buffer-pool + data queue.
///
/// Replaces `satdump::ndsp::DSPStream`.  `ufifo` holds free buffers;
/// `fifo` holds buffers in flight between blocks.
pub struct DspStream {
    fifo: BlockingCircularBuffer<DspBuffer>,
    ufifo: BlockingCircularBuffer<DspBuffer>,
    max_capacity: usize,
}

impl DspStream {
    pub fn new(size: usize) -> Self {
        let fifo = BlockingCircularBuffer::new(size);
        let ufifo = BlockingCircularBuffer::new(size);
        for _ in 0..size {
            ufifo.enqueue(DspBuffer::new());
        }
        Self {
            fifo,
            ufifo,
            max_capacity: size,
        }
    }

    pub fn enqueue(&self, b: DspBuffer) {
        self.fifo.enqueue(b);
    }

    pub fn dequeue(&self) -> DspBuffer {
        self.fifo.dequeue()
    }

    pub fn try_dequeue(&self) -> Option<DspBuffer> {
        self.fifo.try_dequeue()
    }

    /// Pull a buffer from the unused pool, reallocating storage if necessary.
    pub fn alloc(&self, byte_size: usize) -> DspBuffer {
        let mut b = self.ufifo.dequeue();
        if b.byte_capacity() < byte_size {
            b.ensure_capacity(byte_size);
        }
        b
    }

    /// Return a buffer to the unused pool.
    pub fn free(&self, b: DspBuffer) {
        self.ufifo.enqueue(b);
    }

    pub fn size_approx(&self) -> usize {
        self.fifo.size_approx()
    }

    pub fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn new_buffer_terminator(&self, propagate: bool) -> DspBuffer {
        let mut b = self.alloc(0);
        b.type_ = if propagate {
            BufferType::TerminatorPropagating
        } else {
            BufferType::TerminatorNonPropagating
        };
        b
    }

    pub fn new_buffer_samples(&self, max_elements: u32, type_size: u8) -> DspBuffer {
        let byte_size = max_elements as usize * type_size as usize;
        let mut b = self.alloc(byte_size);
        b.type_ = BufferType::Samples;
        b.type_size = type_size;
        b.max_size = max_elements;
        b.size = 0;
        b
    }
}
