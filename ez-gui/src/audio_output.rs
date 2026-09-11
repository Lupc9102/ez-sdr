#[cfg(feature = "audio")]
mod audio_impl {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use crossbeam_channel::Receiver;
    use std::sync::{Arc, Mutex};

    pub enum AudioInputReceiver {
        Direct(Receiver<Vec<f32>>),
        Shared(Arc<Mutex<Receiver<Vec<f32>>>>),
    }

    impl From<Receiver<Vec<f32>>> for AudioInputReceiver {
        fn from(rx: Receiver<Vec<f32>>) -> Self {
            AudioInputReceiver::Direct(rx)
        }
    }

    impl From<Arc<Mutex<Receiver<Vec<f32>>>>> for AudioInputReceiver {
        fn from(rx: Arc<Mutex<Receiver<Vec<f32>>>>) -> Self {
            AudioInputReceiver::Shared(rx)
        }
    }

    pub struct AudioOutput {
        stream: Option<cpal::Stream>,
        sample_rate: u32,
        running: bool,
        failed: bool,
    }

    #[cfg(feature = "audio")]
    impl Default for AudioOutput {
        fn default() -> Self {
            Self::new()
        }
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

        #[must_use = "check if audio output started successfully"]
        pub fn start(&mut self, rx: impl Into<AudioInputReceiver>) -> Result<(), String> {
            if self.running {
                return Ok(());
            }

            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or("No audio output device found")?;
            let supported = device.default_output_config().map_err(|e| e.to_string())?;
            let sample_format = supported.sample_format();
            self.sample_rate = supported.sample_rate();
            let config: cpal::StreamConfig = supported.into();
            let channels = (config.channels as usize).max(1);

            let err_fn = |err| eprintln!("Audio error: {err}");

            let mut input_rx = rx.into();
            let mut ring_buffer = std::collections::VecDeque::<f32>::with_capacity(32768);

            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        match &mut input_rx {
                            AudioInputReceiver::Direct(r) => {
                                while let Ok(samples) = r.try_recv() {
                                    if ring_buffer.len() + samples.len() <= 96000 {
                                        ring_buffer.extend(samples);
                                    }
                                }
                            }
                            AudioInputReceiver::Shared(r) => {
                                if let Ok(guard) = r.try_lock() {
                                    while let Ok(samples) = guard.try_recv() {
                                        if ring_buffer.len() + samples.len() <= 96000 {
                                            ring_buffer.extend(samples);
                                        }
                                    }
                                }
                            }
                        }

                        let num_frames = data.len() / channels;
                        for frame_idx in 0..num_frames {
                            let sample = ring_buffer.pop_front().unwrap_or(0.0);
                            for ch in 0..channels {
                                data[frame_idx * channels + ch] = sample;
                            }
                        }
                        for s in &mut data[num_frames * channels..] {
                            *s = 0.0;
                        }
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_output_stream(
                    config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        match &mut input_rx {
                            AudioInputReceiver::Direct(r) => {
                                while let Ok(samples) = r.try_recv() {
                                    if ring_buffer.len() + samples.len() <= 96000 {
                                        ring_buffer.extend(samples);
                                    }
                                }
                            }
                            AudioInputReceiver::Shared(r) => {
                                if let Ok(guard) = r.try_lock() {
                                    while let Ok(samples) = guard.try_recv() {
                                        if ring_buffer.len() + samples.len() <= 96000 {
                                            ring_buffer.extend(samples);
                                        }
                                    }
                                }
                            }
                        }

                        let num_frames = data.len() / channels;
                        for frame_idx in 0..num_frames {
                            let sample = ring_buffer.pop_front().unwrap_or(0.0);
                            let sample_i16 = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            for ch in 0..channels {
                                data[frame_idx * channels + ch] = sample_i16;
                            }
                        }
                        for s in &mut data[num_frames * channels..] {
                            *s = 0;
                        }
                    },
                    err_fn,
                    None,
                ),
                _ => return Err(format!("Unsupported sample format: {sample_format:?}")),
            }
            .map_err(|e| e.to_string())?;

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
pub use audio_impl::{AudioInputReceiver, AudioOutput};

#[cfg(not(feature = "audio"))]
pub enum AudioInputReceiver {
    Direct(crossbeam_channel::Receiver<Vec<f32>>),
    Shared(std::sync::Arc<std::sync::Mutex<crossbeam_channel::Receiver<Vec<f32>>>>),
}

#[cfg(not(feature = "audio"))]
impl From<crossbeam_channel::Receiver<Vec<f32>>> for AudioInputReceiver {
    fn from(rx: crossbeam_channel::Receiver<Vec<f32>>) -> Self {
        AudioInputReceiver::Direct(rx)
    }
}

#[cfg(not(feature = "audio"))]
impl From<std::sync::Arc<std::sync::Mutex<crossbeam_channel::Receiver<Vec<f32>>>>>
    for AudioInputReceiver
{
    fn from(rx: std::sync::Arc<std::sync::Mutex<crossbeam_channel::Receiver<Vec<f32>>>>) -> Self {
        AudioInputReceiver::Shared(rx)
    }
}

#[cfg(not(feature = "audio"))]
pub struct AudioOutput {
    sample_rate: u32,
    running: bool,
    failed: bool,
}

#[cfg(not(feature = "audio"))]
impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn start(&mut self, _rx: impl Into<AudioInputReceiver>) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::AudioOutput;

