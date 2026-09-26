//! Radio demodulation modes and frequency-band identification.
//!
//! The receiver UI lives in [`crate::radio_ui`]. This module retains the
//! shared mode/data helpers used by the DSP, spectrum, scanner and bookmarks.

/// Supported demodulation modes for the SDR receiver.
///
/// Each variant represents a distinct demodulation scheme with specific
/// bandwidth, sound character, and use-case (AM voice, NFM land mobile,
/// WFM broadcast, SSB for weak-signal HF, RAW for digital decoders).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodMode {
    /// Pick the right mode from the tuned frequency automatically. Beginners
    /// never have to learn modulation — [`DemodMode::for_frequency`] maps the
    /// band to a concrete mode, and [`DemodMode::resolve`] applies it in the
    /// demod loop. Stored as the selected mode; never demodulated directly.
    Auto,
    /// Raw I/Q samples passed through without demodulation.
    Raw,
    /// Amplitude Modulation (8 kHz, used for aviation / AM broadcast).
    Am,
    /// Narrowband FM (12.5 kHz, used for land mobile / ham repeaters).
    Fm,
    /// Wideband FM (200 kHz, used for mono FM broadcast audio).
    Wfm,
    /// Lower Sideband (2.4 kHz, used for HF voice below 10 MHz).
    Lsb,
    /// Upper Sideband (2.4 kHz, used for HF voice above 10 MHz).
    Usb,
    /// Double Sideband coherent product detector (suppressed-carrier voice).
    Dsb,
    /// Continuous-wave Morse, mixed to a configurable audible beat tone.
    Cw,
}

impl DemodMode {
    /// Parse a [`DemodMode`] from a string label (case-sensitive).
    ///
    /// Accepts "AUTO", "RAW", "AM", "FM", "NFM" (→ `Fm`), "WFM", "LSB", "USB", "DSB", "CW".
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "AUTO" => Some(Self::Auto),
            "RAW" => Some(Self::Raw),
            "AM" => Some(Self::Am),
            "FM" | "NFM" => Some(Self::Fm),
            "WFM" => Some(Self::Wfm),
            "LSB" => Some(Self::Lsb),
            "USB" => Some(Self::Usb),
            "DSB" => Some(Self::Dsb),
            "CW" => Some(Self::Cw),
            _ => None,
        }
    }

    /// Return the uppercase string label for this demodulation mode.
    pub fn label(&self) -> &'static str {
        match self {
            DemodMode::Auto => "AUTO",
            DemodMode::Raw => "RAW",
            DemodMode::Am => "AM",
            DemodMode::Fm => "FM",
            DemodMode::Wfm => "WFM",
            DemodMode::Lsb => "LSB",
            DemodMode::Usb => "USB",
            DemodMode::Dsb => "DSB",
            DemodMode::Cw => "CW",
        }
    }

    /// RF channel width, independent of the demodulated audio low-pass cutoff.
    /// USB/LSB select this width on one side of the tuned carrier; other modes
    /// center the complete width around the carrier.
    pub fn default_rf_bandwidth_hz(self) -> f32 {
        match self {
            Self::Auto | Self::Fm => 12_500.0,
            Self::Wfm => 150_000.0,
            Self::Am => 10_000.0,
            Self::Dsb => 4_600.0,
            Self::Lsb | Self::Usb => 2_800.0,
            Self::Cw => 200.0,
            Self::Raw => 48_000.0,
        }
    }

    /// The concrete modulation used on a given frequency, chosen from the same
    /// band knowledge that powers [`identify_frequency`]. Used both to resolve
    /// [`DemodMode::Auto`] and to drive the "Auto → WFM" chip in Listen.
    pub fn for_frequency(freq_hz: u64) -> Self {
        let mhz = freq_hz as f64 / 1e6;
        match mhz {
            // HF: SSB voice — USB above 10 MHz, LSB below (amateur convention).
            f if f < 0.5 => DemodMode::Am,   // LF / longwave
            f if f < 1.71 => DemodMode::Am,  // AM broadcast (MW)
            f if f < 10.0 => DemodMode::Lsb, // 160/80/40m SSB, lower SW
            f if f < 30.0 => DemodMode::Usb, // 20m and up SSB / SW
            // VHF/UHF.
            f if f < 87.5 => DemodMode::Am, // 6m / aircraft edge / misc AM
            f if f < 108.0 => DemodMode::Wfm, // FM broadcast
            f if f < 137.0 => DemodMode::Am, // Aviation VHF (AM)
            f if f < 300.0 => DemodMode::Fm, // 2m, marine, weather, land mobile (NFM)
            f if f < 1000.0 => DemodMode::Fm, // 70cm / UHF land mobile (NFM)
            _ => DemodMode::Fm,             // default narrowband
        }
    }

    /// Resolve to a concrete mode for demodulation. Concrete modes pass through
    /// unchanged; [`DemodMode::Auto`] is mapped via [`DemodMode::for_frequency`].
    pub fn resolve(self, freq_hz: u64) -> Self {
        match self {
            DemodMode::Auto => DemodMode::for_frequency(freq_hz),
            other => other,
        }
    }
}

