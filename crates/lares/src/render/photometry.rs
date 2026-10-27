//! Photometric scale: one linear renderer unit = 100 lux (incident) or
//! 100 cd/m² (radiance). Geometry is in metres. RGB luminance is CIE Y.
use core::f32::consts::TAU;
use isthmus::glam::{Vec3, vec3};

pub const UNITS: f32 = 100.0;
/// IKEA TRÅDFRI GU10 colour/white spectrum: 345 lm, 40° FWHM, 2700 K.
/// <https://www.ikea.com/kw/en/p/tradfri-led-bulb-gu10-345-lumen-wireless-dimmable-colour-and-white-spectrum-70547474/>
pub const GU10_LUMENS: f32 = 345.0;
/// Mix of cos^11 and cos^12 fitted to exactly half intensity at 20°.
/// Polynomial evaluation avoids a per-light GPU logarithm/exponential.
pub const GU10_BEAM_MIX: f32 = 0.147_253_86;
pub const GU10_PEAK_CANDELA: f32 = GU10_LUMENS / (TAU * ((1.0 - GU10_BEAM_MIX) / 12.0 + GU10_BEAM_MIX / 13.0));
/// Finite shadow maps bound negligible distant contributions. No artificial
/// distance attenuation is applied inside 7 m; fade only over the final metre.
pub const LIGHT_RANGE: f32 = 8.0;
/// Fixed indoor camera exposure; exposure is separate from source brightness.
pub const EXPOSURE: f32 = UNITS / (1.2 * 64.0); // EV100 = 6

/// Linear sRGB from CIE xy, normalized to luminance Y=1, rather than max RGB=1.
/// This preserves lumens when changing the white point.
pub fn xy_rgb(x: f32, y: f32) -> Vec3 {
    let xyz = vec3(x / y, 1.0, (1.0 - x - y) / y);
    let rgb = vec3(
        xyz.dot(vec3(3.2406, -1.5372, -0.4986)),
        xyz.dot(vec3(-0.9689, 1.8758, 0.0415)),
        xyz.dot(vec3(0.0557, -0.2040, 1.0570)),
    )
    .max(Vec3::ZERO);
    rgb / rgb.dot(vec3(0.2126, 0.7152, 0.0722))
}

/// Planckian-locus CCT approximation for warm LEDs. CCT specifies chromaticity,
/// not the LED's full spectrum or its colour rendering index.
pub fn warm_white(kelvin: f32) -> Vec3 {
    let t = kelvin.clamp(1667.0, 4000.0);
    let x = -0.266_123_9e9 / t.powi(3) - 0.234_358e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_91;
    let y = if t < 2222.0 {
        -1.106_381_4 * x.powi(3) - 1.348_110_2 * x.powi(2) + 2.185_558_3 * x - 0.202_196_84
    } else {
        -0.954_947_6 * x.powi(3) - 1.374_185_9 * x.powi(2) + 2.091_37 * x - 0.167_488_66
    };
    xy_rgb(x, y)
}

/// CIE D-series daylight locus, with D65 close to the sRGB white point.
pub fn daylight(kelvin: f32) -> Vec3 {
    let t = kelvin.clamp(4000.0, 25000.0);
    let x = if t <= 7000.0 {
        -4.607e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 99.11 / t + 0.244_063
    } else {
        -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 247.48 / t + 0.237_04
    };
    xy_rgb(x, -3.0 * x * x + 2.87 * x - 0.275)
}

/// Sea-level Ineichen/Perez clear-sky irradiance, Linke turbidity 3, converted
/// approximately to illuminance with 110 lm/W beam and 130 lm/W diffuse.
///
/// These efficacies and atmosphere are assumptions, not measured weather.
/// <https://pvlib-python.readthedocs.io/en/stable/reference/generated/pvlib.clearsky.ineichen.html>
pub fn clear_sky(up: f32, day: f32) -> (f32, f32) {
    if up <= 0.0 {
        return (0.0, 0.0);
    }
    let elevation = up.clamp(0.0, 1.0).asin().to_degrees();
    let mass = 1.0 / (up + 0.505_72 * (elevation + 6.079_95).powf(-1.6364));
    let extra = 1361.0 * (1.0 + 0.033 * (TAU * day / 365.0).cos());
    let global = 0.868 * extra * up * (-0.0387 * mass * 3.0).exp();
    let beam =
        (0.827 * extra * (-0.09 * mass * 2.0).exp()).min(global * (1.0 - (0.1 - 0.2 * (-3.0f32).exp()) / 0.982) / up);
    (beam * 110.0, (global - beam * up).max(0.0) * 130.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whites_preserve_photometric_brightness_and_daylight_energy_is_bounded() {
        for t in (1700..=25000).step_by(137) {
            let c = if t < 4000 { warm_white(t as f32) } else { daylight(t as f32) };
            assert!(c.is_finite() && c.min_element() >= 0.0);
            assert!((c.dot(vec3(0.2126, 0.7152, 0.0722)) - 1.0).abs() < 1e-5);
        }
        assert!((daylight(6504.0) - Vec3::ONE).abs().max_element() < 0.002);
        for day in (0..366).step_by(30) {
            for degree in 0..=90 {
                let (beam, diffuse) = clear_sky((degree as f32).to_radians().sin(), day as f32);
                assert!(beam >= 0.0 && diffuse >= 0.0 && beam < 125_000.0 && diffuse < 30_000.0);
            }
        }
        assert_eq!(clear_sky(-0.1, 0.0), (0.0, 0.0));
    }
}
