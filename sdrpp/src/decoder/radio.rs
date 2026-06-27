use crate::dsp::demod::{AmDemod, AgcMode, FmDemod, SsbDemod, SsbMode, StereoSample, WfmDemod, WfmOutput};
use num_complex::Complex32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Raw,
    Am,
    Nfm,
    Wfm,
    Lsb,
    Usb,
    Dsb,
}

pub struct Driver {
    mode: Mode,
    am: Option<AmDemod>,
    nfm: Option<FmDemod>,
    wfm: Option<WfmDemod>,
    ssb: Option<SsbDemod>,
    sample_rate: f32,
    bandwidth: f32,
    agc_attack: f32,
    agc_decay: f32,
    low_pass: bool,
    stereo: bool,
    rds: bool,
}

impl Driver {
    pub fn new(mode: Mode, sample_rate: f32, bandwidth: f32) -> Self {
        let mut d = Self {
            mode,
            am: None,
            nfm: None,
            wfm: None,
            ssb: None,
            sample_rate,
            bandwidth,
            agc_attack: 50.0,
            agc_decay: 5.0,
            low_pass: true,
            stereo: false,
            rds: false,
        };
        d.rebuild();
        d
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.rebuild();
        }
    }

    pub fn set_bandwidth(&mut self, bandwidth: f32) {
        self.bandwidth = bandwidth;
        match self.mode {
            Mode::Am => {
                if let Some(ref mut demod) = self.am {
                    demod.set_bandwidth(bandwidth);
                }
            }
            Mode::Nfm => {
                if let Some(ref mut demod) = self.nfm {
                    demod.set_bandwidth(bandwidth);
                }
            }
            Mode::Wfm => {
                if let Some(ref mut demod) = self.wfm {
                    demod.set_deviation(bandwidth / 2.0);
                }
            }
            Mode::Lsb | Mode::Usb | Mode::Dsb => {
                if let Some(ref mut demod) = self.ssb {
                    demod.set_bandwidth(bandwidth);
                }
            }
            _ => {}
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.rebuild();
    }

    pub fn set_agc_attack(&mut self, attack: f32) {
        self.agc_attack = attack;
        if let Some(ref mut demod) = self.am {
            demod.reset();
        }
        if let Some(ref mut demod) = self.ssb {
            demod.reset();
        }
    }

    pub fn set_agc_decay(&mut self, decay: f32) {
        self.agc_decay = decay;
        if let Some(ref mut demod) = self.am {
            demod.reset();
        }
        if let Some(ref mut demod) = self.ssb {
            demod.reset();
        }
    }

    pub fn set_low_pass(&mut self, low_pass: bool) {
        self.low_pass = low_pass;
        if let Some(ref mut demod) = self.nfm {
            demod.set_low_pass(low_pass);
        }
        if let Some(ref mut demod) = self.wfm {
            demod.set_low_pass(low_pass);
        }
    }

    pub fn set_stereo(&mut self, stereo: bool) {
        self.stereo = stereo;
        if let Some(ref mut demod) = self.wfm {
            demod.set_stereo(stereo);
        }
    }

    pub fn set_rds(&mut self, rds: bool) {
        self.rds = rds;
        if let Some(ref mut demod) = self.wfm {
            demod.set_rds_out(rds);
        }
    }

    pub fn set_carrier_agc(&mut self, carrier: bool) {
        if let Some(ref mut demod) = self.am {
            let mode = if carrier { AgcMode::Carrier } else { AgcMode::Audio };
            demod.set_agc_mode(mode);
        }
    }

    fn rebuild(&mut self) {
        let sr = self.sample_rate;
        let bw = self.bandwidth;
        match self.mode {
            Mode::Am => {
                self.am = Some(AmDemod::new(
                    AgcMode::Audio,
                    bw,
                    self.agc_attack / sr,
                    self.agc_decay / sr,
                    100.0 / sr,
                    sr,
                ));
            }
            Mode::Nfm => {
                self.nfm = Some(FmDemod::new(sr, bw, self.low_pass));
            }
            Mode::Wfm => {
                self.wfm = Some(WfmDemod::new(
                    bw / 2.0,
                    sr,
                    self.stereo,
                    self.low_pass,
                    self.rds,
                ));
            }
            Mode::Lsb => {
                self.ssb = Some(SsbDemod::new(
                    SsbMode::Lsb,
                    bw,
                    sr,
                    self.agc_attack / sr,
                    self.agc_decay / sr,
                ));
            }
            Mode::Usb => {
                self.ssb = Some(SsbDemod::new(
                    SsbMode::Usb,
                    bw,
                    sr,
                    self.agc_attack / sr,
                    self.agc_decay / sr,
                ));
            }
            Mode::Dsb => {
                self.ssb = Some(SsbDemod::new(
                    SsbMode::Dsb,
                    bw,
                    sr,
                    self.agc_attack / sr,
                    self.agc_decay / sr,
                ));
            }
            Mode::Raw => {}
        }
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<StereoSample> {
        match self.mode {
            Mode::Raw => {
                input.iter().map(|&c| StereoSample::mono(c.re)).collect()
            }
            Mode::Am => {
                self.am.as_mut().map(|d| d.process_stereo(input)).unwrap_or_default()
            }
            Mode::Nfm => {
                self.nfm.as_mut().map(|d| d.process_stereo(input)).unwrap_or_default()
            }
            Mode::Wfm => {
                self.wfm.as_mut().map(|d| d.process(input).stereo).unwrap_or_default()
            }
            Mode::Lsb | Mode::Usb | Mode::Dsb => {
                self.ssb.as_mut().map(|d| d.process_stereo(input)).unwrap_or_default()
            }
        }
    }

    pub fn process_wfm(&mut self, input: &[Complex32]) -> Option<WfmOutput> {
        if self.mode != Mode::Wfm {
            return None;
        }
        self.wfm.as_mut().map(|d| d.process(input))
    }
}
