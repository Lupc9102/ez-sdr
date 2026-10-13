//! Explicit scan-geometry georeferencing for LRPT channel images.
//!
//! A TLE alone does not define instrument attitude or scan width. This module
//! therefore requires callers to provide a validated Earth-fixed orbit state
//! and the instrument's measured/documented edge scan angle; it never fills in
//! missing geometry with a guessed footprint.

use image::{GrayImage, Rgba, RgbaImage};
use serde::Serialize;

const WGS84_A_KM: f64 = 6_378.137;
const WGS84_B_KM: f64 = 6_356.752_314_245;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarthFixedOrbitState {
    /// Satellite position in Earth-fixed kilometers.
    pub position_km: [f64; 3],
    /// Earth-fixed velocity in kilometers per second.
    pub velocity_km_s: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanGeometry {
    /// Scan angle at the first image pixel, measured from nadir in radians.
    pub first_pixel_angle_rad: f64,
    /// Scan angle at the last image pixel, measured from nadir in radians.
    pub last_pixel_angle_rad: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundPoint {
    pub longitude_deg: f64,
    pub latitude_deg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroundControlPoint {
    pub pixel_x: u32,
    pub pixel_y: u32,
    pub longitude_deg: f64,
    pub latitude_deg: f64,
    pub unix_time_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeoReferencedImage {
    pub width: u32,
    pub height: u32,
    /// Ground points at each image row, sampled at the requested pixel stride.
    pub scanlines: Vec<Vec<Option<GroundPoint>>>,
    pub ground_control_points: Vec<GroundControlPoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedRaster {
    pub image: RgbaImage,
    /// Continuous longitude bounds; these intentionally pass through 180°
    /// when a swath crosses the antimeridian.
    pub longitude_bounds_deg: [f64; 2],
    pub latitude_bounds_deg: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeoreferenceProducts {
    pub vectors: GeoReferencedImage,
    pub projected: ProjectedRaster,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GeolocationError {
    #[error("image dimensions must be nonzero")]
    EmptyImage,
    #[error("one finite timestamp and orbit state are required for each scanline")]
    InvalidLineMetadata,
    #[error("scan geometry must have finite edge angles within ±80 degrees")]
    InvalidScanGeometry,
    #[error("orbit state must contain finite position and nonzero velocity outside the Earth")]
    InvalidOrbitState,
    #[error("none of the sensor rays intersect the WGS84 ellipsoid")]
    NoGroundIntersections,
}

/// Intersect one sensor ray with the WGS84 reference ellipsoid. Positive scan
/// angle follows the cross-track axis `nadir × velocity`, matching an explicit
/// left-to-right image scan when the input vectors use the same frame.
pub fn intersect_scan_ray(
    orbit: EarthFixedOrbitState,
    angle_rad: f64,
) -> Result<Option<GroundPoint>, GeolocationError> {
    validate_orbit(orbit)?;
    if !angle_rad.is_finite() || angle_rad.abs() > 80_f64.to_radians() {
        return Err(GeolocationError::InvalidScanGeometry);
    }
    scan_ray_unchecked(orbit, angle_rad)
}

fn scan_ray_unchecked(
    orbit: EarthFixedOrbitState,
    angle_rad: f64,
) -> Result<Option<GroundPoint>, GeolocationError> {
    let radial = normalize(orbit.position_km);
    let nadir = scale(radial, -1.0);
    let cross_track = normalize(cross(radial, orbit.velocity_km_s));
    let direction = normalize(add(
        scale(nadir, angle_rad.cos()),
        scale(cross_track, angle_rad.sin()),
    ));
    let [px, py, pz] = orbit.position_km;
    let [dx, dy, dz] = direction;
    let a2 = WGS84_A_KM * WGS84_A_KM;
    let b2 = WGS84_B_KM * WGS84_B_KM;
    let qa = (dx * dx + dy * dy) / a2 + dz * dz / b2;
    let qb = 2.0 * ((px * dx + py * dy) / a2 + pz * dz / b2);
    let qc = (px * px + py * py) / a2 + pz * pz / b2 - 1.0;
    let discriminant = qb.mul_add(qb, -4.0 * qa * qc);
    if discriminant < 0.0 {
        return Ok(None);
    }
    let root = discriminant.sqrt();
    let near = (-qb - root) / (2.0 * qa);
    let far = (-qb + root) / (2.0 * qa);
    let distance = [near, far]
        .into_iter()
        .filter(|distance| *distance >= 0.0)
        .fold(f64::INFINITY, f64::min);
    if !distance.is_finite() {
        return Ok(None);
    }
    let [x, y, z] = add(orbit.position_km, scale(direction, distance));
    let longitude_deg = y.atan2(x).to_degrees();
    let latitude_deg = (z / (x * x + y * y).sqrt()).atan().to_degrees();
    Ok(Some(GroundPoint {
        longitude_deg,
        latitude_deg,
    }))
}

/// Geolocate scan rows using per-row TLE-propagated orbit states and actual
/// line times. Pixels whose rays miss the Earth remain explicitly unlocated.
pub fn geolocate_scanlines(
    width: u32,
    line_times_unix_seconds: &[f64],
    orbit_states: &[EarthFixedOrbitState],
    geometry: ScanGeometry,
    control_point_stride: u32,
) -> Result<GeoReferencedImage, GeolocationError> {
    if width == 0 || line_times_unix_seconds.is_empty() {
        return Err(GeolocationError::EmptyImage);
    }
    if line_times_unix_seconds.len() != orbit_states.len()
        || line_times_unix_seconds.iter().any(|time| !time.is_finite())
    {
        return Err(GeolocationError::InvalidLineMetadata);
    }
    validate_geometry(geometry)?;
    for &orbit in orbit_states {
        validate_orbit(orbit)?;
    }
    let height = line_times_unix_seconds.len() as u32;
    let mut scanlines = Vec::with_capacity(line_times_unix_seconds.len());
    let mut ground_control_points = Vec::new();
    let stride = control_point_stride.max(1);
    for (row, (&time, &orbit)) in line_times_unix_seconds.iter().zip(orbit_states).enumerate() {
        let mut points = Vec::with_capacity((width as usize).div_ceil(stride as usize));
        let mut x = 0;
        while x < width {
            let fraction = if width == 1 {
                0.5
            } else {
                f64::from(x) / f64::from(width - 1)
            };
            let angle = geometry.first_pixel_angle_rad
                + fraction * (geometry.last_pixel_angle_rad - geometry.first_pixel_angle_rad);
            let point = scan_ray_unchecked(orbit, angle)?;
            if let Some(ground) = point {
                ground_control_points.push(GroundControlPoint {
                    pixel_x: x,
                    pixel_y: row as u32,
                    longitude_deg: ground.longitude_deg,
                    latitude_deg: ground.latitude_deg,
                    unix_time_seconds: time,
                });
            }
            points.push(point);
            x = x.saturating_add(stride);
        }
        if !(width - 1).is_multiple_of(stride) {
            let x = width - 1;
            let fraction = if width == 1 {
                0.5
            } else {
                f64::from(x) / f64::from(width - 1)
            };
            let angle = geometry.first_pixel_angle_rad
                + fraction * (geometry.last_pixel_angle_rad - geometry.first_pixel_angle_rad);
            let point = intersect_scan_ray(orbit, angle)?;
            if let Some(ground) = point {
                ground_control_points.push(GroundControlPoint {
                    pixel_x: x,
                    pixel_y: row as u32,
                    longitude_deg: ground.longitude_deg,
                    latitude_deg: ground.latitude_deg,
                    unix_time_seconds: time,
                });
            }
            points.push(point);
        }
        scanlines.push(points);
    }
    Ok(GeoReferencedImage {
        width,
        height,
        scanlines,
        ground_control_points,
    })
}

/// Nearest-neighbor regrid a grayscale LRPT image onto a WGS84
/// equirectangular (longitude/latitude) raster. The source-to-ground pass is
/// performed twice with only one row of coordinates resident at a time. The
/// output alpha channel remains zero for unobserved target pixels.
pub fn project_equirectangular_nearest(
    source: &GrayImage,
    line_times_unix_seconds: &[f64],
    orbit_states: &[EarthFixedOrbitState],
    geometry: ScanGeometry,
    control_point_stride: u32,
) -> Result<(GeoReferencedImage, ProjectedRaster), GeolocationError> {
    let width = source.width();
    let height = source.height();
    if width == 0 || height == 0 {
        return Err(GeolocationError::EmptyImage);
    }
    if line_times_unix_seconds.len() != height as usize
        || orbit_states.len() != height as usize
        || line_times_unix_seconds.iter().any(|time| !time.is_finite())
    {
        return Err(GeolocationError::InvalidLineMetadata);
    }
    validate_geometry(geometry)?;
    for &orbit in orbit_states {
        validate_orbit(orbit)?;
    }

    let stride = control_point_stride.max(1);
    let mut scanlines = Vec::with_capacity(height as usize);
    let mut ground_control_points = Vec::new();
    let mut bounds = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut previous_row_start = None;

    // Pass one gathers exact bounds and sparse source-pixel GCPs without
    // retaining an image-sized coordinate array.
    for (row, (&time, &orbit)) in line_times_unix_seconds.iter().zip(orbit_states).enumerate() {
        let mut points = Vec::with_capacity((width as usize).div_ceil(stride as usize) + 1);
        let mut previous_lon = None;
        let mut x = 0;
        while x < width {
            let point = scan_ground_point(orbit, x, width, geometry)?;
            record_projected_point(
                point,
                x,
                row as u32,
                time,
                x.is_multiple_of(stride) || x == width - 1,
                previous_lon,
                previous_row_start,
                &mut previous_lon,
                &mut points,
                &mut ground_control_points,
                &mut bounds,
            );
            if x == 0 {
                previous_row_start = previous_lon;
            }
            x = x.saturating_add(stride);
        }
        if !(width - 1).is_multiple_of(stride) {
            let x = width - 1;
            let point = scan_ground_point(orbit, x, width, geometry)?;
            record_projected_point(
                point,
                x,
                row as u32,
                time,
                true,
                previous_lon,
                previous_row_start,
                &mut previous_lon,
                &mut points,
                &mut ground_control_points,
                &mut bounds,
            );
        }
        scanlines.push(points);
    }
    if !bounds.iter().all(|value| value.is_finite()) {
        return Err(GeolocationError::NoGroundIntersections);
    }

    // Pass two reprojects each source sample to its nearest target pixel.
    let mut projected = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    let mut previous_row_start = None;
    for (y, &orbit) in orbit_states.iter().enumerate() {
        let mut previous_lon = None;
        for x in 0..width {
            let Some(point) = scan_ground_point(orbit, x, width, geometry)? else {
                continue;
            };
            let reference = previous_lon
                .or(previous_row_start)
                .unwrap_or(point.longitude_deg);
            let longitude = unwrap_longitude(point.longitude_deg, reference);
            if x == 0 {
                previous_row_start = Some(longitude);
            }
            previous_lon = Some(longitude);
            let target_x = nearest_grid_index(longitude, bounds[0], bounds[1], width);
            let target_y = nearest_grid_index(
                bounds[3] - point.latitude_deg,
                0.0,
                bounds[3] - bounds[2],
                height,
            );
            let pixel = projected.get_pixel_mut(target_x, target_y);
            if pixel[3] == 0 {
                let value = source.get_pixel(x, y as u32)[0];
                *pixel = Rgba([value, value, value, 255]);
            }
        }
    }
    Ok((
        GeoReferencedImage {
            width,
            height,
            scanlines,
            ground_control_points,
        },
        ProjectedRaster {
            image: projected,
            longitude_bounds_deg: [bounds[0], bounds[1]],
            latitude_bounds_deg: [bounds[2], bounds[3]],
        },
    ))
}

fn scan_ground_point(
    orbit: EarthFixedOrbitState,
    x: u32,
    width: u32,
    geometry: ScanGeometry,
) -> Result<Option<GroundPoint>, GeolocationError> {
    let fraction = if width == 1 {
        0.5
    } else {
        f64::from(x) / f64::from(width - 1)
    };
    let angle = geometry.first_pixel_angle_rad
        + fraction * (geometry.last_pixel_angle_rad - geometry.first_pixel_angle_rad);
    scan_ray_unchecked(orbit, angle)
}

#[allow(clippy::too_many_arguments)]
fn record_projected_point(
    point: Option<GroundPoint>,
    x: u32,
    y: u32,
    time: f64,
    include_sample: bool,
    previous_lon: Option<f64>,
    previous_row_start: Option<f64>,
    next_previous_lon: &mut Option<f64>,
    scanline: &mut Vec<Option<GroundPoint>>,
    gcps: &mut Vec<GroundControlPoint>,
    bounds: &mut [f64; 4],
) {
    let Some(point) = point else {
        if include_sample {
            scanline.push(None);
        }
        return;
    };
    let reference = previous_lon
        .or(previous_row_start)
        .unwrap_or(point.longitude_deg);
    let longitude = unwrap_longitude(point.longitude_deg, reference);
    *next_previous_lon = Some(longitude);
    bounds[0] = bounds[0].min(longitude);
    bounds[1] = bounds[1].max(longitude);
    bounds[2] = bounds[2].min(point.latitude_deg);
    bounds[3] = bounds[3].max(point.latitude_deg);
    if include_sample {
        scanline.push(Some(point));
        gcps.push(GroundControlPoint {
            pixel_x: x,
            pixel_y: y,
            longitude_deg: longitude,
            latitude_deg: point.latitude_deg,
            unix_time_seconds: time,
        });
    }
}

fn unwrap_longitude(longitude: f64, reference: f64) -> f64 {
    longitude + 360.0 * ((reference - longitude) / 360.0).round()
}

fn nearest_grid_index(value: f64, minimum: f64, maximum: f64, size: u32) -> u32 {
    if size <= 1 || maximum <= minimum {
        return size.saturating_sub(1) / 2;
    }
    (((value - minimum) / (maximum - minimum) * f64::from(size - 1)).round() as i64)
        .clamp(0, i64::from(size - 1)) as u32
}

#[derive(Serialize)]
struct FeatureCollection {
    #[serde(rename = "type")]
    kind: &'static str,
    features: Vec<Feature>,
}

#[derive(Serialize)]
struct Feature {
    #[serde(rename = "type")]
    kind: &'static str,
    properties: LineProperties,
    geometry: Option<LineGeometry>,
}

#[derive(Serialize)]
struct LineProperties {
    row: usize,
    valid_points: usize,
}

#[derive(Serialize)]
struct LineGeometry {
    #[serde(rename = "type")]
    kind: &'static str,
    coordinates: Vec<[f64; 2]>,
}

/// Serialize valid scanline centers as GeoJSON LineStrings. Missing/out-of-Earth
/// points are skipped and never replaced by a guessed coordinate.
pub fn geojson(georeference: &GeoReferencedImage) -> Result<String, serde_json::Error> {
    let features = georeference
        .scanlines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let coordinates: Vec<_> = line
                .iter()
                .filter_map(|point| point.map(|p| [p.longitude_deg, p.latitude_deg]))
                .collect();
            Feature {
                kind: "Feature",
                properties: LineProperties {
                    row,
                    valid_points: coordinates.len(),
                },
                geometry: (coordinates.len() >= 2).then_some(LineGeometry {
                    kind: "LineString",
                    coordinates,
                }),
            }
        })
        .collect();
    serde_json::to_string_pretty(&FeatureCollection {
        kind: "FeatureCollection",
        features,
    })
}

/// Serialize sparse georeferencing control points for GDAL-compatible workflows.
pub fn gcps_json(georeference: &GeoReferencedImage) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&georeference.ground_control_points)
}

/// Create a GDAL VRT dataset document that references a source raster and
/// attaches the packet-time/TLE-derived ground control points. GDAL can warp
/// this VRT to a chosen map projection without pretending the orbital swath is
/// a simple affine world file.
pub fn vrt_document(
    raster_filename: &str,
    width: u32,
    height: u32,
    ground_control_points: &[GroundControlPoint],
) -> Result<String, String> {
    if width == 0 || height == 0 || ground_control_points.is_empty() {
        return Err(
            "a VRT needs nonzero raster dimensions and at least one ground control point".into(),
        );
    }
    let raster_filename = std::path::Path::new(raster_filename)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "the VRT source raster name is invalid".to_string())?;
    let mut gcps = String::new();
    for (index, gcp) in ground_control_points.iter().enumerate() {
        if gcp.pixel_x >= width
            || gcp.pixel_y >= height
            || !gcp.longitude_deg.is_finite()
            || !gcp.latitude_deg.is_finite()
            || !gcp.unix_time_seconds.is_finite()
        {
            return Err("VRT ground control points must be finite and inside the raster".into());
        }
        gcps.push_str(&format!(
            "    <GCP Id=\"{index}\" Pixel=\"{}\" Line=\"{}\" X=\"{:.10}\" Y=\"{:.10}\" Z=\"0\"/>\n",
            gcp.pixel_x, gcp.pixel_y, gcp.longitude_deg, gcp.latitude_deg
        ));
    }
    let escaped_filename = raster_filename
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Ok(format!(
        "<VRTDataset rasterXSize=\"{width}\" rasterYSize=\"{height}\">\n  <SRS>EPSG:4326</SRS>\n  <GCPList Projection=\"EPSG:4326\">\n{gcps}  </GCPList>\n  <VRTRasterBand dataType=\"Byte\" band=\"1\">\n    <ColorInterp>Gray</ColorInterp>\n    <SimpleSource>\n      <SourceFilename relativeToVRT=\"1\">{escaped_filename}</SourceFilename>\n      <SourceBand>1</SourceBand>\n      <SourceProperties RasterXSize=\"{width}\" RasterYSize=\"{height}\" DataType=\"Byte\" BlockXSize=\"{width}\" BlockYSize=\"1\"/>\n      <SrcRect xOff=\"0\" yOff=\"0\" xSize=\"{width}\" ySize=\"{height}\"/>\n      <DstRect xOff=\"0\" yOff=\"0\" xSize=\"{width}\" ySize=\"{height}\"/>\n    </SimpleSource>\n  </VRTRasterBand>\n</VRTDataset>\n"
    ))
}

/// Provenance and bounds for a WGS84 equirectangular regrid. Coordinates keep
/// a continuous longitude branch through the antimeridian; transparent pixels
/// are unobserved swath holes.
pub fn projected_sidecar_json(projected: &ProjectedRaster) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Sidecar {
        projection: &'static str,
        width: u32,
        height: u32,
        longitude_bounds_deg: [f64; 2],
        latitude_bounds_deg: [f64; 2],
        pixel_registration: &'static str,
        transparent_pixels: &'static str,
        provenance: &'static str,
    }
    serde_json::to_string_pretty(&Sidecar {
        projection: "WGS84 equirectangular longitude/latitude (EPSG:4326)",
        width: projected.image.width(),
        height: projected.image.height(),
        longitude_bounds_deg: projected.longitude_bounds_deg,
        latitude_bounds_deg: projected.latitude_bounds_deg,
        pixel_registration: "nearest-neighbor pixel centers",
        transparent_pixels: "unobserved: outside sensor rays or where rays miss the ellipsoid",
        provenance:
            "MSU-MR CDS packet line times + selected loaded TLE + supplied scan edge angles",
    })
}

fn validate_orbit(orbit: EarthFixedOrbitState) -> Result<(), GeolocationError> {
    let finite = orbit
        .position_km
        .iter()
        .chain(orbit.velocity_km_s.iter())
        .all(|v| v.is_finite());
    let radius = norm(orbit.position_km);
    let speed = norm(orbit.velocity_km_s);
    if !finite || radius <= WGS84_A_KM || speed <= f64::EPSILON {
        return Err(GeolocationError::InvalidOrbitState);
    }
    if norm(cross(orbit.velocity_km_s, orbit.position_km)) <= f64::EPSILON {
        return Err(GeolocationError::InvalidOrbitState);
    }
    Ok(())
}

fn validate_geometry(geometry: ScanGeometry) -> Result<(), GeolocationError> {
    if !geometry.first_pixel_angle_rad.is_finite()
        || !geometry.last_pixel_angle_rad.is_finite()
        || geometry.first_pixel_angle_rad.abs() > 80_f64.to_radians()
        || geometry.last_pixel_angle_rad.abs() > 80_f64.to_radians()
    {
        return Err(GeolocationError::InvalidScanGeometry);
    }
    Ok(())
}

fn norm(v: [f64; 3]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    scale(v, 1.0 / norm(v))
}
fn scale(v: [f64; 3], k: f64) -> [f64; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equatorial_orbit() -> EarthFixedOrbitState {
        EarthFixedOrbitState {
            position_km: [7000.0, 0.0, 0.0],
            velocity_km_s: [0.0, 7.5, 0.0],
        }
    }

    #[test]
    fn nadir_and_cross_track_ray_intersect_known_ellipsoid_points() {
        let center = intersect_scan_ray(equatorial_orbit(), 0.0)
            .unwrap()
            .unwrap();
        assert!(center.longitude_deg.abs() < 1e-9);
        assert!(center.latitude_deg.abs() < 1e-9);
        let north = intersect_scan_ray(equatorial_orbit(), 20_f64.to_radians())
            .unwrap()
            .unwrap();
        let south = intersect_scan_ray(equatorial_orbit(), -20_f64.to_radians())
            .unwrap()
            .unwrap();
        assert!((north.latitude_deg + south.latitude_deg).abs() < 1e-8);
        assert!(north.latitude_deg > 0.0 && south.latitude_deg < 0.0);
        assert!(
            north.latitude_deg.abs() > 1.0 && north.latitude_deg.abs() < 3.0,
            "intersection={north:?}"
        );
    }

    #[test]
    fn invalid_geometry_and_line_metadata_are_rejected() {
        assert_eq!(
            intersect_scan_ray(equatorial_orbit(), 90_f64.to_radians()),
            Err(GeolocationError::InvalidScanGeometry)
        );
        assert_eq!(
            geolocate_scanlines(
                4,
                &[1.0],
                &[],
                ScanGeometry {
                    first_pixel_angle_rad: 0.0,
                    last_pixel_angle_rad: 0.0
                },
                1
            ),
            Err(GeolocationError::InvalidLineMetadata)
        );
    }

    #[test]
    fn export_preserves_missing_intersections_and_emits_valid_geojson() {
        let georef = geolocate_scanlines(
            3,
            &[1.0],
            &[equatorial_orbit()],
            ScanGeometry {
                first_pixel_angle_rad: -0.1,
                last_pixel_angle_rad: 0.1,
            },
            1,
        )
        .unwrap();
        assert_eq!(georef.ground_control_points.len(), 3);
        let json = geojson(&georef).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "FeatureCollection");
        assert_eq!(parsed["features"][0]["geometry"]["type"], "LineString");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&gcps_json(&georef).unwrap())
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn geolocation_samples_large_cross_track_images_with_bounded_memory() {
        let georef = geolocate_scanlines(
            1_568,
            &[1.0, 2.0],
            &[equatorial_orbit(); 2],
            ScanGeometry {
                first_pixel_angle_rad: -0.2,
                last_pixel_angle_rad: 0.2,
            },
            100,
        )
        .unwrap();
        assert_eq!(georef.width, 1_568);
        assert_eq!(georef.scanlines.len(), 2);
        assert_eq!(georef.scanlines[0].len(), 17);
        assert_eq!(georef.ground_control_points[0].pixel_x, 0);
        assert_eq!(georef.ground_control_points[15].pixel_x, 1_500);
        assert_eq!(georef.ground_control_points[16].pixel_x, 1_567);
        assert!(georef.scanlines[0].len() < georef.width as usize);
    }

    #[test]
    fn vrt_references_raster_and_validates_real_ground_control_points() {
        let gcp = GroundControlPoint {
            pixel_x: 3,
            pixel_y: 2,
            longitude_deg: 12.5,
            latitude_deg: -4.25,
            unix_time_seconds: 1_700_000_000.0,
        };
        let vrt = vrt_document("channel&raw.png", 8, 6, std::slice::from_ref(&gcp)).unwrap();
        assert!(vrt.contains("<SRS>EPSG:4326</SRS>"));
        assert!(vrt.contains("<GCPList Projection=\"EPSG:4326\">"));
        assert!(!vrt.contains("<GeoTransform>"));
        assert!(vrt.contains("channel&amp;raw.png"));
        assert!(vrt.contains("Pixel=\"3\" Line=\"2\" X=\"12.5000000000\" Y=\"-4.2500000000\""));
        assert!(vrt_document("channel.png", 0, 6, &[gcp]).is_err());
    }

    #[test]
    fn nearest_neighbor_projection_preserves_observed_pixels_and_transparent_swath_holes() {
        let source = GrayImage::from_raw(5, 1, vec![10, 20, 30, 40, 50]).unwrap();
        let (vectors, projected) = project_equirectangular_nearest(
            &source,
            &[1.0],
            &[equatorial_orbit()],
            ScanGeometry {
                first_pixel_angle_rad: -80_f64.to_radians(),
                last_pixel_angle_rad: 80_f64.to_radians(),
            },
            1,
        )
        .unwrap();
        assert!(vectors.scanlines[0][0].is_none());
        assert!(vectors.scanlines[0][4].is_none());
        let opaque = projected
            .image
            .pixels()
            .filter(|pixel| pixel[3] == 255)
            .count();
        let transparent = projected
            .image
            .pixels()
            .filter(|pixel| pixel[3] == 0)
            .count();
        assert!(opaque > 0);
        assert!(transparent > 0);
        assert!(projected
            .image
            .pixels()
            .any(|pixel| pixel[3] == 255 && pixel[0] >= 20));
        assert!(projected.longitude_bounds_deg[0] <= projected.longitude_bounds_deg[1]);
        let metadata: serde_json::Value =
            serde_json::from_str(&projected_sidecar_json(&projected).unwrap()).unwrap();
        assert!(metadata["provenance"]
            .as_str()
            .unwrap()
            .contains("selected loaded TLE"));
        assert!(metadata["transparent_pixels"]
            .as_str()
            .unwrap()
            .contains("unobserved"));
    }

    #[test]
    fn longitude_unwrap_keeps_dateline_crossing_local() {
        assert!((unwrap_longitude(-179.8, 179.9) - 180.2).abs() < 1e-9);
        assert!((unwrap_longitude(179.8, -179.9) + 180.2).abs() < 1e-9);
    }

    #[test]
    fn projected_gcp_grid_retains_orbit_curvature_instead_of_an_affine_guess() {
        let source = GrayImage::from_pixel(5, 3, image::Luma([77]));
        let states = [
            EarthFixedOrbitState {
                position_km: [7_000.0, 0.0, 0.0],
                velocity_km_s: [0.0, 7.5, 0.0],
            },
            EarthFixedOrbitState {
                position_km: [6_999.0, 100.0, 2.0],
                velocity_km_s: [-0.1, 7.5, 0.2],
            },
            EarthFixedOrbitState {
                position_km: [6_996.0, 200.0, 5.0],
                velocity_km_s: [-0.2, 7.5, 0.4],
            },
        ];
        let (vectors, _) = project_equirectangular_nearest(
            &source,
            &[1.0, 2.0, 3.0],
            &states,
            ScanGeometry {
                first_pixel_angle_rad: -0.2,
                last_pixel_angle_rad: 0.2,
            },
            1,
        )
        .unwrap();
        let center: Vec<_> = vectors
            .ground_control_points
            .iter()
            .filter(|gcp| gcp.pixel_x == 2)
            .map(|gcp| [gcp.longitude_deg, gcp.latitude_deg])
            .collect();
        assert_eq!(center.len(), 3);
        let first_delta = center[1][0] - center[0][0];
        let second_delta = center[2][0] - center[1][0];
        assert!((first_delta - second_delta).abs() > 1e-7);
    }
}
