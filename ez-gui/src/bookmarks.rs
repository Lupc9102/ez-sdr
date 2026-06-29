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
    /// Import bookmarks from a CSV file (columns: name,frequency_hz,mode,category,notes or bandwidth_hz).
    /// Skips rows with frequency already in the list. Header row auto-detected.
    /// Returns (imported count, error message)
    pub fn import_csv(&mut self, path: &str) -> (usize, String) {
        let content = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return (0, format!("Read error: {}", e)),
        };
        let existing_freqs: std::collections::HashSet<u64> = self.bookmarks.iter().map(|b| b.frequency_hz).collect();
        let mut count = 0;
        let mut header: Option<Vec<String>> = None;
        for line in content.lines() {
            let trim = line.trim();
            if trim.is_empty() { continue; }
            let parts: Vec<String> = trim.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect();
            if parts.is_empty() { continue; }

            // Detect header row
            if header.is_none() {
                if parts[0].eq_ignore_ascii_case("name") || parts[0].eq_ignore_ascii_case("frequency_hz") {
                    header = Some(parts.iter().map(|s| s.to_lowercase()).collect());
                    continue;
                }
                // No header — use positional: name,frequency_hz,mode,category,notes
                header = Some(vec!["name".into(), "frequency_hz".into(), "mode".into(), "category".into(), "notes".into()]);
            }

            let col = |name: &str| -> Option<&str> {
                header.as_ref()?.iter().position(|h| h == name)
                    .and_then(|i| parts.get(i))
                    .map(|s| s.as_str())
                    .filter(|s| !s.is_empty())
            };

            let freq_str = col("frequency_hz").unwrap_or("");
            let freq_hz: u64 = if let Ok(v) = freq_str.parse::<u64>() {
                v
            } else if let Ok(v) = freq_str.parse::<f64>() {
                if v < 10_000.0 { (v * 1_000_000.0) as u64 } else { v as u64 }
            } else {
                continue;
            };

            if existing_freqs.contains(&freq_hz) { continue; }

            let name = col("name").unwrap_or("Imported").to_string();
            let mode = col("mode").unwrap_or("NFM").to_string();
            let category = col("category").unwrap_or("Imported").to_string();
            let notes = col("notes").unwrap_or("").to_string();
            let bandwidth_hz: u32 = col("bandwidth_hz")
                .and_then(|s| s.parse().ok())
                .unwrap_or(12_500);

            self.bookmarks.push(Bookmark {
                name,
                frequency_hz: freq_hz,
                mode,
                bandwidth_hz,
                category,
                notes,
            });
            count += 1;
        }
        (count, String::new())
    }

    /// Export bookmarks to CSV. Returns (path, error)
    pub fn export_csv(&self) -> (String, String) {
        let default_name = format!("ez_sdr_bookmarks_{}.csv",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0));
        let path = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("CSV", &["csv"])
            .save_file();
        let path = match path {
            Some(p) => p,
            None => return (String::new(), String::new()),
        };
        let mut csv = String::from("name,frequency_hz,frequency_mhz,mode,category,notes\n");
        for bm in &self.bookmarks {
            csv.push_str(&format!("{},{},{:.6},{},{},{}\n",
                bm.name.replace(',', ";"),
                bm.frequency_hz,
                bm.frequency_hz as f64 / 1e6,
                bm.mode,
                bm.category.replace(',', ";"),
                bm.notes.replace(',', ";")));
        }
        let path_str = path.to_string_lossy().to_string();
        match std::fs::write(&path, &csv) {
            Ok(_) => (path_str, String::new()),
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
