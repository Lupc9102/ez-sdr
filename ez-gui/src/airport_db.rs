use std::collections::HashMap;

use rusqlite::Connection;

/// Normalized frequency type (OurAirports `type` column is NOT a controlled vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FreqType {
    Emergency,
    Atis,
    Awos,
    Clearance,
    Ground,
    Tower,
    Approach,
    Departure,
    Center,
    Unicom,
    Ctaf,
    Fss,
    Ramp,
    Other,
}

impl FreqType {
    /// Normalize the raw free-text type code to a canonical enum value.
    pub fn from_raw(raw: &str) -> Self {
        let s = raw.trim().to_ascii_uppercase();
        if s.is_empty() {
            return Self::Other;
        }
        // Emergency-style
        if s.contains("EMERG") || s.contains("GUARD") || s.contains("121.5") {
            return Self::Emergency;
        }
        // ATIS / weather
        if s.contains("ATIS") { return Self::Atis; }
        if s.contains("AWOS") || s.contains("ASOS") { return Self::Awos; }
        // Clearance delivery
        if s.contains("CLD") || s.contains("CLNC") || s.contains("CLEAR") || s.contains("DEL") { return Self::Clearance; }
        // Ground
        if s.contains("GND") || s.contains("GROUND") { return Self::Ground; }
        // Tower
        if s.contains("TWR") || s.contains("TOWER") { return Self::Tower; }
        // Approach / Arrival
        if s.contains("APP") || s.contains("ARR") || s.contains("APPROACH") { return Self::Approach; }
        // Departure
        if s.contains("DEP") { return Self::Departure; }
        // Center / Area control
        if s.contains("CNTR") || s.contains("ACC") || s.contains("ARTC") || s.contains("CENTER") || s.contains("CENTRE") { return Self::Center; }
        // CTAF / ATF
        if s.contains("CTAF") || s.contains("ATF") { return Self::Ctaf; }
        // UNICOM
        if s.contains("UNIC") { return Self::Unicom; }
        // Flight service
        if s.contains("FSS") || s.contains("RDO") || s.contains("RCO") { return Self::Fss; }
        // Ramp
        if s.contains("RMP") || s.contains("RAMP") { return Self::Ramp; }
        Self::Other
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Emergency => "EMERG",
            Self::Atis => "ATIS",
            Self::Awos => "AWOS",
            Self::Clearance => "CLNC",
            Self::Ground => "GND",
            Self::Tower => "TWR",
            Self::Approach => "APP",
            Self::Departure => "DEP",
            Self::Center => "CNTR",
            Self::Unicom => "UNICOM",
            Self::Ctaf => "CTAF",
            Self::Fss => "FSS",
            Self::Ramp => "RAMP",
            Self::Other => "OTHER",
        }
    }

    /// Display priority (lower = shown first).
    pub fn priority(self) -> u8 {
        match self {
            Self::Emergency => 0,
            Self::Atis => 1,
            Self::Awos => 2,
            Self::Clearance => 3,
            Self::Ground => 4,
            Self::Tower => 5,
            Self::Approach => 6,
            Self::Departure => 7,
            Self::Center => 8,
            Self::Ctaf => 9,
            Self::Unicom => 10,
            Self::Ramp => 11,
            Self::Fss => 12,
            Self::Other => 13,
        }
    }

    pub fn badge_color(self) -> egui::Color32 {
        match self {
            Self::Emergency => egui::Color32::from_rgb(220, 70, 70),
            Self::Atis | Self::Awos => egui::Color32::from_rgb(120, 200, 255),
            Self::Clearance | Self::Ground => egui::Color32::from_rgb(150, 200, 150),
            Self::Tower => egui::Color32::from_rgb(80, 220, 120),
            Self::Approach | Self::Departure => egui::Color32::from_rgb(255, 200, 80),
            Self::Center => egui::Color32::from_rgb(200, 150, 255),
            _ => egui::Color32::from_gray(170),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Airport {
    pub ident: String,
    pub icao: String,
    pub iata: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub country: String,
    pub atype: String,
    pub scheduled: bool,
}

#[derive(Debug, Clone)]
pub struct AirportFreq {
    pub airport_ident: String,
    pub freq_type: FreqType,
    pub raw_type: String,
    pub description: String,
    pub frequency_mhz: f64,
}

/// Antenna dimensions for a given frequency (pure math).
#[derive(Debug, Clone)]
pub struct AntennaDims {
    pub freq_mhz: f64,
    pub quarter_wave_cm: f64,
    pub half_wave_dipole_cm: f64,
    pub ground_plane_radial_cm: f64,
    pub coax_collinear_segment_cm: f64,
    pub suggested_antenna: &'static str,
    pub suggested_mode: &'static str,
    pub suggested_bw_hz: u32,
}

pub fn antenna_dims(freq_mhz: f64) -> AntennaDims {
    let quarter = 7500.0 / freq_mhz;
    let half = 15000.0 / freq_mhz;
    AntennaDims {
        freq_mhz,
        quarter_wave_cm: quarter,
        half_wave_dipole_cm: half,
        ground_plane_radial_cm: quarter,
        // Coax collinear half-wave segment, RG-58 velocity factor 0.66
        coax_collinear_segment_cm: half * 0.66,
        suggested_antenna: suggested_antenna(freq_mhz),
        suggested_mode: suggested_mode(freq_mhz),
        suggested_bw_hz: suggested_bw(freq_mhz),
    }
}

fn suggested_antenna(freq_mhz: f64) -> &'static str {
    if freq_mhz < 30.0 {
        "Long-wire / magnetic loop (HF)"
    } else if freq_mhz < 110.0 {
        "Half-wave dipole or discone (VHF low)"
    } else if freq_mhz < 137.0 {
        "Half-wave dipole / discone (airband)"
    } else if freq_mhz < 150.0 {
        "V-dipole 53.4 cm arms @ 120 deg (NOAA 137 MHz)"
    } else if freq_mhz < 200.0 {
        "Quarter-wave vertical + ground plane"
    } else if freq_mhz < 500.0 {
        "Discone or quarter-wave vertical"
    } else if freq_mhz < 1000.0 {
        "Discone or log-periodic (UHF)"
    } else if (1080.0..=1100.0).contains(&freq_mhz) {
        "Quarter-wave ground-plane (6.9 cm) or coaxial collinear"
    } else if (1680.0..=1710.0).contains(&freq_mhz) {
        "Helical (7-12 turns RHCP) or grid dish (GOES)"
    } else {
        "Quarter-wave vertical + ground plane"
    }
}

fn suggested_mode(freq_mhz: f64) -> &'static str {
    if freq_mhz < 30.0 {
        "AM"
    } else if (1080.0..=1100.0).contains(&freq_mhz) {
        "RAW"
    } else if freq_mhz > 1500.0 {
        "RAW"
    } else if (137.0..=138.0).contains(&freq_mhz) {
        "WFM"
    } else if freq_mhz < 200.0 && freq_mhz > 87.0 && (freq_mhz < 108.0) {
        "WFM"
    } else {
        "AM"
    }
}