/// Information about a known frequency band identified by [`identify_frequency`].
#[derive(Debug, PartialEq)]
pub struct FreqIdInfo {
    /// Band name (e.g. "Aviation VHF", "FM Broadcast").
    pub band: &'static str,
    /// One-line summary of the band.
    pub short_desc: &'static str,
    /// Longer description covering sub-bands and typical uses.
    pub detail: &'static str,
    /// Demodulation and usage tips.
    pub tips: &'static str,
    /// Human-readable description of what audio to expect.
    pub what_to_hear: &'static str,
}

/// Look up a frequency in the built-in band database and return its
/// identification info, or `None` if the frequency is not recognised.
#[must_use]
pub fn identify_frequency(freq_hz: u64) -> Option<FreqIdInfo> {
    let entries: &[(u64, u64, &str, &str, &str, &str)] = &[
        (150_000,   500_000,   "LF/MF",         "Long & medium wave",
            "AM broadcast (MW), maritime beacons, time signals (DCF77/MSF).",
            "Use AM demod. Long-wave AM goes down to 150 kHz. DCF77 at 77.5 kHz carries atomic time."),
        (1_800_000, 3_500_000, "160m HF Amateur","Amateur 160m (Top Band)",
            "CW at 1.8 MHz, voice SSB from 1.84 MHz. Very long-range at night.",
            "Use LSB for voice. The built-in CW mode provides an audible beat note but does not transcribe Morse. Best reception after dark."),
        (3_500_000, 4_000_000, "80m HF Amateur", "Amateur 80m band",
            "Busy night band — CW, SSB voice, digital modes. Excellent DX at night.",
            "Use LSB. Expect crowded frequencies especially 3.5–3.8 MHz."),
        (7_000_000, 7_300_000, "40m HF Amateur", "Amateur 40m band",
            "CW and digital 7.0–7.07, SSB 7.1–7.3. Strong DX day and night.",
            "LSB below 10 MHz. FT8 digital at 7.074 MHz is very busy."),
        (10_000_000, 10_150_000, "30m HF Amateur", "Amateur 30m band",
            "CW and digital only. FT8 at 10.136 MHz. No phone allowed.",
            "USB. Narrow band — good for digital modes like FT8/FT4."),
        (14_000_000, 14_350_000, "20m HF Amateur", "Amateur 20m band",
            "Most popular HF amateur band. Excellent DX any time of day.",
            "USB above 10 MHz. FT8 at 14.074 MHz. SSB voice from 14.150 MHz."),
        (21_000_000, 21_450_000, "15m HF Amateur", "Amateur 15m band",
            "Good daytime DX, especially solar maximum. Opens to distant DX.",
            "USB. FT8 at 21.074 MHz. Active during day."),
        (24_890_000, 24_990_000, "12m HF Amateur", "Amateur 12m band",
            "Near full shortwave for DX. Best near solar maximum.",
            "USB. FT8 at 24.915 MHz."),
        (26_965_000, 27_405_000, "CB (Citizens Band)", "CB radio — 40 channels AM",
            "Truckers, 4x4 off-road, short-range comms. Channel 19 = 27.185 MHz trucker net.",
            "AM demod. Ch9 (27.065 MHz) is emergency channel. USB is used for DX on some channels."),
        (28_000_000, 29_700_000, "10m HF Amateur", "Amateur 10m band",
            "Excellent when solar cycle is active. Worldwide DX with modest antennas.",
            "USB. FT8 at 28.074 MHz. CW at 28.0–28.070 MHz."),
        (50_000_000, 54_000_000, "6m Amateur",    "Amateur 6m 'magic band'",
            "VHF sporadic-E propagation — can provide continent-wide DX unexpectedly.",
            "USB for voice/FT8. Known for surprise openings with low power."),
        (88_000_000, 108_000_000, "FM Broadcast", "Commercial FM radio (88–108 MHz)",
            "Mono music, news, and talk radio. WFM demod, wide 200 kHz BW.",
            "WFM mode, BW ~200 kHz. Stereo multiplex and optional RDS decoding are available when enabled."),
        (108_000_000, 118_000_000, "VOR/ILS",     "Aviation navigation aids",
            "VHF Omni-directional Range (VOR) and Instrument Landing System. Not voice.",
            "AM demod. These are navigation signals — you'll hear a morse identifier and tone."),
        (118_000_000, 137_000_000, "Aviation VHF", "Air Traffic Control (ATC)",
            "ATC talking to aircraft. Approach, ground, tower, ATIS, centre frequencies.",
            "AM demod. ATIS (airport weather) are automated — listen for your local airport."),
        (137_000_000, 138_000_000, "NOAA Satellites","Weather satellite downlinks",
            "Legacy polar weather-satellite downlink band. NOAA-15, NOAA-18, and NOAA-19 were decommissioned in 2025; do not expect their former APT transmissions.",
            "Check the current spacecraft/operator status before recording. EZ-SDR does not include a NOAA APT decoder."),
        (144_000_000, 148_000_000, "Amateur 2m",   "2-meter amateur radio band",
            "Most active VHF amateur band. FM repeaters, simplex, satellite links, weak-signal.",
            "NFM for voice. 144.0–144.1 MHz CW/SSB DX. 144.390 MHz is APRS."),
        (150_000_000, 156_000_000, "Land Mobile",  "Public safety, utilities, business",
            "Police, fire, taxis, railways. Mix of NFM voice and digital (DMR, P25).",
            "NFM. Digital signals sound like fast data/buzzing — need separate decoder."),
        (156_000_000, 162_050_000, "Marine VHF",   "Maritime communications",
            "Channel 16 (156.8 MHz) = international distress and hailing. Working channels 17–28.",
            "NFM. DSC digital safety calls on Ch70 (156.525 MHz)."),
        (162_400_000, 162_600_000, "NOAA WX Radio","US NOAA Weather Radio",
            "7 channels of continuous weather broadcasts, warnings, forecasts.",
            "Use NFM. Automated voice — very strong signal near transmitters."),
        (406_000_000, 406_100_000, "EPIRB/PLB",    "Emergency distress beacons (406 MHz)",
            "EPIRB and PLB satellite-linked emergency beacons. Narrow digital bursts.",
            "NFM. Should be silent unless a genuine emergency — do not transmit here."),
        (420_000_000, 450_000_000, "Amateur 70cm",  "70-centimeter amateur band",
            "FM repeaters, weak-signal EME, ATV, digital modes. Most active: 430–440 MHz.",
            "NFM for voice repeaters. 432.1 MHz SSB weak-signal DX. 433.0 MHz simplex."),
        (433_000_000, 435_000_000, "ISM 433 MHz",  "License-free ISM devices",
            "Car key fobs, wireless doorbells, weather stations, cheap sensors.",
            "NFM or RAW. Short OOK/FSK bursts — decode with rtl_433 tool."),
        (450_000_000, 470_000_000, "UHF LMR",      "UHF land mobile radio",
            "Business, public safety, taxis, transport. Mix of FM voice and digital.",
            "NFM. DMR, P25, NXDN digital systems sound like buzzing/data bursts."),
        (890_000_000, 960_000_000, "GSM 900",      "2G cellular (GSM)",
            "Legacy 2G voice/SMS. Uplink 890–915 MHz, downlink 935–960 MHz.",
            "RAW — encrypted. You'll see signal but can't decode content legally."),
        (1_090_000_000, 1_090_000_000, "ADS-B",    "Aircraft position transponders",
            "ADS-B 1090ES — aircraft broadcast position, altitude, speed. 1 second updates.",
            "Use the ADS-B tab in ez-sdr! RAW mode + 2.4 MSps. Works at 1090 ±1 MHz."),
        (1_215_000_000, 1_240_000_000, "L-band Radar","Radar altimeters, navigation",
            "Radar altimeters and L-band surveillance radars. Pulsed signals.",
            "RAW. Short bursts visible in the waterfall."),
        (1_525_000_000, 1_559_000_000, "L-band Sat","L-band satellite downlinks",
            "Inmarsat/Iridium voice, AERO aviation data, SCADA, MSS phones.",
            "WFM/NFM. Inmarsat AERO at 1.5465 GHz carries ATC/aircraft data."),
        (1_559_000_000, 1_610_000_000, "GPS/GNSS",  "GPS/GALILEO/GLONASS signals",
            "Navigation satellite signals. Very weak broadband BPSK. L1 at 1575.42 MHz.",
            "RAW with wide BW. Use dedicated GPS software — too weak for audio."),
        (1_626_000_000, 1_661_000_000, "Iridium",   "Iridium satellite phones",
            "Iridium NEXT LEO satellite constellation. Burst data, voice, IoT links.",
            "RAW or WFM. Bursts every ~90 seconds when satellites pass."),
        (1_694_000_000, 1_700_000_000, "GOES Sat",  "GOES weather satellite downlinks",
            "GOES-16/17/18 East/West at 1694.1 MHz: HRIT full-disk weather images.",
            "RAW, needs 2+ MSps and special decoder (goestools/SatDump)."),
    ];
    for &(lo, hi, band, short_desc, detail, tips) in entries {
        if freq_hz >= lo && freq_hz <= hi.max(lo) {
            let what_to_hear = what_to_hear_for_band(band);
            return Some(FreqIdInfo {
                band,
                short_desc,
                detail,
                tips,
                what_to_hear,
            });
        }
    }
    None
}

