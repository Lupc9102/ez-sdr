//! Frequency presets database for quick access to common stations and services.
//!
//! Provides one-click tuning to popular frequencies: FM/AM stations, weather
//! radio, aircraft bands, ham repeaters, satellites, etc.

use serde::{Deserialize, Serialize};

/// A frequency preset with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyPreset {
    /// Display name (e.g. "BBC Radio 1", "NOAA Weather", "2m Calling").
    pub name: String,
    /// Frequency in Hz.
    pub frequency_hz: u64,
    /// Demodulation mode suggestion.
    pub mode: String,
    /// Category for filtering.
    pub category: PresetCategory,
    /// Optional description.
    pub description: String,
    /// Optional location/region (e.g. "UK", "US-CA", "Global").
    pub region: Option<String>,
}

/// Preset categories for organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetCategory {
    /// FM broadcast (88-108 MHz).
    FmBroadcast,
    /// AM broadcast (530-1700 kHz).
    AmBroadcast,
    /// Weather radio (NOAA, etc).
    Weather,
    /// Aircraft (ATC, ADS-B).
    Aircraft,
    /// Amateur radio.
    HamRadio,
    /// Satellites (NOAA APT, ham sats).
    Satellite,
    /// Marine VHF.
    Marine,
    /// Public service (police, fire, EMS).
    PublicService,
    /// Other/misc.
    Other,
}

impl PresetCategory {
    pub fn label(&self) -> &'static str {
        match self {
            PresetCategory::FmBroadcast => "FM Radio",
            PresetCategory::AmBroadcast => "AM Radio",
            PresetCategory::Weather => "Weather",
            PresetCategory::Aircraft => "Aircraft",
            PresetCategory::HamRadio => "Ham Radio",
            PresetCategory::Satellite => "Satellites",
            PresetCategory::Marine => "Marine",
            PresetCategory::PublicService => "Public Service",
            PresetCategory::Other => "Other",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            PresetCategory::FmBroadcast => "📻",
            PresetCategory::AmBroadcast => "📡",
            PresetCategory::Weather => "🌦️",
            PresetCategory::Aircraft => "✈️",
            PresetCategory::HamRadio => "📡",
            PresetCategory::Satellite => "🛰️",
            PresetCategory::Marine => "⚓",
            PresetCategory::PublicService => "🚨",
            PresetCategory::Other => "🔧",
        }
    }

    pub fn all() -> &'static [PresetCategory] {
        &[
            PresetCategory::FmBroadcast,
            PresetCategory::AmBroadcast,
            PresetCategory::Weather,
            PresetCategory::Aircraft,
            PresetCategory::HamRadio,
            PresetCategory::Satellite,
            PresetCategory::Marine,
            PresetCategory::PublicService,
            PresetCategory::Other,
        ]
    }
}

/// Built-in frequency presets database.
pub struct FrequencyDatabase;

impl FrequencyDatabase {
    /// Get all built-in presets.
    pub fn all_presets() -> Vec<FrequencyPreset> {
        let mut presets = Vec::new();

        // Weather radio (NOAA, US)
        presets.extend(Self::weather_presets());

        // Aircraft
        presets.extend(Self::aircraft_presets());

        // Ham radio calling frequencies
        presets.extend(Self::ham_presets());

        // Satellites
        presets.extend(Self::satellite_presets());

        // Marine VHF
        presets.extend(Self::marine_presets());

        // FM broadcast examples (region-specific, user should customize)
        presets.extend(Self::fm_broadcast_examples());

        presets
    }

