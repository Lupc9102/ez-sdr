#![deny(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_code)]
//! Cancellable RTL read_async pump shared by daemon IQ and local GUI capture.
//! Bounded callback reads yield every 200 ms. Cancellation repeatedly signals
//! the native event loop and joins it before the owning caller closes the device.
use super::{callback_iq, stream_worker::StreamWorker, ReadPending};
use anyhow::Result;
use num_complex::Complex32;
use std::ffi::{c_int, c_void};
use std::sync::Arc;

#[link(name = "rtlsdr")]
extern "C" {
    fn rtlsdr_read_async(
        dev: *mut c_void,
        callback: Option<unsafe extern "C" fn(*mut u8, u32, *mut c_void)>,
        ctx: *mut c_void,
        buffers: u32,
        length: u32,
    ) -> c_int;
    fn rtlsdr_cancel_async(dev: *mut c_void) -> c_int;
}
#[derive(Clone, Copy)]
struct Device(*mut c_void);
// SAFETY: the pointer remains owned by the caller. Only the read worker and
// librtlsdr's cross-thread cancellation API use it until the worker is joined.
unsafe impl Send for Device {}
impl Device {
    unsafe fn run(self, producer: Arc<callback_iq::CallbackProducer>) -> Result<()> {
        let context = Arc::as_ptr(&producer).cast_mut().cast::<c_void>();
        // SAFETY: producer lives through read_async; callback buffers are copied
        // before return. Sixteen 32 KiB USB buffers satisfy the 512-byte multiple.
        let code = unsafe { rtlsdr_read_async(self.0, Some(callback), context, 16, 32768) };
        anyhow::ensure!(code >= 0, "RTL asynchronous USB read failed: {code}");
        Ok(())
    }
    unsafe fn cancel(self) {
        // SAFETY: caller retains the open device until the native worker exits.
        // -2 means it is not RUNNING yet/already cancelling; retry at the join.
        let _ = unsafe { rtlsdr_cancel_async(self.0) };
    }
}
unsafe extern "C" fn callback(bytes: *mut u8, length: u32, context: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        if bytes.is_null() || context.is_null() || length == 0 {
            return;
        }
        // SAFETY: read_async owns the live producer context and supplies length
        // readable bytes until this callback returns. push never waits for IQ consumers.
        let producer = unsafe { &*context.cast::<callback_iq::CallbackProducer>() };
        let bytes = unsafe { std::slice::from_raw_parts(bytes, length as usize) };
        producer.push(bytes);
    });
}

pub struct RtlAsyncStream {
    reader: callback_iq::CallbackReader,
    producer: Arc<callback_iq::CallbackProducer>,
    worker: StreamWorker,
    terminal: Option<Result<(), String>>,
}
impl RtlAsyncStream {
    /// Start a read worker on an already configured, open librtlsdr device.
    ///
    /// # Safety
    /// device must be valid and exclusively owned by the caller. Do not close,
    /// retune or otherwise access it until stop returns (Drop also stops/joins).
    /// The only concurrent device call is the documented cancel_async API.
    pub unsafe fn start(device: *mut c_void) -> Result<Self> {
        anyhow::ensure!(
            !device.is_null(),
            "RTL async stream requires an open device"
        );
        let device = Device(device);
        let (producer, reader) = callback_iq::channel(64, false);
        let run_producer = Arc::clone(&producer);
        // SAFETY: start's caller owns the device through the worker's lifetime.
        let worker = StreamWorker::spawn(
            move || unsafe { device.run(run_producer) },
            move || unsafe { device.cancel() },
        )?;
        Ok(Self {
            reader,
            producer,
            worker,
            terminal: None,
        })
    }
    fn after_read(&mut self, result: Result<usize>) -> Result<usize> {
        if matches!(
            result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<ReadPending>()),
            Some(ReadPending::Timeout)
        ) {
            if let Some(ended) = self.worker.finish_if_ready() {
                self.terminal = Some(ended.map_err(|error| error.to_string()));
            }
            if let Some(terminal) = &self.terminal {
                return terminal.clone().map(|()| 0).map_err(anyhow::Error::msg);
            }
        }
        result
    }
    pub fn read_iq(&mut self, output: &mut [Complex32], rate: u32) -> Result<usize> {
        let result = self.reader.read(output, rate);
        self.after_read(result)
    }
    /// Return a complete CU8 byte range. Acquisition clock indices count complex pairs.
    pub fn read_uc8(&mut self, output: &mut [u8], rate: u32) -> Result<usize> {
        let result = self.reader.read_uc8(output, rate);
        self.after_read(result)
    }
    #[must_use]
    pub fn capture_clock(&self) -> Option<ez_proto::CaptureClock> {
        self.reader.capture_clock()
    }
    #[must_use]
    pub fn dropped_samples(&self) -> u64 {
        self.producer.dropped_samples()
    }
    pub fn stop(&mut self) -> Result<()> {
        self.worker.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_callback_copies_usb_bytes_before_buffer_reuse() {
        let (producer, mut reader) = callback_iq::channel(1, false);
        let mut usb = (0..=255).collect::<Vec<u8>>();
        let expected = usb.clone();
        // SAFETY: buffer/context remain live throughout the injected native callback.
        unsafe {
            callback(
                usb.as_mut_ptr(),
                usb.len() as u32,
                Arc::as_ptr(&producer).cast_mut().cast(),
            );
        }
        usb.fill(0);
        let mut output = [0u8; 256];
        assert_eq!(reader.read_uc8(&mut output, 2_400_000).unwrap(), 256);
        assert_eq!(&output[..], &expected);
        assert_eq!(reader.capture_clock().unwrap().source_sample_index, 0);
    }
    #[test]
    fn asynchronous_failure_preserves_queued_prefix_then_reports_the_driver_error() {
        let (producer, reader) = callback_iq::channel(1, false);
        assert!(producer.push(&[12, 34, 56, 78]));
        let worker =
            StreamWorker::spawn(|| anyhow::bail!("injected native read failure"), || {}).unwrap();
        let mut stream = RtlAsyncStream {
            reader,
            producer,
            worker,
            terminal: None,
        };
        let mut output = [0u8; 16];
        assert_eq!(stream.read_uc8(&mut output, 2_400_000).unwrap(), 4);
        assert_eq!(&output[..4], &[12, 34, 56, 78]);
        let error = stream.read_uc8(&mut output, 2_400_000).unwrap_err();
        assert!(error.to_string().contains("native read failure"));
        stream.stop().unwrap();
    }
    #[test]
    fn silent_stream_read_yields_and_stop_joins_the_quiet_worker() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancelled = Arc::new(AtomicBool::new(false));
        let run_flag = Arc::clone(&cancelled);
        let stop_flag = Arc::clone(&cancelled);
        let (producer, reader) = callback_iq::channel(1, false);
        let worker = StreamWorker::spawn(
            move || {
                while !run_flag.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            move || stop_flag.store(true, Ordering::Release),
        )
        .unwrap();
        let mut stream = RtlAsyncStream {
            reader,
            producer,
            worker,
            terminal: None,
        };
        let start = std::time::Instant::now();
        let error = stream
            .read_iq(&mut [Complex32::default(); 8], 2_400_000)
            .unwrap_err();
        assert!(error.downcast_ref::<ReadPending>().is_some());
        stream.stop().unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(cancelled.load(Ordering::Acquire));
    }
}