fn suggested_bw(freq_mhz: f64) -> u32 {
    if (118.0..=137.0).contains(&freq_mhz) {
        8_000 // airband voice
    } else if (137.0..=138.0).contains(&freq_mhz) {
        38_000 // NOAA APT
    } else if (1080.0..=1100.0).contains(&freq_mhz) {
        2_000_000 // ADS-B
    } else if (1680.0..=1710.0).contains(&freq_mhz) {
        600_000 // GOES
    } else if (88.0..=108.0).contains(&freq_mhz) {
        200_000 // FM broadcast
    } else {
        12_500
    }
}

pub struct AirportDb {
    pub airports: HashMap<String, Airport>,
    pub freqs: HashMap<String, Vec<AirportFreq>>,
    pub cached_sqlite: bool,
}

impl Default for AirportDb {
    fn default() -> Self {
        Self { airports: HashMap::new(), freqs: HashMap::new(), cached_sqlite: false }
    }
}

impl AirportDb {
    /// Load from the SQLite cache if present; otherwise hydrate from the
    /// hardcoded fallback list (always works offline).
    pub fn load() -> Self {
        if let Ok(conn) = Connection::open("ez_sdr.db") {
            if let Ok(count) = conn.query_row("SELECT COUNT(*) FROM airports", [], |r| r.get::<_, i64>(0)) {
                if count > 0 {
                    return Self::load_from_sqlite(&conn);
                }
            }
        }
        Self::load_fallback()
    }

