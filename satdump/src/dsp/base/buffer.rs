//! DSP buffer types – safe replacement for `dsp_buffer.h`.

use std::mem::size_of;

/// Buffer content type (translated from `dsp_buffer_type_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferType {
    TerminatorPropagating = 0,
    TerminatorNonPropagating = 1,
    Samples = 2,
    Invalid = 255,
}

/// Owned sample buffer with type-erased storage.
///
/// Mirrors `satdump::ndsp::DSPBuffer`.  The internal `Vec<u8>` keeps the
/// payload; typed views are obtained via [`as_slice`](DspBuffer::as_slice)
/// / [`as_mut_slice`](DspBuffer::as_mut_slice).
pub struct DspBuffer {
    storage: Vec<u8>,
    pub type_: BufferType,
    pub type_size: u8,
    pub max_size: u32,
    pub size: u32,
}

impl Default for DspBuffer {
    fn default() -> Self {
        Self {
            storage: Vec::new(),
            type_: BufferType::Invalid,
            type_size: 0,
            max_size: 0,
            size: 0,
        }
    }
}

impl DspBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure at least `byte_size` bytes of backing storage.
    pub fn ensure_capacity(&mut self, byte_size: usize) {
        if self.storage.len() < byte_size {
            self.storage.resize(byte_size, 0);
        }
    }

    /// Byte length of the currently allocated storage.
    pub fn byte_capacity(&self) -> usize {
        self.storage.len()
    }

    /// Return a typed immutable view of the **active** samples
    /// (`self.size` elements, not the full capacity).
    pub fn as_slice<T: Copy>(&self) -> &[T] {
        assert_eq!(
            self.type_size as usize,
            size_of::<T>(),
            "type size mismatch"
        );
        let n = self.size as usize;
        assert!(self.storage.len() >= n * size_of::<T>());
        unsafe { std::slice::from_raw_parts(self.storage.as_ptr() as *const T, n) }
    }

    /// Return a typed mutable view of the **full allocated capacity**
    /// (`self.max_size` elements).
    pub fn as_mut_slice<T: Copy>(&mut self) -> &mut [T] {
        assert_eq!(
            self.type_size as usize,
            size_of::<T>(),
            "type size mismatch"
        );
        let n = self.max_size as usize;
        assert!(self.storage.len() >= n * size_of::<T>());
        unsafe { std::slice::from_raw_parts_mut(self.storage.as_mut_ptr() as *mut T, n) }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self.type_,
            BufferType::TerminatorPropagating | BufferType::TerminatorNonPropagating
        )
    }

    pub fn terminator_should_propagate(&self) -> bool {
        matches!(self.type_, BufferType::TerminatorPropagating)
    }
}