    #[test]
    fn audio_output_new_defaults() {
        let ao = AudioOutput::new();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
        assert_eq!(ao.sample_rate(), 48000);
    }

    #[test]
    fn audio_output_stop_clears_state() {
        let mut ao = AudioOutput::new();
        ao.mark_failed();
        assert!(ao.has_failed());
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
    }

    #[test]
    fn audio_output_start_fails_without_feature() {
        let mut ao = AudioOutput::new();
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
        let result = ao.start(std::sync::Arc::new(std::sync::Mutex::new(rx)));
        #[cfg(feature = "audio")]
        {
            let _ = result;
        }
        #[cfg(not(feature = "audio"))]
        {
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("compiled"));
        }
    }

    #[test]
    fn audio_output_mark_failed() {
        let mut ao = AudioOutput::new();
        assert!(!ao.has_failed());
        ao.mark_failed();
        assert!(ao.has_failed());
    }

    #[test]
    fn audio_output_stop_idempotent() {
        let mut ao = AudioOutput::new();
        // Stop when already stopped should not panic
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
        // Stop again
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
    }

    #[test]
    fn audio_output_mark_failed_twice() {
        let mut ao = AudioOutput::new();
        ao.mark_failed();
        assert!(ao.has_failed());
        ao.mark_failed();
        assert!(ao.has_failed());
    }

    #[test]
    fn audio_output_mark_failed_after_stop() {
        let mut ao = AudioOutput::new();
        ao.mark_failed();
        assert!(ao.has_failed());
        ao.stop();
        assert!(!ao.has_failed());
        ao.mark_failed();
        assert!(ao.has_failed());
    }

    #[test]
    fn audio_output_stop_after_stop() {
        let mut ao = AudioOutput::new();
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
    }

    #[test]
    fn test_audio_output_start_stop() {
        let mut ao = AudioOutput::new();
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
        let _ = ao.start(std::sync::Arc::new(std::sync::Mutex::new(rx)));
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
    }

    #[test]
    fn test_audio_output_double_start() {
        let mut ao = AudioOutput::new();
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let _ = ao.start(rx.clone());
        let _ = ao.start(rx);
        ao.stop();
    }

    #[test]
    fn test_audio_output_stop_without_start() {
        let mut ao = AudioOutput::new();
        ao.stop();
        assert!(!ao.is_running());
        assert!(!ao.has_failed());
    }
}