fn what_to_hear_for_band(band: &str) -> &'static str {
    match band {
        "FM Broadcast" => "music, news, or talk radio",
        "Aviation VHF" => "air traffic control voice (pilots + towers)",
        "Marine VHF" => "coast guard, ships, harbour calls",
        "NOAA WX Radio" => "automated weather forecast and alerts",
        "NOAA Satellites" => "a distinctive chirping APT image data signal",
        "Amateur 2m" => "amateur radio voice, APRS data bursts",
        "Amateur 70cm" => "amateur radio repeaters and digital modes",
        "Land Mobile" => "professional voice radio (police, fire, business)",
        "CB (Citizens Band)" => "truckers and CB radio operators",
        "LF/MF" => "AM broadcast stations or navigation beacons",
        "ADS-B" => "nothing audible — use the ADS-B tab to see aircraft",
        "VOR/ILS" => "a morse-code station identifier and nav tone",
        _ => "",
    }
}

/// Returns (`suggested_mode`, `band_name`, reason) if the frequency matches a well-known band
/// and the suggestion would differ from the current mode.
pub fn suggest_demod_for_freq(freq_hz: u64) -> Option<(DemodMode, &'static str, &'static str)> {
    let bands: &[(u64, u64, DemodMode, &str, &str)] = &[
        // HF amateur bands (LSB preferred below 10 MHz)
        (
            1_800_000,
            2_000_000,
            DemodMode::Lsb,
            "160m Band",
            "LSB for voice; use the built-in CW mode for an audible beat note",
        ),
        (
            3_500_000,
            4_000_000,
            DemodMode::Lsb,
            "80m Band",
            "LSB for voice/digital",
        ),
        (
            7_000_000,
            7_300_000,
            DemodMode::Lsb,
            "40m Band",
            "LSB for voice/FT8",
        ),
        (
            10_100_000,
            10_150_000,
            DemodMode::Usb,
            "30m Band",
            "USB for digital modes; use the built-in CW mode for an audible beat note",
        ),
        (
            14_000_000,
            14_350_000,
            DemodMode::Usb,
            "20m Band",
            "USB for voice/FT8 (most popular)",
        ),
        (
            21_000_000,
            21_450_000,
            DemodMode::Usb,
            "15m Band",
            "USB for voice/FT8",
        ),
        (
            24_890_000,
            24_990_000,
            DemodMode::Usb,
            "12m Band",
            "USB for voice/FT8",
        ),
        (
            28_000_000,
            29_700_000,
            DemodMode::Usb,
            "10m Band",
            "USB for voice/FT8; use the built-in CW mode for an audible beat note",
        ),
        (
            50_000_000,
            54_000_000,
            DemodMode::Usb,
            "6m Band",
            "USB for voice/FT8 (sporadic-E)",
        ),
        // Medium wave
        (
            150_000,
            500_000,
            DemodMode::Am,
            "LF/MF Band",
            "AM for beacons/time signals",
        ),
        (
            26_965_000,
            27_405_000,
            DemodMode::Am,
            "CB Radio",
            "AM for Citizens Band",
        ),
        // VHF/UHF
        (
            88_000_000,
            108_000_000,
            DemodMode::Wfm,
            "FM Broadcast",
            "WFM for commercial radio",
        ),
        (
            118_000_000,
            137_000_000,
            DemodMode::Am,
            "Aviation",
            "AM for air-to-ground voice",
        ),
        (
            137_000_000,
            138_000_000,
            DemodMode::Fm,
            "NOAA APT",
            "NFM for weather satellite",
        ),
        (
            144_000_000,
            148_000_000,
            DemodMode::Fm,
            "Amateur 2m",
            "NFM for repeaters/simplex",
        ),
        (
            150_000_000,
            156_000_000,
            DemodMode::Fm,
            "Land Mobile",
            "NFM for land mobile radio",
        ),
        (
            156_000_000,
            162_050_000,
            DemodMode::Fm,
            "Marine VHF",
            "NFM for ship/coast guard",
        ),
        (
            162_400_000,
            162_600_000,
            DemodMode::Fm,
            "NOAA Weather",
            "NFM for NOAA broadcasts",
        ),
        (
            406_000_000,
            406_100_000,
            DemodMode::Fm,
            "EPIRB/PLB",
            "NFM for emergency beacons",
        ),
        (
            420_000_000,
            450_000_000,
            DemodMode::Fm,
            "Amateur 70cm",
            "NFM for repeaters/digital",
        ),
        (
            433_000_000,
            435_000_000,
            DemodMode::Fm,
            "ISM 433 MHz",
            "NFM or RAW for sensor data",
        ),
        (
            450_000_000,
            470_000_000,
            DemodMode::Fm,
            "UHF LMR",
            "NFM for UHF land mobile",
        ),
        // Microwave/satellite
        (
            1_090_000_000,
            1_090_000_000,
            DemodMode::Raw,
            "ADS-B",
            "RAW for aircraft transponders",
        ),
    ];
    for &(lo, hi, mode, name, reason) in bands {
        if freq_hz >= lo && freq_hz <= hi.max(lo) {
            return Some((mode, name, reason));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsb_cw_labels_and_independent_channel_widths() {
        assert_eq!(DemodMode::from_label("DSB"), Some(DemodMode::Dsb));
        assert_eq!(DemodMode::from_label("CW"), Some(DemodMode::Cw));
        assert_eq!(DemodMode::Dsb.label(), "DSB");
        assert_eq!(DemodMode::Cw.label(), "CW");
        assert_eq!(DemodMode::Dsb.default_rf_bandwidth_hz(), 4_600.0);
        assert_eq!(DemodMode::Cw.default_rf_bandwidth_hz(), 200.0);
        assert_eq!(DemodMode::Wfm.default_rf_bandwidth_hz(), 150_000.0);
        assert_eq!(DemodMode::Usb.default_rf_bandwidth_hz(), 2_800.0);
        assert_eq!(DemodMode::Cw.resolve(7_030_000), DemodMode::Cw);
    }

    #[test]
    fn demod_mode_roundtrip() {
        for mode in &[
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Lsb,
            DemodMode::Usb,
        ] {
            let label = mode.label();
            let back = DemodMode::from_label(label);
            assert_eq!(back, Some(*mode));
        }
    }

    #[test]
    fn demod_mode_from_label_case_sensitive() {
        assert_eq!(DemodMode::from_label("raw"), None);
        assert_eq!(DemodMode::from_label("AM"), Some(DemodMode::Am));
        assert_eq!(DemodMode::from_label("FM"), Some(DemodMode::Fm));
        assert_eq!(DemodMode::from_label("NFM"), Some(DemodMode::Fm));
    }

    #[test]
    fn demod_mode_from_label_unknown() {
        assert_eq!(DemodMode::from_label("DIGITAL"), None);
        assert_eq!(DemodMode::from_label(""), None);
        assert_eq!(DemodMode::from_label("LSB"), Some(DemodMode::Lsb));
        assert_eq!(DemodMode::from_label("USB"), Some(DemodMode::Usb));
        assert_eq!(DemodMode::from_label("WFM"), Some(DemodMode::Wfm));
        assert_eq!(DemodMode::from_label("RAW"), Some(DemodMode::Raw));
    }

    #[test]
    fn demod_mode_from_label_whitespace() {
        assert_eq!(DemodMode::from_label(" AM"), None);
        assert_eq!(DemodMode::from_label("FM "), None);
    }

    #[test]
    fn demod_mode_label_all_variants() {
        assert_eq!(DemodMode::Raw.label(), "RAW");
        assert_eq!(DemodMode::Am.label(), "AM");
        assert_eq!(DemodMode::Fm.label(), "FM");
        assert_eq!(DemodMode::Wfm.label(), "WFM");
        assert_eq!(DemodMode::Lsb.label(), "LSB");
        assert_eq!(DemodMode::Usb.label(), "USB");
    }

    #[test]
    fn demod_mode_debug_and_clone() {
        let mode = DemodMode::Fm;
        let cloned = mode;
        assert_eq!(mode, cloned);
        assert_eq!(format!("{mode:?}"), "Fm");
    }

    #[test]
    fn identify_frequency_adsb() {
        let info = identify_frequency(1090000000);
        assert!(info.is_some());
    }

    #[test]
    fn identify_frequency_airband() {
        let info = identify_frequency(120000000);
        assert!(info.is_some(), "120 MHz ATC should be identified");
    }

    #[test]
    fn identify_frequency_fm_broadcast() {
        let info = identify_frequency(100000000);
        assert!(info.is_some(), "100 MHz FM broadcast should be identified");
    }

    #[test]
    fn identify_frequency_unknown_range() {
        assert_eq!(identify_frequency(1), None);
    }

    #[test]
    fn identify_frequency_marine_vhf() {
        let info = identify_frequency(156800000);
        assert!(info.is_some());
        assert_eq!(
            info.expect("156.8 MHz should identify as Marine VHF").band,
            "Marine VHF"
        );
    }

    #[test]
    fn identify_frequency_amateur_2m() {
        let info = identify_frequency(145500000);
        assert!(info.is_some());
        assert_eq!(
            info.expect("145.5 MHz should identify as Amateur 2m").band,
            "Amateur 2m"
        );
    }

    #[test]
    fn identify_frequency_noaa_wx() {
        let info = identify_frequency(162550000);
        assert!(info.is_some());
        // NOAA WX is adjacent to Marine VHF but outside its 156–162.05 MHz
        // allocation; accept either category if a future table changes order.
        let band = info
            .expect("162.55 MHz should identify as NOAA or Marine")
            .band;
        assert!(
            band == "NOAA WX Radio" || band == "Marine VHF",
            "expected NOAA WX Radio or Marine VHF, got {band}"
        );
    }

    #[test]
    fn identify_frequency_fm_broadcast_info() {
        let info = identify_frequency(100000000).expect("100 MHz should identify as FM Broadcast");
        assert_eq!(info.what_to_hear, "music, news, or talk radio");
    }

    #[test]
    fn identify_frequency_aviation_info() {
        let info = identify_frequency(120000000).expect("120 MHz should identify as Aviation");
        assert_eq!(
            info.what_to_hear,
            "air traffic control voice (pilots + towers)"
        );
    }

    #[test]
    fn suggest_demod_for_freq_adsb() {
        let result = suggest_demod_for_freq(1090000000);
        assert_eq!(
            result,
            Some((DemodMode::Raw, "ADS-B", "RAW for aircraft transponders"))
        );
    }

    #[test]
    fn suggest_demod_for_freq_airband() {
        let result = suggest_demod_for_freq(120000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Am));
    }

    #[test]
    fn suggest_demod_for_freq_unknown() {
        assert_eq!(suggest_demod_for_freq(999999), None);
    }

    #[test]
    fn suggest_demod_for_freq_fm_broadcast() {
        let result = suggest_demod_for_freq(100000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Wfm));
    }

    #[test]
    fn suggest_demod_for_freq_cb_radio() {
        let result = suggest_demod_for_freq(27000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Am));
    }

    #[test]
    fn suggest_demod_for_freq_noaa_sat() {
        let result = suggest_demod_for_freq(137620000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Fm));
    }

    #[test]
    fn suggest_demod_for_freq_marine() {
        let result = suggest_demod_for_freq(156800000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Fm));
    }

    #[test]
    fn suggest_demod_for_freq_hf_lsb() {
        let result = suggest_demod_for_freq(7000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Lsb));
    }

    #[test]
    fn suggest_demod_for_freq_hf_usb() {
        let result = suggest_demod_for_freq(14000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Usb));
    }

    #[test]
    fn suggest_demod_for_freq_amateur_70cm() {
        let result = suggest_demod_for_freq(435000000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Fm));
    }

    #[test]
    fn test_demod_mode_from_label_correctness() {
        for mode in &[
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Lsb,
            DemodMode::Usb,
        ] {
            let label = mode.label();
            let back = DemodMode::from_label(label);
            assert_eq!(back, Some(*mode), "round-trip failed for {mode:?}");
        }
        assert_eq!(DemodMode::from_label("NFM"), Some(DemodMode::Fm));
    }

    #[test]
    fn test_identify_frequency_band_categories() {
        // LF/MF (includes AM broadcast on medium wave, 150-500 kHz)
        let info = identify_frequency(300_000).expect("300 kHz should identify as LF/MF");
        assert_eq!(info.band, "LF/MF");

        // 160m HF Amateur
        let info =
            identify_frequency(1_850_000).expect("1.85 MHz should identify as 160m HF Amateur");
        assert_eq!(info.band, "160m HF Amateur");

        // 80m HF Amateur
        let info =
            identify_frequency(3_750_000).expect("3.75 MHz should identify as 80m HF Amateur");
        assert_eq!(info.band, "80m HF Amateur");

        // 40m HF Amateur
        let info =
            identify_frequency(7_150_000).expect("7.15 MHz should identify as 40m HF Amateur");
        assert_eq!(info.band, "40m HF Amateur");

        // 20m HF Amateur
        let info =
            identify_frequency(14_200_000).expect("14.2 MHz should identify as 20m HF Amateur");
        assert_eq!(info.band, "20m HF Amateur");

        // 15m HF Amateur
        let info =
            identify_frequency(21_200_000).expect("21.2 MHz should identify as 15m HF Amateur");
        assert_eq!(info.band, "15m HF Amateur");

        // 10m HF Amateur
        let info =
            identify_frequency(28_500_000).expect("28.5 MHz should identify as 10m HF Amateur");
        assert_eq!(info.band, "10m HF Amateur");

        // CB Radio
        let info = identify_frequency(27_185_000).expect("27.185 MHz should identify as CB Radio");
        assert_eq!(info.band, "CB (Citizens Band)");

        // VOR/ILS navigation
        let info = identify_frequency(112_000_000).expect("112 MHz should identify as VOR/ILS");
        assert_eq!(info.band, "VOR/ILS");

        // Aviation VHF
        let info =
            identify_frequency(120_000_000).expect("120 MHz should identify as Aviation VHF");
        assert_eq!(info.band, "Aviation VHF");

        // NOAA Satellites
        let info =
            identify_frequency(137_500_000).expect("137.5 MHz should identify as NOAA Satellites");
        assert_eq!(info.band, "NOAA Satellites");

        // FM Broadcast
        let info =
            identify_frequency(100_000_000).expect("100 MHz should identify as FM Broadcast");
        assert_eq!(info.band, "FM Broadcast");

        // Marine VHF
        let info =
            identify_frequency(156_800_000).expect("156.8 MHz should identify as Marine VHF");
        assert_eq!(info.band, "Marine VHF");

        // NOAA WX Radio (or Marine VHF due to overlap)
        let info =
            identify_frequency(162_550_000).expect("162.55 MHz should identify as NOAA or Marine");
        assert!(
            info.band == "NOAA WX Radio" || info.band == "Marine VHF",
            "expected NOAA WX Radio or Marine VHF, got {}",
            info.band
        );

        // Amateur 2m
        let info =
            identify_frequency(145_500_000).expect("145.5 MHz should identify as Amateur 2m");
        assert_eq!(info.band, "Amateur 2m");

        // Amateur 70cm
        let info =
            identify_frequency(435_000_000).expect("435 MHz should identify as Amateur 70cm");
        assert_eq!(info.band, "Amateur 70cm");

        // ISM 433 MHz overlaps with Amateur 70cm (420-450) and ISM (433-435);
        // first match wins → Amateur 70cm.
        let info =
            identify_frequency(434_000_000).expect("434 MHz should identify as Amateur 70cm");
        assert_eq!(info.band, "Amateur 70cm");

        // Land Mobile
        let info = identify_frequency(152_000_000).expect("152 MHz should identify as Land Mobile");
        assert_eq!(info.band, "Land Mobile");

        // GSM 900 (cellular)
        let info = identify_frequency(940_000_000).expect("940 MHz should identify as GSM 900");
        assert_eq!(info.band, "GSM 900");

        // ADS-B
        let info = identify_frequency(1_090_000_000).expect("1090 MHz should identify as ADS-B");
        assert_eq!(info.band, "ADS-B");

        // L-band Radar
        let info =
            identify_frequency(1_230_000_000).expect("1230 MHz should identify as L-band Radar");
        assert_eq!(info.band, "L-band Radar");

        // L-band Sat
        let info =
            identify_frequency(1_540_000_000).expect("1540 MHz should identify as L-band Sat");
        assert_eq!(info.band, "L-band Sat");

        // GPS/GNSS
        let info =
            identify_frequency(1_575_420_000).expect("1575.42 MHz should identify as GPS/GNSS");
        assert_eq!(info.band, "GPS/GNSS");

        // Iridium
        let info = identify_frequency(1_640_000_000).expect("1640 MHz should identify as Iridium");
        assert_eq!(info.band, "Iridium");

        // GOES Sat
        let info = identify_frequency(1_695_000_000).expect("1695 MHz should identify as GOES Sat");
        assert_eq!(info.band, "GOES Sat");

        // Verify all returned BandInfo have reasonable descriptions
        for freq in &[300_000, 7_150_000, 100_000_000, 120_000_000, 1_090_000_000] {
            let info = identify_frequency(*freq)
                .expect("each test frequency should identify successfully");
            assert!(
                !info.short_desc.is_empty(),
                "short_desc empty for freq {freq}"
            );
            assert!(!info.detail.is_empty(), "detail empty for freq {freq}");
            assert!(!info.tips.is_empty(), "tips empty for freq {freq}");
        }
    }

    #[test]
    fn test_suggest_demod_for_freq_more() {
        // DC (0 Hz) → None
        assert_eq!(suggest_demod_for_freq(0), None);

        // Very high frequency (100 GHz) → None
        assert_eq!(suggest_demod_for_freq(100_000_000_000), None);

        // LF/MF (300 kHz, like AM broadcast) → Am
        let result = suggest_demod_for_freq(300_000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Am));

        // NOAA satellite 137.9125 MHz → Fm
        let result = suggest_demod_for_freq(137_912_500);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Fm));

        // CB radio 27.185 MHz → Am
        let result = suggest_demod_for_freq(27_185_000);
        assert_eq!(result.map(|r| r.0), Some(DemodMode::Am));

        // Boundary just below first band → None
        assert_eq!(suggest_demod_for_freq(149_999), None);

        // Boundary at start of LF/MF band → Am
        assert_eq!(
            suggest_demod_for_freq(150_000).map(|r| r.0),
            Some(DemodMode::Am)
        );

        // Boundary at end of LF/MF band → Am
        assert_eq!(
            suggest_demod_for_freq(500_000).map(|r| r.0),
            Some(DemodMode::Am)
        );

        // Just beyond last mapped band → None
        assert!(suggest_demod_for_freq(1_700_000_000).is_none());
    }

    #[test]
    fn test_demod_mode_all_methods() {
        for mode in &[
            DemodMode::Raw,
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Lsb,
            DemodMode::Usb,
        ] {
            let label = mode.label();
            assert!(!label.is_empty());
            assert_eq!(DemodMode::from_label(label), Some(*mode));

            // Wrong case returns None
            let wrong_case = label.to_lowercase();
            assert_eq!(DemodMode::from_label(&wrong_case), None);

            // Clone derive works
            let cloned = *mode;
            assert_eq!(mode, &cloned);

            // Debug derive works
            let debug_str = format!("{mode:?}");
            assert!(!debug_str.is_empty());
        }
    }

    #[test]
    fn test_suggest_demod_for_freq_entire_map() {
        // Test every defined frequency band by its center frequency
        let known_freqs: &[(u64, DemodMode)] = &[
            (300_000, DemodMode::Am),        // LF/MF
            (1_900_000, DemodMode::Lsb),     // 160m
            (3_750_000, DemodMode::Lsb),     // 80m
            (7_150_000, DemodMode::Lsb),     // 40m
            (10_125_000, DemodMode::Usb),    // 30m
            (14_200_000, DemodMode::Usb),    // 20m
            (21_200_000, DemodMode::Usb),    // 15m
            (24_940_000, DemodMode::Usb),    // 12m
            (28_500_000, DemodMode::Usb),    // 10m
            (52_000_000, DemodMode::Usb),    // 6m
            (27_185_000, DemodMode::Am),     // CB Radio
            (100_000_000, DemodMode::Wfm),   // FM Broadcast
            (120_000_000, DemodMode::Am),    // Aviation
            (137_500_000, DemodMode::Fm),    // NOAA APT
            (145_000_000, DemodMode::Fm),    // Amateur 2m
            (153_000_000, DemodMode::Fm),    // Land Mobile
            (160_000_000, DemodMode::Fm),    // Marine VHF
            (162_500_000, DemodMode::Fm),    // NOAA Weather (overlaps Marine VHF; Fm wins)
            (406_050_000, DemodMode::Fm),    // EPIRB/PLB
            (435_000_000, DemodMode::Fm),    // Amateur 70cm
            (434_000_000, DemodMode::Fm),    // ISM 433 MHz
            (460_000_000, DemodMode::Fm),    // UHF LMR
            (1_090_000_000, DemodMode::Raw), // ADS-B
        ];
        for &(freq, expected_mode) in known_freqs {
            let result = suggest_demod_for_freq(freq);
            assert!(
                result.is_some(),
                "expected Some for freq {} Hz ({:.3} MHz)",
                freq,
                freq as f64 / 1e6
            );
            assert_eq!(
                result.map(|r| r.0),
                Some(expected_mode),
                "mode mismatch for {} Hz: expected {:?}, got {:?}",
                freq,
                expected_mode,
                result.map(|r| r.0)
            );
        }
    }
}