    fn load_from_sqlite(conn: &Connection) -> Self {
        let mut db = Self::default();
        db.cached_sqlite = true;
        if let Ok(mut stmt) = conn.prepare(
            "SELECT ident, icao, iata, name, lat, lon, country, type, scheduled FROM airports"
        ) {
            let rows = stmt.query_map([], |r| Ok(Airport {
                ident: r.get(0)?,
                icao: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                iata: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                name: r.get(3)?,
                lat: r.get(4)?,
                lon: r.get(5)?,
                country: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                atype: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                scheduled: r.get::<_, Option<i64>>(8)?.map(|v| v != 0).unwrap_or(false),
            }));
            if let Ok(rows) = rows {
                for a in rows.flatten() {
                    db.airports.insert(a.ident.clone(), a);
                }
            }
        }
        if let Ok(mut stmt) = conn.prepare(
            "SELECT airport_ident, type, description, frequency_mhz FROM airport_freqs"
        ) {
            let rows = stmt.query_map([], |r| Ok(AirportFreq {
                airport_ident: r.get(0)?,
                raw_type: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                description: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                frequency_mhz: r.get(3)?,
                freq_type: FreqType::Other,
            }));
            if let Ok(rows) = rows {
                for mut f in rows.flatten() {
                    f.freq_type = FreqType::from_raw(&f.raw_type);
                    db.freqs.entry(f.airport_ident.clone()).or_default().push(f);
                }
            }
        }
        for v in db.freqs.values_mut() {
            v.sort_by_key(|f| f.freq_type.priority());
        }
        db
    }

    fn load_fallback() -> Self {
        let mut db = Self::default();
        for &(ident, icao, iata, name, lat, lon, country, atype, freqs) in FALLBACK_AIRPORTS {
            let a = Airport {
                ident: ident.to_string(), icao: icao.to_string(), iata: iata.to_string(),
                name: name.to_string(), lat, lon, country: country.to_string(),
                atype: atype.to_string(), scheduled: true,
            };
            let fv: Vec<AirportFreq> = freqs.iter().map(|&(t, d, mhz)| AirportFreq {
                airport_ident: ident.to_string(),
                raw_type: t.to_string(),
                description: d.to_string(),
                frequency_mhz: mhz,
                freq_type: FreqType::from_raw(t),
            }).collect();
            db.airports.insert(a.ident.to_string(), a);
            db.freqs.insert(ident.to_string(), fv);
        }
        db
    }

    /// Search across ident/icao/iata/name; ranked (exact > prefix > substring).
    pub fn search(&self, query: &str, type_filter: &str, limit: usize) -> Vec<&Airport> {
        let q = query.trim().to_ascii_uppercase();
        let mut hits: Vec<(u8, &Airport)> = Vec::new();
        for a in self.airports.values() {
            if type_filter != "all" && a.atype != type_filter {
                continue;
            }
            let ident = a.ident.to_ascii_uppercase();
            let icao = a.icao.to_ascii_uppercase();
            let iata = a.iata.to_ascii_uppercase();
            let name = a.name.to_ascii_uppercase();
            if q.is_empty() {
                if a.scheduled {
                    hits.push((5, a));
                }
            } else if ident == q || icao == q || iata == q {
                hits.push((0, a));
            } else if ident.starts_with(&q) || icao.starts_with(&q) || iata.starts_with(&q) {
                hits.push((1, a));
            } else if name.starts_with(&q) {
                hits.push((2, a));
            } else if ident.contains(&q) || icao.contains(&q) || iata.contains(&q) {
                hits.push((3, a));
            } else if name.contains(&q) {
                hits.push((4, a));
            }
        }
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        hits.into_iter().take(limit).map(|(_, a)| a).collect()
    }

