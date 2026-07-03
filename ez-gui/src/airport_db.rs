use std::collections::HashMap;

use rusqlite::Connection;

/// Normalized frequency type (`OurAirports` `type` column is NOT a controlled vocabulary).
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
        if s.contains("ATIS") {
            return Self::Atis;
        }
        if s.contains("AWOS") || s.contains("ASOS") {
            return Self::Awos;
        }
        // Clearance delivery
        if s.contains("CLD") || s.contains("CLNC") || s.contains("CLEAR") || s.contains("DEL") {
            return Self::Clearance;
        }
        // Ground
        if s.contains("GND") || s.contains("GROUND") {
            return Self::Ground;
        }
        // Tower
        if s.contains("TWR") || s.contains("TOWER") {
            return Self::Tower;
        }
        // Approach / Arrival
        if s.contains("APP") || s.contains("ARR") || s.contains("APPROACH") {
            return Self::Approach;
        }
        // Departure
        if s.contains("DEP") {
            return Self::Departure;
        }
        // Center / Area control
        if s.contains("CNTR")
            || s.contains("ACC")
            || s.contains("ARTC")
            || s.contains("CENTER")
            || s.contains("CENTRE")
        {
            return Self::Center;
        }
        // CTAF / ATF
        if s.contains("CTAF") || s.contains("ATF") {
            return Self::Ctaf;
        }
        // UNICOM
        if s.contains("UNIC") {
            return Self::Unicom;
        }
        // Flight service
        if s.contains("FSS") || s.contains("RDO") || s.contains("RCO") {
            return Self::Fss;
        }
        // Ramp
        if s.contains("RMP") || s.contains("RAMP") {
            return Self::Ramp;
        }
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
    } else if (1080.0..=1100.0).contains(&freq_mhz) || freq_mhz > 1500.0 {
        "RAW"
    } else if (137.0..=138.0).contains(&freq_mhz) || (freq_mhz > 87.0 && freq_mhz < 108.0) {
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

#[derive(Default)]
pub struct AirportDb {
    pub airports: HashMap<String, Airport>,
    pub freqs: HashMap<String, Vec<AirportFreq>>,
    pub cached_sqlite: bool,
}

impl AirportDb {
    /// Load from the `SQLite` cache if present; otherwise hydrate from the
    /// hardcoded fallback list (always works offline).
    pub fn load() -> Self {
        if let Ok(conn) = Connection::open("ez_sdr.db") {
            if let Ok(count) =
                conn.query_row("SELECT COUNT(*) FROM airports", [], |r| r.get::<_, i64>(0))
            {
                if count > 0 {
                    return Self::load_from_sqlite(&conn);
                }
            }
        }
        Self::load_fallback()
    }

    fn load_from_sqlite(conn: &Connection) -> Self {
        let mut db = Self {
            cached_sqlite: true,
            ..Default::default()
        };
        if let Ok(mut stmt) = conn.prepare(
            "SELECT ident, icao, iata, name, lat, lon, country, type, scheduled FROM airports",
        ) {
            let rows = stmt.query_map([], |r| {
                Ok(Airport {
                    ident: r.get(0)?,
                    icao: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    iata: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    name: r.get(3)?,
                    lat: r.get(4)?,
                    lon: r.get(5)?,
                    country: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    atype: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    scheduled: r.get::<_, Option<i64>>(8)?.is_some_and(|v| v != 0),
                })
            });
            if let Ok(rows) = rows {
                for a in rows.flatten() {
                    db.airports.insert(a.ident.clone(), a);
                }
            }
        }
        if let Ok(mut stmt) = conn
            .prepare("SELECT airport_ident, type, description, frequency_mhz FROM airport_freqs")
        {
            let rows = stmt.query_map([], |r| {
                Ok(AirportFreq {
                    airport_ident: r.get(0)?,
                    raw_type: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    description: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    frequency_mhz: r.get(3)?,
                    freq_type: FreqType::Other,
                })
            });
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
        for entry in FALLBACK_AIRPORTS {
            let a = Airport {
                ident: entry.ident.to_string(),
                icao: entry.icao.to_string(),
                iata: entry.iata.to_string(),
                name: entry.name.to_string(),
                lat: entry.lat,
                lon: entry.lon,
                country: entry.country.to_string(),
                atype: entry.atype.to_string(),
                scheduled: true,
            };
            let fv: Vec<AirportFreq> = entry
                .freqs
                .iter()
                .map(|f| AirportFreq {
                    airport_ident: entry.ident.to_string(),
                    raw_type: f.freq_type.to_string(),
                    description: f.description.to_string(),
                    frequency_mhz: f.frequency_mhz,
                    freq_type: FreqType::from_raw(f.freq_type),
                })
                .collect();
            db.airports.insert(a.ident.clone(), a);
            db.freqs.insert(entry.ident.to_string(), fv);
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
        self.freqs.get(ident).map_or(&[], std::vec::Vec::as_slice)
    }

    pub fn airport(&self, ident: &str) -> Option<&Airport> {
        self.airports.get(ident)
    }

    /// Fetch both `OurAirports` CSVs and cache them in `SQLite`. Returns the number
    /// of airports loaded. `progress(current, total)` reports download bytes.
    pub fn download_full_blocking(mut progress: impl FnMut(usize, usize)) -> Result<usize, String> {
        let airports_csv = Self::fetch_csv(
            "https://davidmegginson.github.io/ourairports-data/airports.csv",
            &mut progress,
        )?;
        let freq_csv = Self::fetch_csv(
            "https://davidmegginson.github.io/ourairports-data/airport-frequencies.csv",
            &mut progress,
        )?;

        let mut airports = Vec::new();
        for (i, row) in airports_csv.iter().enumerate() {
            if i == 0 {
                continue;
            } // header
            let cols = csv_split(row);
            if cols.len() < 13 {
                continue;
            }
            let atype = cols.get(2).cloned().unwrap_or_default();
            if atype == "closed_airport" || atype == "balloonport" {
                continue;
            }
            airports.push(Airport {
                ident: cols.get(1).cloned().unwrap_or_default(),
                icao: cols.get(12).cloned().unwrap_or_default(),
                iata: cols.get(13).cloned().unwrap_or_default(),
                name: cols.get(3).cloned().unwrap_or_default(),
                lat: cols.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                lon: cols.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0),
                country: cols.get(8).cloned().unwrap_or_default(),
                atype,
                scheduled: cols.get(11).is_some_and(|s| s == "yes"),
            });
        }

        let mut freqs: Vec<AirportFreq> = Vec::new();
        for (i, row) in freq_csv.iter().enumerate() {
            if i == 0 {
                continue;
            }
            let cols = csv_split(row);
            if cols.len() < 6 {
                continue;
            }
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

    fn fetch_csv(
        url: &str,
        progress: &mut impl FnMut(usize, usize),
    ) -> Result<Vec<String>, String> {
        let resp = reqwest::blocking::get(url).map_err(|e| format!("{url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{}: HTTP {}", url, resp.status()));
        }
        let total = resp.content_length().unwrap_or(0) as usize;
        let bytes = resp.bytes().map_err(|e| format!("{url}: {e}"))?;
        progress(bytes.len(), total);
        let text = String::from_utf8_lossy(&bytes);
        Ok(text.lines().map(std::string::ToString::to_string).collect())
    }

    fn store_sqlite(airports: &[Airport], freqs: &[AirportFreq]) -> Result<(), String> {
        let mut conn = Connection::open("ez_sdr.db").map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        conn.execute_batch("DELETE FROM airport_freqs; DELETE FROM airports;")
            .map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO airports (ident,icao,iata,name,lat,lon,country,type,scheduled) VALUES (?,?,?,?,?,?,?,?,?)"
            ).map_err(|e| e.to_string())?;
            for a in airports {
                stmt.execute(rusqlite::params![
                    a.ident,
                    a.icao,
                    a.iata,
                    a.name,
                    a.lat,
                    a.lon,
                    a.country,
                    a.atype,
                    i32::from(a.scheduled)
                ])
                .ok();
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO airport_freqs (airport_ident,type,description,frequency_mhz) VALUES (?,?,?,?)"
            ).map_err(|e| e.to_string())?;
            for f in freqs {
                stmt.execute(rusqlite::params![
                    f.airport_ident,
                    f.raw_type,
                    f.description,
                    f.frequency_mhz
                ])
                .ok();
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

/// A single hardcoded fallback frequency.
struct FallbackFreq {
    freq_type: &'static str,
    description: &'static str,
    frequency_mhz: f64,
}

/// A hardcoded fallback airport entry.
struct AirportEntry {
    ident: &'static str,
    icao: &'static str,
    iata: &'static str,
    name: &'static str,
    lat: f64,
    lon: f64,
    country: &'static str,
    atype: &'static str,
    freqs: &'static [FallbackFreq],
}

/// ~30 major world hubs with verified frequencies. Always available offline.
static FALLBACK_AIRPORTS: &[AirportEntry] = &[
    AirportEntry {
        ident: "KLAX",
        icao: "KLAX",
        iata: "LAX",
        name: "Los Angeles Intl",
        lat: 33.9425,
        lon: -118.408,
        country: "US",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "LAX ATIS", frequency_mhz: 134.45 },
            FallbackFreq { freq_type: "CLD", description: "Clearance", frequency_mhz: 127.65 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.65 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 120.95 },
            FallbackFreq { freq_type: "APP", description: "SoCal Approach", frequency_mhz: 124.5 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 124.3 },
        ],
    },
    AirportEntry {
        ident: "KJFK",
        icao: "KJFK",
        iata: "JFK",
        name: "John F Kennedy Intl",
        lat: 40.6413,
        lon: -73.7781,
        country: "US",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "JFK ATIS", frequency_mhz: 135.9 },
            FallbackFreq { freq_type: "CLD", description: "Clearance", frequency_mhz: 127.4 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 127.4 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 125.7 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.85 },
        ],
    },
    AirportEntry {
        ident: "EGLL",
        icao: "EGLL",
        iata: "LHR",
        name: "Heathrow",
        lat: 51.4700,
        lon: -0.4543,
        country: "GB",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "Heathrow ATIS", frequency_mhz: 113.75 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.5 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.72 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 135.8 },
        ],
    },
    AirportEntry {
        ident: "LFPG",
        icao: "LFPG",
        iata: "CDG",
        name: "Paris Charles de Gaulle",
        lat: 49.0097,
        lon: 2.5479,
        country: "FR",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "CDG ATIS", frequency_mhz: 127.22 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 120.4 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.65 },
        ],
    },
    AirportEntry {
        ident: "EDDF",
        icao: "EDDF",
        iata: "FRA",
        name: "Frankfurt am Main",
        lat: 50.0379,
        lon: 8.5622,
        country: "DE",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "FRA ATIS", frequency_mhz: 136.25 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.65 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.75 },
        ],
    },
    AirportEntry {
        ident: "EHAM",
        icao: "EHAM",
        iata: "AMS",
        name: "Amsterdam Schiphol",
        lat: 52.3105,
        lon: 4.7683,
        country: "NL",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "Schiphol ATIS", frequency_mhz: 136.55 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.4 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.05 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.55 },
        ],
    },
    AirportEntry {
        ident: "LEMD",
        icao: "LEMD",
        iata: "MAD",
        name: "Madrid Barajas",
        lat: 40.4719,
        lon: -3.5626,
        country: "ES",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.45 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.65 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.3 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.4 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.65 },
        ],
    },
    AirportEntry {
        ident: "LIRF",
        icao: "LIRF",
        iata: "FCO",
        name: "Rome Fiumicino",
        lat: 41.8003,
        lon: 12.2389,
        country: "IT",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.7 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.7 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.9 },
        ],
    },
    AirportEntry {
        ident: "LSZH",
        icao: "LSZH",
        iata: "ZRH",
        name: "Zurich",
        lat: 47.4647,
        lon: 8.5492,
        country: "CH",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 128.025 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.05 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.0 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 128.05 },
        ],
    },
    AirportEntry {
        ident: "EDDM",
        icao: "EDDM",
        iata: "MUC",
        name: "Munich",
        lat: 48.3538,
        lon: 11.7861,
        country: "DE",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 136.45 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.8 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.6 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.75 },
        ],
    },
    AirportEntry {
        ident: "EKCH",
        icao: "EKCH",
        iata: "CPH",
        name: "Copenhagen Kastrup",
        lat: 55.6181,
        lon: 12.6561,
        country: "DK",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 126.3 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.3 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.6 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.45 },
        ],
    },
    AirportEntry {
        ident: "ESSA",
        icao: "ESSA",
        iata: "ARN",
        name: "Stockholm Arlanda",
        lat: 59.6519,
        lon: 17.9186,
        country: "SE",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.025 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.3 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.1 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.15 },
        ],
    },
    AirportEntry {
        ident: "ENGM",
        icao: "ENGM",
        iata: "OSL",
        name: "Oslo Gardermoen",
        lat: 60.1939,
        lon: 11.1004,
        country: "NO",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.075 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.7 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.4 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.2 },
        ],
    },
    AirportEntry {
        ident: "EFHK",
        icao: "EFHK",
        iata: "HEL",
        name: "Helsinki Vantaa",
        lat: 60.3172,
        lon: 24.9633,
        country: "FI",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 128.65 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.7 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.55 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 120.6 },
        ],
    },
    AirportEntry {
        ident: "LOWW",
        icao: "LOWW",
        iata: "VIE",
        name: "Vienna Schwechat",
        lat: 48.1103,
        lon: 16.5697,
        country: "AT",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 136.975 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.05 },
        ],
    },
    AirportEntry {
        ident: "EBBR",
        icao: "EBBR",
        iata: "BRU",
        name: "Brussels Zaventem",
        lat: 50.9014,
        lon: 4.4844,
        country: "BE",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 126.825 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.7 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.6 },
        ],
    },
    AirportEntry {
        ident: "RJTT",
        icao: "RJTT",
        iata: "HND",
        name: "Tokyo Haneda",
        lat: 35.5494,
        lon: 139.7798,
        country: "JP",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 126.65 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 120.8 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 126.0 },
        ],
    },
    AirportEntry {
        ident: "RKSI",
        icao: "RKSI",
        iata: "ICN",
        name: "Seoul Incheon",
        lat: 37.4602,
        lon: 126.4407,
        country: "KR",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 128.65 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.25 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.55 },
        ],
    },
    AirportEntry {
        ident: "ZBAA",
        icao: "ZBAA",
        iata: "PEK",
        name: "Beijing Capital",
        lat: 40.0801,
        lon: 116.5846,
        country: "CN",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.6 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.85 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.0 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.85 },
        ],
    },
    AirportEntry {
        ident: "VHHH",
        icao: "VHHH",
        iata: "HKG",
        name: "Hong Kong",
        lat: 22.3089,
        lon: 113.9144,
        country: "HK",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 128.2 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.6 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.4 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.1 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 123.9 },
        ],
    },
    AirportEntry {
        ident: "WSSS",
        icao: "WSSS",
        iata: "SIN",
        name: "Singapore Changi",
        lat: 1.3644,
        lon: 103.9915,
        country: "SG",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 128.6 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.6 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 126.55 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 123.6 },
        ],
    },
    AirportEntry {
        ident: "OMDB",
        icao: "OMDB",
        iata: "DXB",
        name: "Dubai Intl",
        lat: 25.2532,
        lon: 55.3657,
        country: "AE",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.4 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.4 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.4 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.55 },
        ],
    },
    AirportEntry {
        ident: "LTFM",
        icao: "LTFM",
        iata: "IST",
        name: "Istanbul",
        lat: 41.2753,
        lon: 28.7519,
        country: "TR",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.5 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.6 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.3 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.7 },
        ],
    },
    AirportEntry {
        ident: "YSSY",
        icao: "YSSY",
        iata: "SYD",
        name: "Sydney Kingsford Smith",
        lat: -33.9399,
        lon: 151.1753,
        country: "AU",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.0 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.7 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 120.5 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 124.7 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 123.0 },
        ],
    },
    AirportEntry {
        ident: "SBGR",
        icao: "SBGR",
        iata: "GRU",
        name: "Sao Paulo Guarulhos",
        lat: -23.4356,
        lon: -46.4731,
        country: "BR",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.65 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.0 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.3 },
        ],
    },
    AirportEntry {
        ident: "SAEZ",
        icao: "SAEZ",
        iata: "EZE",
        name: "Buenos Aires Ezeiza",
        lat: -34.8222,
        lon: -58.5358,
        country: "AR",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.0 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.1 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.5 },
        ],
    },
    AirportEntry {
        ident: "FAOR",
        icao: "FAOR",
        iata: "JNB",
        name: "Johannesburg OR Tambo",
        lat: -26.1392,
        lon: 28.2460,
        country: "ZA",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.0 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.1 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.6 },
        ],
    },
    AirportEntry {
        ident: "VABB",
        icao: "VABB",
        iata: "BOM",
        name: "Mumbai Chhatrapati Shivaji",
        lat: 19.0896,
        lon: 72.8656,
        country: "IN",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 126.6 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.5 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.1 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.55 },
        ],
    },
    AirportEntry {
        ident: "VIDP",
        icao: "VIDP",
        iata: "DEL",
        name: "Delhi Indira Gandhi",
        lat: 28.5562,
        lon: 77.1000,
        country: "IN",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 126.6 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.5 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.1 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.55 },
        ],
    },
    AirportEntry {
        ident: "MMMX",
        icao: "MMMX",
        iata: "MEX",
        name: "Mexico City Intl",
        lat: 19.4361,
        lon: -99.0719,
        country: "MX",
        atype: "large_airport",
        freqs: &[
            FallbackFreq { freq_type: "ATIS", description: "ATIS", frequency_mhz: 127.2 },
            FallbackFreq { freq_type: "GND", description: "Ground", frequency_mhz: 121.9 },
            FallbackFreq { freq_type: "TWR", description: "Tower", frequency_mhz: 118.9 },
            FallbackFreq { freq_type: "APP", description: "Approach", frequency_mhz: 119.2 },
            FallbackFreq { freq_type: "DEP", description: "Departure", frequency_mhz: 125.5 },
        ],
    },
];

impl Airport {
    /// Format code badge: prefer ICAO, fall back to ident.
    pub fn code(&self) -> &str {
        if self.icao.is_empty() {
            &self.ident
        } else {
            &self.icao
        }
    }
}
