use crate::source_manager::SourceManager;

pub struct BookmarkDb {
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Debug, Clone)]
pub struct Bookmark {
    pub name: String,
    pub frequency_hz: u64,
    pub mode: String,
    pub bandwidth_hz: u32,
    pub category: String,
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
                Bookmark { name: " ham 2m".into(), frequency_hz: 145_500_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Ham".into(), notes: "144-146 MHz".into() },
                Bookmark { name: "ham 70cm".into(), frequency_hz: 435_000_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Ham".into(), notes: "430-440 MHz".into() },
                Bookmark { name: "ISS Voice".into(), frequency_hz: 145_800_000, mode: "NFM".into(), bandwidth_hz: 12_500, category: "Space".into(), notes: "".into() },
                Bookmark { name: "ISS SSTV".into(), frequency_hz: 145_800_000, mode: "WFM".into(), bandwidth_hz: 34_000, category: "Space".into(), notes: "".into() },
                Bookmark { name: "Inmarsat Aero".into(), frequency_hz: 1_541_500_000, mode: "WFM".into(), bandwidth_hz: 600_000, category: "Satellite".into(), notes: "".into() },
                Bookmark { name: "Iridium".into(), frequency_hz: 1_626_000_000, mode: "WFM".into(), bandwidth_hz: 41_000, category: "Satellite".into(), notes: "".into() },
                Bookmark { name: "Starlink".into(), frequency_hz: 1_600_000_000, mode: "WFM".into(), bandwidth_hz: 50_000_000, category: "Satellite".into(), notes: "Downlink ~11.7-12.7 GHz".into() },
                Bookmark { name: "L-band GPS L1".into(), frequency_hz: 1_575_420_000, mode: "RAW".into(), bandwidth_hz: 2_000_000, category: "Navigation".into(), notes: "".into() },
                Bookmark { name: "Galileo E1".into(), frequency_hz: 1_575_420_000, mode: "RAW".into(), bandwidth_hz: 2_000_000, category: "Navigation".into(), notes: "".into() },
                Bookmark { name: "GSM 900 UL".into(), frequency_hz: 890_000_000, mode: "RAW".into(), bandwidth_hz: 200_000, category: "Cellular".into(), notes: "Uplink".into() },
                Bookmark { name: "GSM 900 DL".into(), frequency_hz: 935_000_000, mode: "RAW".into(), bandwidth_hz: 200_000, category: "Cellular".into(), notes: "Downlink".into() },
                Bookmark { name: "LoRa 868".into(), frequency_hz: 868_100_000, mode: "RAW".into(), bandwidth_hz: 125_000, category: "IoT".into(), notes: "EU ISM band".into() },
                Bookmark { name: "ISM 433".into(), frequency_hz: 433_920_000, mode: "RAW".into(), bandwidth_hz: 300_000, category: "IoT".into(), notes: "".into() },
            ],
        }
    }
}

impl BookmarkDb {
    pub fn ui(&mut self, ui: &mut egui::Ui, source: &mut SourceManager) {
        ui.heading("Frequency Bookmarks");
        let categories: Vec<_> = self.bookmarks.iter().map(|b| b.category.clone()).collect::<std::collections::HashSet<_>>().into_iter().collect();
        for cat in categories {
            ui.collapsing(&cat, |ui| {
                for bm in self.bookmarks.iter().filter(|b| b.category == cat) {
                    ui.horizontal(|ui| {
                        ui.label(&bm.name);
                        ui.monospace(format!("{:.3} MHz", bm.frequency_hz as f64 / 1e6));
                        if ui.button("Tune").clicked() {
                            source.frequency_hz = bm.frequency_hz;
                        }
                    });
                }
            });
        }
    }
}
