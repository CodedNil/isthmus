//! Geographic sun direction from NOAA's approximate solar equations.
//! <https://gml.noaa.gov/grad/solcalc/solareqns.PDF>
use core::f64::consts::TAU;
use isthmus::{
    ShaderData,
    glam::{Vec3, vec3},
};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, ShaderData)]
pub struct Sun {
    /// Direction toward the sun in the renderer's z-up world.
    pub direction: Vec3,
    pub radiance: Vec3,
    pub sky: Vec3,
}

/// Longitude is east-positive; north rotates clockwise from screen-up.
/// The default location is deliberately coarse, for a public floor plan.
#[derive(Clone, Copy)]
pub struct SunSettings {
    pub latitude: f64,
    pub longitude: f64,
    pub north_degrees: f64,
}
impl Default for SunSettings {
    fn default() -> Self {
        Self { latitude: 52.5, longitude: -1.5, north_degrees: 100.0 }
    }
}

impl Sun {
    pub fn at_utc(unix_seconds: f64, settings: SunSettings) -> Self {
        let unix_seconds = if unix_seconds.is_finite() { unix_seconds } else { 0.0 };
        let mut day = (unix_seconds / 86_400.0).floor() as i64;
        let hour = unix_seconds.rem_euclid(86_400.0) / 3600.0;
        let mut year = 1970;
        let year_days = |year: i32| if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
        while day < 0 {
            year -= 1;
            day += year_days(year);
        }
        while day >= year_days(year) {
            day -= year_days(year);
            year += 1;
        }
        let gamma = TAU / year_days(year) as f64 * (day as f64 + (hour - 12.0) / 24.0);
        let (sin, cos) = gamma.sin_cos();
        let (sin2, cos2) = (gamma * 2.0).sin_cos();
        let (sin3, cos3) = (gamma * 3.0).sin_cos();
        let equation = 229.18 * (0.000_075 + 0.001_868 * cos - 0.032_077 * sin - 0.014_615 * cos2 - 0.040_849 * sin2);
        let declination = 0.006_918 - 0.399_912 * cos + 0.070_257 * sin - 0.006_758 * cos2 + 0.000_907 * sin2
            - 0.002_697 * cos3
            + 0.001_48 * sin3;
        let latitude = if settings.latitude.is_finite() { settings.latitude.clamp(-90.0, 90.0) } else { 0.0 };
        let longitude = if settings.longitude.is_finite() { settings.longitude.rem_euclid(360.0) } else { 0.0 };
        let north = if settings.north_degrees.is_finite() { settings.north_degrees } else { 0.0 };
        let angle = (hour * 15.0 + equation * 0.25 + longitude - 180.0).to_radians();
        let (sin_h, cos_h) = angle.sin_cos();
        let (sin_d, cos_d) = declination.sin_cos();
        let (sin_l, cos_l) = latitude.to_radians().sin_cos();
        let east = -cos_d * sin_h;
        let northward = cos_l * sin_d - sin_l * cos_d * cos_h;
        let up = sin_l * sin_d + cos_l * cos_d * cos_h;
        let (sin_n, cos_n) = north.to_radians().sin_cos();
        let direction =
            vec3((east * cos_n + northward * sin_n) as f32, (east * sin_n - northward * cos_n) as f32, up as f32)
                .normalize();
        let elevation = direction.z;
        let daylight = ((elevation + 0.10) / 0.55).clamp(0.0, 1.0);
        let daylight = daylight * daylight * (3.0 - 2.0 * daylight);
        let horizon = (elevation / 0.10).clamp(0.0, 1.0);
        let horizon = horizon * horizon * (3.0 - 2.0 * horizon);
        let temperature = (elevation / 0.40).clamp(0.0, 1.0);
        let radiance = vec3(1.0, 0.43, 0.17).lerp(vec3(1.0, 0.98, 0.94), temperature) * (6.8 * horizon);
        let sky = vec3(0.0015, 0.0025, 0.006).lerp(vec3(1.20, 1.44, 1.90), daylight);
        Self { direction, radiance, sky }
    }
}

/// Wall clock, rather than animation time; UTC avoids timezone and DST errors.
#[cfg(not(target_arch = "wasm32"))]
pub fn utc_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |time| time.as_secs_f64())
}
#[cfg(target_arch = "wasm32")]
pub fn utc_now() -> f64 {
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date, js_name = now)]
        fn date_now() -> f64;
    }
    date_now() * 0.001
}

#[cfg(test)]
mod tests {
    use super::*;
    const EQUINOX: f64 = 1_773_964_800.0; // 2026-03-20 00:00 UTC

    #[test]
    fn sun_moves_east_to_west_and_sets() {
        let settings = SunSettings { latitude: 0.0, longitude: 0.0, north_degrees: 0.0 };
        let morning = Sun::at_utc(EQUINOX + 8.0 * 3600.0, settings);
        let noon = Sun::at_utc(EQUINOX + 12.0 * 3600.0, settings);
        let evening = Sun::at_utc(EQUINOX + 16.0 * 3600.0, settings);
        let night = Sun::at_utc(EQUINOX, settings);
        assert!(morning.direction.x > 0.5 && evening.direction.x < -0.5);
        assert!(noon.direction.z > 0.99);
        assert!(night.direction.z < 0.0 && night.radiance == Vec3::ZERO);
        assert!(night.sky.max_element() < 0.01);
        let rotated = Sun::at_utc(EQUINOX + 8.0 * 3600.0, SunSettings { north_degrees: 90.0, ..settings });
        assert!((rotated.direction.y - morning.direction.x).abs() < 0.001);
    }

    #[test]
    fn latitude_seasons_and_longitude_affect_the_sun() {
        let settings = SunSettings::default();
        let summer = Sun::at_utc(1_782_000_000.0 + 12.0 * 3600.0, settings); // June 21
        let winter = Sun::at_utc(1_797_811_200.0 + 12.0 * 3600.0, settings); // December 21
        assert!(summer.direction.z > winter.direction.z + 0.4);
        let west =
            Sun::at_utc(EQUINOX + 12.0 * 3600.0, SunSettings { longitude: -90.0, north_degrees: 0.0, ..settings });
        assert!(west.direction.x > 0.8);
    }
}