    fn weather_presets() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "NOAA Weather 1 (162.400 MHz)".to_string(),
                frequency_hz: 162_400_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.400 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 2 (162.425 MHz)".to_string(),
                frequency_hz: 162_425_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.425 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 3 (162.450 MHz)".to_string(),
                frequency_hz: 162_450_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.450 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 4 (162.475 MHz)".to_string(),
                frequency_hz: 162_475_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.475 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 5 (162.500 MHz)".to_string(),
                frequency_hz: 162_500_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.500 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 6 (162.525 MHz)".to_string(),
                frequency_hz: 162_525_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.525 MHz".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "NOAA Weather 7 (162.550 MHz)".to_string(),
                frequency_hz: 162_550_000,
                mode: "FM".to_string(),
                category: PresetCategory::Weather,
                description: "NOAA Weather Radio - 162.550 MHz".to_string(),
                region: Some("US".to_string()),
            },
        ]
    }

    fn aircraft_presets() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "ADS-B (1090 MHz)".to_string(),
                frequency_hz: 1_090_000_000,
                mode: "RAW".to_string(),
                category: PresetCategory::Aircraft,
                description: "Mode-S ADS-B aircraft tracking".to_string(),
                region: Some("Global".to_string()),
            },
            FrequencyPreset {
                name: "Tower (118.1 MHz - varies)".to_string(),
                frequency_hz: 118_100_000,
                mode: "AM".to_string(),
                category: PresetCategory::Aircraft,
                description: "Airport tower frequency (example, check local)".to_string(),
                region: None,
            },
            FrequencyPreset {
                name: "Ground (121.9 MHz - varies)".to_string(),
                frequency_hz: 121_900_000,
                mode: "AM".to_string(),
                category: PresetCategory::Aircraft,
                description: "Ground control frequency (example, check local)".to_string(),
                region: None,
            },
            FrequencyPreset {
                name: "Emergency (121.5 MHz)".to_string(),
                frequency_hz: 121_500_000,
                mode: "AM".to_string(),
                category: PresetCategory::Aircraft,
                description: "International aviation emergency frequency".to_string(),
                region: Some("Global".to_string()),
            },
        ]
    }

    fn ham_presets() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "2m Calling (146.520 MHz)".to_string(),
                frequency_hz: 146_520_000,
                mode: "FM".to_string(),
                category: PresetCategory::HamRadio,
                description: "2m FM simplex calling frequency".to_string(),
                region: Some("US/CA".to_string()),
            },
            FrequencyPreset {
                name: "70cm Calling (446.000 MHz)".to_string(),
                frequency_hz: 446_000_000,
                mode: "FM".to_string(),
                category: PresetCategory::HamRadio,
                description: "70cm FM simplex calling frequency".to_string(),
                region: Some("US".to_string()),
            },
            FrequencyPreset {
                name: "ISS APRS (145.825 MHz)".to_string(),
                frequency_hz: 145_825_000,
                mode: "FM".to_string(),
                category: PresetCategory::HamRadio,
                description: "International Space Station APRS downlink".to_string(),
                region: Some("Global".to_string()),
            },
            FrequencyPreset {
                name: "20m FT8 (14.074 MHz)".to_string(),
                frequency_hz: 14_074_000,
                mode: "USB".to_string(),
                category: PresetCategory::HamRadio,
                description: "20m FT8 digital mode center frequency".to_string(),
                region: Some("Global".to_string()),
            },
        ]
    }

    fn satellite_presets() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "Meteor-M2-4 LRPT (137.100 MHz)".to_string(),
                frequency_hz: 137_100_000,
                mode: "RAW".to_string(),
                category: PresetCategory::Satellite,
                description:
                    "Meteor-M2-4 LRPT downlink; verify current transmitter status before a pass"
                        .to_string(),
                region: Some("Global".to_string()),
            },
            FrequencyPreset {
                name: "Meteor-M2-3 LRPT (137.900 MHz)".to_string(),
                frequency_hz: 137_900_000,
                mode: "RAW".to_string(),
                category: PresetCategory::Satellite,
                description:
                    "Meteor-M2-3 LRPT downlink; verify current transmitter status before a pass"
                        .to_string(),
                region: Some("Global".to_string()),
            },
        ]
    }

    fn marine_presets() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "Marine Ch 16 (156.800 MHz)".to_string(),
                frequency_hz: 156_800_000,
                mode: "FM".to_string(),
                category: PresetCategory::Marine,
                description: "International marine distress/calling channel".to_string(),
                region: Some("Global".to_string()),
            },
            FrequencyPreset {
                name: "Marine Ch 09 (156.450 MHz)".to_string(),
                frequency_hz: 156_450_000,
                mode: "FM".to_string(),
                category: PresetCategory::Marine,
                description: "Alternate calling channel".to_string(),
                region: Some("Global".to_string()),
            },
        ]
    }

    fn fm_broadcast_examples() -> Vec<FrequencyPreset> {
        vec![
            FrequencyPreset {
                name: "FM 88.1 MHz (example)".to_string(),
                frequency_hz: 88_100_000,
                mode: "WFM".to_string(),
                category: PresetCategory::FmBroadcast,
                description: "Local FM station (customize for your area)".to_string(),
                region: None,
            },
            FrequencyPreset {
                name: "FM 98.5 MHz (example)".to_string(),
                frequency_hz: 98_500_000,
                mode: "WFM".to_string(),
                category: PresetCategory::FmBroadcast,
                description: "Local FM station (customize for your area)".to_string(),
                region: None,
            },
            FrequencyPreset {
                name: "FM 104.3 MHz (example)".to_string(),
                frequency_hz: 104_300_000,
                mode: "WFM".to_string(),
                category: PresetCategory::FmBroadcast,
                description: "Local FM station (customize for your area)".to_string(),
                region: None,
            },
        ]
    }

    /// Filter presets by category.
    pub fn by_category(category: PresetCategory) -> Vec<FrequencyPreset> {
        Self::all_presets()
            .into_iter()
            .filter(|p| p.category == category)
            .collect()
    }

    /// Search presets by name or description.
    pub fn search(query: &str) -> Vec<FrequencyPreset> {
        let query_lower = query.to_lowercase();
        Self::all_presets()
            .into_iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query_lower)
                    || p.description.to_lowercase().contains(&query_lower)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_exist() {
        let presets = FrequencyDatabase::all_presets();
        assert!(!presets.is_empty(), "Database should have presets");
        assert!(
            presets.len() > 20,
            "Should have reasonable number of presets"
        );
    }

    #[test]
    fn category_filtering_works() {
        let weather = FrequencyDatabase::by_category(PresetCategory::Weather);
        assert!(!weather.is_empty(), "Should have weather presets");

        for preset in weather {
            assert_eq!(preset.category, PresetCategory::Weather);
        }
    }

    #[test]
    fn search_works() {
        let noaa = FrequencyDatabase::search("NOAA");
        assert!(!noaa.is_empty(), "Should find NOAA presets");

        let aircraft = FrequencyDatabase::search("aircraft");
        assert!(!aircraft.is_empty(), "Should find aircraft presets");
    }

    #[test]
    fn all_categories_have_labels() {
        for category in PresetCategory::all() {
            assert!(!category.label().is_empty());
            assert!(!category.icon().is_empty());
        }
    }

    #[test]
    fn decommissioned_noaa_apt_satellites_are_not_active_presets() {
        let satellites = FrequencyDatabase::by_category(PresetCategory::Satellite);
        assert!(satellites
            .iter()
            .all(|preset| !preset.name.starts_with("NOAA ")));
    }
}