    pub fn frequencies_for(&self, ident: &str) -> &[AirportFreq] {
        self.freqs.get(ident).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn airport(&self, ident: &str) -> Option<&Airport> {
        self.airports.get(ident)
    }

    /// Fetch both OurAirports CSVs and cache them in SQLite. Returns the number
    /// of airports loaded. `progress(current, total)` reports download bytes.
    pub fn download_full_blocking(mut progress: impl FnMut(usize, usize)) -> Result<usize, String> {
        let airports_csv = Self::fetch_csv("https://davidmegginson.github.io/ourairports-data/airports.csv", &mut progress)?;
        let freq_csv = Self::fetch_csv("https://davidmegginson.github.io/ourairports-data/airport-frequencies.csv", &mut progress)?;

        let mut airports = Vec::new();
        for (i, row) in airports_csv.iter().enumerate() {
            if i == 0 { continue; } // header
            let cols = csv_split(row);
            if cols.len() < 13 { continue; }
            let atype = cols.get(2).cloned().unwrap_or_default();
            if atype == "closed_airport" || atype == "balloonport" { continue; }
            airports.push(Airport {
                ident: cols.get(1).cloned().unwrap_or_default(),
                icao: cols.get(12).cloned().unwrap_or_default(),
                iata: cols.get(13).cloned().unwrap_or_default(),
                name: cols.get(3).cloned().unwrap_or_default(),
                lat: cols.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                lon: cols.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                country: cols.get(8).cloned().unwrap_or_default(),
                atype,
                scheduled: cols.get(11).map(|s| s == "yes").unwrap_or(false),
            });
        }

        let mut freqs: Vec<AirportFreq> = Vec::new();
        for (i, row) in freq_csv.iter().enumerate() {
            if i == 0 { continue; }
            let cols = csv_split(row);
            if cols.len() < 6 { continue; }
            let raw_type = cols.get(3).cloned().unwrap_or_default();
            let f = AirportFreq {
                airport_ident: cols.get(2).cloned().unwrap_or_default(),
                raw_type: raw_type.clone(),
                description: cols.get(4).cloned().unwrap_or_default(),
                frequency_mhz: cols.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                freq_type: FreqType::from_raw(&raw_type),
            };
            if f.frequency_mhz > 0.0 {
                freqs.push(f);
            }
        }

        let count = airports.len();
        Self::store_sqlite(&airports, &freqs)?;
        Ok(count)
    }

    fn fetch_csv(url: &str, progress: &mut impl FnMut(usize, usize)) -> Result<Vec<String>, String> {
        let resp = reqwest::blocking::get(url).map_err(|e| format!("{}: {}", url, e))?;
        if !resp.status().is_success() {
            return Err(format!("{}: HTTP {}", url, resp.status()));
        }
        let total = resp.content_length().unwrap_or(0) as usize;
        let bytes = resp.bytes().map_err(|e| format!("{}: {}", url, e))?;
        progress(bytes.len(), total);
        let text = String::from_utf8_lossy(&bytes);
        Ok(text.lines().map(|l| l.to_string()).collect())
    }

    fn store_sqlite(airports: &[Airport], freqs: &[AirportFreq]) -> Result<(), String> {
        let mut conn = Connection::open("ez_sdr.db").map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        conn.execute_batch("DELETE FROM airport_freqs; DELETE FROM airports;").map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO airports (ident,icao,iata,name,lat,lon,country,type,scheduled) VALUES (?,?,?,?,?,?,?,?,?)"
            ).map_err(|e| e.to_string())?;
            for a in airports {
                stmt.execute(rusqlite::params![a.ident, a.icao, a.iata, a.name, a.lat, a.lon, a.country, a.atype, a.scheduled as i32])
                    .ok();
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO airport_freqs (airport_ident,type,description,frequency_mhz) VALUES (?,?,?,?)"
            ).map_err(|e| e.to_string())?;
            for f in freqs {
                stmt.execute(rusqlite::params![f.airport_ident, f.raw_type, f.description, f.frequency_mhz]).ok();
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Minimal RFC-4180-ish CSV row splitter (handles quoted fields with commas).
fn csv_split(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// ~30 major world hubs with verified frequencies. Always available offline.
/// Tuple: (ident, icao, iata, name, lat, lon, country, type, &[(freq_type, desc, mhz)])
static FALLBACK_AIRPORTS: &[(&str, &str, &str, &str, f64, f64, &str, &str, &[(&str, &str, f64)])] = &[
    ("KLAX", "KLAX", "LAX", "Los Angeles Intl", 33.9425, -118.408, "US", "large_airport",
     &[("ATIS", "LAX ATIS", 134.45), ("CLD", "Clearance", 127.65), ("GND", "Ground", 121.65), ("TWR", "Tower", 120.95), ("APP", "SoCal Approach", 124.5), ("DEP", "Departure", 124.3)]),
    ("KJFK", "KJFK", "JFK", "John F Kennedy Intl", 40.6413, -73.7781, "US", "large_airport",
     &[("ATIS", "JFK ATIS", 135.9), ("CLD", "Clearance", 127.4), ("GND", "Ground", 121.9), ("TWR", "Tower", 127.4), ("APP", "Approach", 125.7), ("DEP", "Departure", 125.85)]),
    ("EGLL", "EGLL", "LHR", "Heathrow", 51.4700, -0.4543, "GB", "large_airport",
     &[("ATIS", "Heathrow ATIS", 113.75), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.5), ("APP", "Approach", 119.72), ("DEP", "Departure", 135.8)]),
    ("LFPG", "LFPG", "CDG", "Paris Charles de Gaulle", 49.0097, 2.5479, "FR", "large_airport",
     &[("ATIS", "CDG ATIS", 127.22), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.1), ("APP", "Approach", 120.4), ("DEP", "Departure", 125.65)]),
    ("EDDF", "EDDF", "FRA", "Frankfurt am Main", 50.0379, 8.5622, "DE", "large_airport",
     &[("ATIS", "FRA ATIS", 136.25), ("GND", "Ground", 121.65), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.2), ("DEP", "Departure", 120.75)]),
    ("EHAM", "EHAM", "AMS", "Amsterdam Schiphol", 52.3105, 4.7683, "NL", "large_airport",
     &[("ATIS", "Schiphol ATIS", 136.55), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.4), ("APP", "Approach", 119.05), ("DEP", "Departure", 125.55)]),
    ("LEMD", "LEMD", "MAD", "Madrid Barajas", 40.4719, -3.5626, "ES", "large_airport",
     &[("ATIS", "ATIS", 127.45), ("GND", "Ground", 121.65), ("TWR", "Tower", 118.3), ("APP", "Approach", 119.4), ("DEP", "Departure", 125.65)]),
    ("LIRF", "LIRF", "FCO", "Rome Fiumicino", 41.8003, 12.2389, "IT", "large_airport",
     &[("ATIS", "ATIS", 127.7), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.7), ("DEP", "Departure", 120.9)]),
    ("LSZH", "LSZH", "ZRH", "Zurich", 47.4647, 8.5492, "CH", "large_airport",
     &[("ATIS", "ATIS", 128.025), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.05), ("APP", "Approach", 119.0), ("DEP", "Departure", 128.05)]),
    ("EDDM", "EDDM", "MUC", "Munich", 48.3538, 11.7861, "DE", "large_airport",
     &[("ATIS", "ATIS", 136.45), ("GND", "Ground", 121.8), ("TWR", "Tower", 118.6), ("APP", "Approach", 119.2), ("DEP", "Departure", 120.75)]),
    ("EKCH", "EKCH", "CPH", "Copenhagen Kastrup", 55.6181, 12.6561, "DK", "large_airport",
     &[("ATIS", "ATIS", 126.3), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.3), ("APP", "Approach", 119.6), ("DEP", "Departure", 120.45)]),
    ("ESSA", "ESSA", "ARN", "Stockholm Arlanda", 59.6519, 17.9186, "SE", "large_airport",
     &[("ATIS", "ATIS", 127.025), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.3), ("APP", "Approach", 119.1), ("DEP", "Departure", 120.15)]),
    ("ENGM", "ENGM", "OSL", "Oslo Gardermoen", 60.1939, 11.1004, "NO", "large_airport",
     &[("ATIS", "ATIS", 127.075), ("GND", "Ground", 121.7), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.4), ("DEP", "Departure", 120.2)]),
    ("EFHK", "EFHK", "HEL", "Helsinki Vantaa", 60.3172, 24.9633, "FI", "large_airport",
     &[("ATIS", "ATIS", 128.65), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.7), ("APP", "Approach", 119.55), ("DEP", "Departure", 120.6)]),
    ("LOWW", "LOWW", "VIE", "Vienna Schwechat", 48.1103, 16.5697, "AT", "large_airport",
     &[("ATIS", "ATIS", 136.975), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.2), ("DEP", "Departure", 125.05)]),
    ("EBBR", "EBBR", "BRU", "Brussels Zaventem", 50.9014, 4.4844, "BE", "large_airport",
     &[("ATIS", "ATIS", 126.825), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.7), ("APP", "Approach", 119.2), ("DEP", "Departure", 125.6)]),
    ("RJTT", "RJTT", "HND", "Tokyo Haneda", 35.5494, 139.7798, "JP", "large_airport",
     &[("ATIS", "ATIS", 126.65), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.1), ("APP", "Approach", 120.8), ("DEP", "Departure", 126.0)]),
    ("RKSI", "RKSI", "ICN", "Seoul Incheon", 37.4602, 126.4407, "KR", "large_airport",
     &[("ATIS", "ATIS", 128.65), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.25), ("DEP", "Departure", 125.55)]),
    ("ZBAA", "ZBAA", "PEK", "Beijing Capital", 40.0801, 116.5846, "CN", "large_airport",
     &[("ATIS", "ATIS", 127.6), ("GND", "Ground", 121.85), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.0), ("DEP", "Departure", 125.85)]),
    ("VHHH", "VHHH", "HKG", "Hong Kong", 22.3089, 113.9144, "HK", "large_airport",
     &[("ATIS", "ATIS", 128.2), ("GND", "Ground", 121.6), ("TWR", "Tower", 118.4), ("APP", "Approach", 119.1), ("DEP", "Departure", 123.9)]),
    ("WSSS", "WSSS", "SIN", "Singapore Changi", 1.3644, 103.9915, "SG", "large_airport",
     &[("ATIS", "ATIS", 128.6), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.6), ("APP", "Approach", 126.55), ("DEP", "Departure", 123.6)]),
    ("OMDB", "OMDB", "DXB", "Dubai Intl", 25.2532, 55.3657, "AE", "large_airport",
     &[("ATIS", "ATIS", 127.4), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.4), ("APP", "Approach", 119.4), ("DEP", "Departure", 125.55)]),
    ("LTFM", "LTFM", "IST", "Istanbul", 41.2753, 28.7519, "TR", "large_airport",
     &[("ATIS", "ATIS", 127.5), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.6), ("APP", "Approach", 119.3), ("DEP", "Departure", 125.7)]),
    ("YSSY", "YSSY", "SYD", "Sydney Kingsford Smith", -33.9399, 151.1753, "AU", "large_airport",
     &[("ATIS", "ATIS", 127.0), ("GND", "Ground", 121.7), ("TWR", "Tower", 120.5), ("APP", "Approach", 124.7), ("DEP", "Departure", 123.0)]),
    ("SBGR", "SBGR", "GRU", "Sao Paulo Guarulhos", -23.4356, -46.4731, "BR", "large_airport",
     &[("ATIS", "ATIS", 127.65), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.0), ("APP", "Approach", 119.2), ("DEP", "Departure", 125.3)]),
    ("SAEZ", "SAEZ", "EZE", "Buenos Aires Ezeiza", -34.8222, -58.5358, "AR", "large_airport",
     &[("ATIS", "ATIS", 127.0), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.1), ("DEP", "Departure", 125.5)]),
    ("FAOR", "FAOR", "JNB", "Johannesburg OR Tambo", -26.1392, 28.2460, "ZA", "large_airport",
     &[("ATIS", "ATIS", 127.0), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.1), ("APP", "Approach", 119.2), ("DEP", "Departure", 125.6)]),
    ("VABB", "VABB", "BOM", "Mumbai Chhatrapati Shivaji", 19.0896, 72.8656, "IN", "large_airport",
     &[("ATIS", "ATIS", 126.6), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.5), ("APP", "Approach", 119.1), ("DEP", "Departure", 125.55)]),
    ("VIDP", "VIDP", "DEL", "Delhi Indira Gandhi", 28.5562, 77.1000, "IN", "large_airport",
     &[("ATIS", "ATIS", 126.6), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.5), ("APP", "Approach", 119.1), ("DEP", "Departure", 125.55)]),
    ("MMMX", "MMMX", "MEX", "Mexico City Intl", 19.4361, -99.0719, "MX", "large_airport",
     &[("ATIS", "ATIS", 127.2), ("GND", "Ground", 121.9), ("TWR", "Tower", 118.9), ("APP", "Approach", 119.2), ("DEP", "Departure", 125.5)]),
];

impl Airport {
    /// Format code badge: prefer ICAO, fall back to ident.
    pub fn code(&self) -> &str {
        if !self.icao.is_empty() { &self.icao } else { &self.ident }
    }
}
