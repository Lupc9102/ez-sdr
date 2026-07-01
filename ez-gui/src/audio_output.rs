#[cfg(feature = "audio")]
mod audio_impl {
    use std::sync::{Arc, Mutex};
    use crossbeam_channel::Receiver;
    use cpal::traits::{HostTrait, DeviceTrait, StreamTrait};

    pub struct AudioOutput {
        stream: Option<cpal::Stream>,
        sample_rate: u32,
        running: bool,
        failed: bool,
    }

    impl AudioOutput {
        pub fn new() -> Self {
            Self {
                stream: None,
                sample_rate: 48000,
                running: false,
                failed: false,
            }
        }

        pub fn start(&mut self, rx: Arc<Mutex<Receiver<Vec<f32>>>>) -> Result<(), String> {
            if self.running {
                return Ok(());
            }

            let host = cpal::default_host();
            let device = host.default_output_device().ok_or("No audio output device found")?;
            let supported = device.default_output_config().map_err(|e| e.to_string())?;
            let sample_format = supported.sample_format();
            self.sample_rate = supported.sample_rate().0;
            let config: cpal::StreamConfig = supported.into();

            let err_fn = |err| eprintln!("Audio error: {}", err);

            let stream = match sample_format {
                cpal::SampleFormat::F32 => {
                    let rx = rx.clone();
                    device.build_output_stream(
                        &config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            if let Ok(guard) = rx.try_lock() {
                                if let Ok(samples) = guard.try_recv() {
                                    let len = samples.len().min(data.len());
                                    data[..len].copy_from_slice(&samples[..len]);
                                    for s in &mut data[len..] {
                                        *s = 0.0;
                                    }
                                } else {
                                    for s in data.iter_mut() {
                                        *s = 0.0;
                                    }
                                }
                            } else {
                                for s in data.iter_mut() {
                                    *s = 0.0;
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::I16 => {
                    let rx = rx.clone();
                    device.build_output_stream(
                        &config,
                        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                            if let Ok(guard) = rx.try_lock() {
                                if let Ok(samples) = guard.try_recv() {
                                    let len = samples.len().min(data.len());
                                    for i in 0..len {
                                        data[i] = (samples[i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
                                    }
                                    for s in &mut data[len..] {
                                        *s = 0;
                                    }
                                } else {
                                    for s in data.iter_mut() {
                                        *s = 0;
                                    }
                                }
                            } else {
                                for s in data.iter_mut() {
                                    *s = 0;
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
            }.map_err(|e| e.to_string())?;

            stream.play().map_err(|e| e.to_string())?;
            self.stream = Some(stream);
            self.running = true;
            Ok(())
        }

        pub fn is_running(&self) -> bool {
            self.running
        }

        pub fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        pub fn has_failed(&self) -> bool {
            self.failed
        }

        pub fn mark_failed(&mut self) {
            self.failed = true;
        }

        pub fn stop(&mut self) {
            self.stream = None;
            self.running = false;
            self.failed = false;
        }
    }
}

#[cfg(feature = "audio")]
pub use audio_impl::AudioOutput;

#[cfg(not(feature = "audio"))]
pub struct AudioOutput {
    sample_rate: u32,
    running: bool,
    failed: bool,
}

#[cfg(not(feature = "audio"))]
impl AudioOutput {
    pub fn new() -> Self {
        Self {
            sample_rate: 48000,
            running: false,
            failed: false,
        }
    }

    pub fn start(&mut self, _rx: std::sync::Arc<std::sync::Mutex<crossbeam_channel::Receiver<Vec<f32>>>>) -> Result<(), String> {
        Err("Audio support not compiled in".to_string())
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn has_failed(&self) -> bool {
        self.failed
    }

    pub fn mark_failed(&mut self) {
        self.failed = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.failed = false;
    }
}
