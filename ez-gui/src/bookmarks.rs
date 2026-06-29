use serde::{Deserialize, Serialize};

pub struct BookmarkDb {
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub frequency_hz: u64,
    pub mode: String,
    #[allow(dead_code)]
    pub bandwidth_hz: u32,
    pub category: String,
    #[allow(dead_code)]
    pub notes: String,
}

impl Default for BookmarkDb {
    fn default() -> Self {
        Self {
            bookmarks: vec![
                Bookmark { name: "NOAA 15 APT".into(), frequency_hz: 137_620_000, mode: "WFM".into(), bandwidth_hz: 34_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "NOAA 18 APT".into(), frequency_hz: 137_912_500, mode: "WFM".into(), bandwidth_hz: 34_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "NOAA 19 APT".into(), frequency_hz: 137_100_000, mode: "WFM".into(), bandwidth_hz: 34_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "Meteor-M2 LRPT".into(), frequency_hz: 137_900_000, mode: "WFM".into(), bandwidth_hz: 140_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "Meteor-M2-2 LRPT".into(), frequency_hz: 137_100_000, mode: "WFM".into(), bandwidth_hz: 140_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "GOES-16 HRIT".into(), frequency_hz: 1_694_100_000, mode: "WFM".into(), bandwidth_hz: 600_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "GOES-17 HRIT".into(), frequency_hz: 1_694_100_000, mode: "WFM".into(), bandwidth_hz: 600_000, category: "Weather".into(), notes: "".into() },
                Bookmark { name: "ADS-B 1090".into(), frequency_hz: 1_090_000_000, mode: "RAW".into(), bandwidth_hz: 2_000_000, category: "Aviation".into(), notes: "".into() },
                Bookmark { name: "Airband VHF".into(), frequency_hz: 118_000_000, mode: "AM".into(), bandwidth_hz: 8_000, category: "Aviation".into(), notes: "118-136 MHz".into() },
                Bookmark { name: "Marine VHF Ch16".into(), frequency_hz: 156_800_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Marine".into(), notes: "".into() },
                Bookmark { name: "Pager 2m".into(), frequency_hz: 153_000_000, mode: "RAW".into(), bandwidth_hz: 25_000, category: "Pager".into(), notes: "".into() },
                Bookmark { name: "FM Radio".into(), frequency_hz: 100_000_000, mode: "WFM".into(), bandwidth_hz: 200_000, category: "Broadcast".into(), notes: "87.5-108 MHz".into() },
                Bookmark { name: "DAB Band III".into(), frequency_hz: 220_000_000, mode: "WFM".into(), bandwidth_hz: 1_500_000, category: "Broadcast".into(), notes: "".into() },
                Bookmark { name: "Ham 2m".into(), frequency_hz: 145_500_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Ham".into(), notes: "144-146 MHz".into() },
                Bookmark { name: "Ham 70cm".into(), frequency_hz: 435_000_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Ham".into(), notes: "430-440 MHz".into() },
                Bookmark { name: "ISS Voice".into(), frequency_hz: 145_800_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Space".into(), notes: "".into() },
                Bookmark { name: "ISS SSTV".into(), frequency_hz: 145_800_000, mode: "WFM".into(), bandwidth_hz: 34_000, category: "Space".into(), notes: "".into() },
                Bookmark { name: "Inmarsat Aero".into(), frequency_hz: 1_541_500_000, mode: "WFM".into(), bandwidth_hz: 600_000, category: "Satellite".into(), notes: "".into() },
                Bookmark { name: "Iridium".into(), frequency_hz: 1_626_000_000, mode: "WFM".into(), bandwidth_hz: 41_000, category: "Satellite".into(), notes: "".into() },
                Bookmark { name: "GPS L1".into(), frequency_hz: 1_575_420_000, mode: "RAW".into(), bandwidth_hz: 2_000_000, category: "Navigation".into(), notes: "".into() },
                Bookmark { name: "Galileo E1".into(), frequency_hz: 1_575_420_000, mode: "RAW".into(), bandwidth_hz: 2_000_000, category: "Navigation".into(), notes: "".into() },
                Bookmark { name: "GSM 900 UL".into(), frequency_hz: 890_000_000, mode: "RAW".into(), bandwidth_hz: 200_000, category: "Cellular".into(), notes: "Uplink".into() },
                Bookmark { name: "GSM 900 DL".into(), frequency_hz: 935_000_000, mode: "RAW".into(), bandwidth_hz: 200_000, category: "Cellular".into(), notes: "Downlink".into() },
                Bookmark { name: "LoRa 868".into(), frequency_hz: 868_100_000, mode: "RAW".into(), bandwidth_hz: 125_000, category: "IoT".into(), notes: "EU ISM band".into() },
                Bookmark { name: "ISM 433".into(), frequency_hz: 433_920_000, mode: "RAW".into(), bandwidth_hz: 300_000, category: "IoT".into(), notes: "".into() },
            ],
        }
    }
}

const BOOKMARKS_FILE: &str = "ez_sdr_bookmarks.json";

impl BookmarkDb {
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.bookmarks) {
            let _ = std::fs::write(BOOKMARKS_FILE, json);
        }
    }

    pub fn load_saved() -> Option<Vec<Bookmark>> {
        let s = std::fs::read_to_string(BOOKMARKS_FILE).ok()?;
        serde_json::from_str(&s).ok()
    }

    pub fn load_or_default() -> Self {
        let bookmarks = Self::load_saved().unwrap_or_else(|| Self::default().bookmarks);
        Self { bookmarks }
    }
}

impl BookmarkDb {
    /// Import bookmarks from a CSV file.
    /// Expected columns: name,frequency_hz,mode,category (header row optional)
    /// Returns (imported count, error message)
    pub fn import_csv(&mut self, path: &str) -> (usize, String) {
        let content = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return (0, format!("Read error: {}", e)),
        };
        let mut count = 0;
        for (i, line) in content.lines().enumerate() {
            let parts: Vec<&str> = line.splitn(4, ',').collect();
            if parts.len() < 2 { continue; }
            // Skip header row
            if i == 0 && parts[0].eq_ignore_ascii_case("name") { continue; }
            let name = parts[0].trim().trim_matches('"').to_string();
            let freq_str = parts[1].trim().trim_matches('"');
            let freq_hz: u64 = match freq_str.parse() {
                Ok(v) => v,
                Err(_) => {
                    // Try MHz
                    match freq_str.parse::<f64>() {
                        Ok(v) if v < 10_000.0 => (v * 1_000_000.0) as u64,
                        _ => continue,
                    }
                }
            };
            let mode = parts.get(2).map(|s| s.trim().trim_matches('"')).unwrap_or("NFM").to_string();
            let category = parts.get(3).map(|s| s.trim().trim_matches('"')).unwrap_or("Imported").to_string();
            self.bookmarks.push(Bookmark {
                name,
                frequency_hz: freq_hz,
                mode,
                bandwidth_hz: 12_500,
                category,
                notes: String::new(),
            });
            count += 1;
        }
        (count, String::new())
    }

    /// Export bookmarks to CSV. Returns (path, error)
    pub fn export_csv(&self) -> (String, String) {
        let filename = format!("ez_sdr_bookmarks_{}.csv",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0));
        let mut csv = String::from("name,frequency_hz,mode,category,notes\n");
        for bm in &self.bookmarks {
            csv.push_str(&format!("{},{},{},{},{}\n",
                bm.name.replace(',', ";"),
                bm.frequency_hz,
                bm.mode,
                bm.category.replace(',', ";"),
                bm.notes.replace(',', ";")));
        }
        match std::fs::write(&filename, &csv) {
            Ok(_) => (filename, String::new()),
            Err(e) => (String::new(), format!("Export failed: {}", e)),
        }
    }
}

impl Bookmark {
    pub fn freq_display(&self) -> String {
        if self.frequency_hz >= 1_000_000_000 {
            format!("{:.3} GHz", self.frequency_hz as f64 / 1e9)
        } else if self.frequency_hz >= 1_000_000 {
            format!("{:.3} MHz", self.frequency_hz as f64 / 1e6)
        } else {
            format!("{:.1} kHz", self.frequency_hz as f64 / 1e3)
        }
    }
}
