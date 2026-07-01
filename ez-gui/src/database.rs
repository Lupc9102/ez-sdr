use rusqlite::{Connection, Result};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub struct Database {
    conn: Connection,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct AircraftRecord {
    pub icao: u32,
    pub callsign: String,
    pub first_seen: f64,
    pub last_seen: f64,
    pub count: u32,
    pub lat: f64,
    pub lon: f64,
    pub altitude: u32,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct PassRecord {
    pub satellite: String,
    pub aos: f64,
    pub los: f64,
    pub max_elevation: f64,
    pub frequency_hz: u64,
    pub recorded: bool,
    pub output_path: String,
    pub peak_snr: f32,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct BookmarkRecord {
    pub name: String,
    pub frequency_hz: u64,
    pub mode: String,
    pub bandwidth_hz: u32,
    pub notes: String,
}

#[allow(dead_code)]
impl Database {
    pub fn open_or_create() -> Result<Self> {
        let conn = Connection::open("ez_sdr.db")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS aircraft (
                icao INTEGER PRIMARY KEY,
                callsign TEXT DEFAULT '',
                first_seen REAL NOT NULL,
                last_seen REAL NOT NULL,
                count INTEGER DEFAULT 1,
                lat REAL DEFAULT 0.0,
                lon REAL DEFAULT 0.0,
                altitude INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS passes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                satellite TEXT NOT NULL,
                aos REAL NOT NULL,
                los REAL NOT NULL,
                max_elevation REAL NOT NULL,
                frequency_hz INTEGER NOT NULL,
                recorded INTEGER DEFAULT 0,
                output_path TEXT DEFAULT '',
                peak_snr REAL DEFAULT 0.0
            );
            CREATE TABLE IF NOT EXISTS bookmarks (
                name TEXT PRIMARY KEY,
                frequency_hz INTEGER NOT NULL,
                mode TEXT NOT NULL DEFAULT 'FM',
                bandwidth_hz INTEGER NOT NULL DEFAULT 12500,
                notes TEXT DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS sdr_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp REAL NOT NULL,
                event TEXT NOT NULL,
                frequency_hz INTEGER,
                gain_db REAL,
                notes TEXT DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS recordings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                filename TEXT NOT NULL,
                start_time REAL NOT NULL,
                end_time REAL,
                frequency_hz INTEGER NOT NULL,
                sample_rate INTEGER NOT NULL,
                format TEXT NOT NULL,
                size_bytes INTEGER DEFAULT 0,
                satellite TEXT,
                pass_id INTEGER REFERENCES passes(id)
            );
            CREATE TABLE IF NOT EXISTS airports (
                ident TEXT PRIMARY KEY,
                icao TEXT DEFAULT '',
                iata TEXT DEFAULT '',
                name TEXT NOT NULL,
                lat REAL DEFAULT 0.0,
                lon REAL DEFAULT 0.0,
                country TEXT DEFAULT '',
                type TEXT DEFAULT '',
                scheduled INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS airport_freqs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                airport_ident TEXT NOT NULL,
                type TEXT DEFAULT '',
                description TEXT DEFAULT '',
                frequency_mhz REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_freqs_airport ON airport_freqs(airport_ident);
        ")?;
        Ok(Self { conn })
    }

    pub fn upsert_aircraft(&self, record: &AircraftRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO aircraft (icao, callsign, first_seen, last_seen, count, lat, lon, altitude)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(icao) DO UPDATE SET
                callsign = CASE WHEN excluded.callsign != '' THEN excluded.callsign ELSE callsign END,
                last_seen = excluded.last_seen,
                count = count + 1,
                lat = excluded.lat,
                lon = excluded.lon,
                altitude = excluded.altitude",
            rusqlite::params![record.icao, record.callsign, record.first_seen, record.last_seen, record.count, record.lat, record.lon, record.altitude],
        )?;
        Ok(())
    }

    pub fn record_pass(&self, record: &PassRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO passes (satellite, aos, los, max_elevation, frequency_hz, recorded, output_path, peak_snr)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![record.satellite, record.aos, record.los, record.max_elevation, record.frequency_hz, record.recorded as i32, record.output_path, record.peak_snr],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn log_event(&self, event: &str, freq: u64, gain: f64, notes: &str) -> Result<()> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        self.conn.execute(
            "INSERT INTO sdr_logs (timestamp, event, frequency_hz, gain_db, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![ts, event, freq as i64, gain, notes],
        )?;
        Ok(())
    }

    pub fn get_all_aircraft(&self) -> Result<Vec<AircraftRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT icao, callsign, first_seen, last_seen, count, lat, lon, altitude FROM aircraft ORDER BY last_seen DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AircraftRecord {
                icao: row.get(0)?,
                callsign: row.get(1)?,
                first_seen: row.get(2)?,
                last_seen: row.get(3)?,
                count: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                altitude: row.get(7)?,
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(rows)
    }

    pub fn get_aircraft_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM aircraft", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}
