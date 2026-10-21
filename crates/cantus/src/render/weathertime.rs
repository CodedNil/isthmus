use crate::{
    app::Background,
    config::MAX_WORLD_CLOCKS,
    platform,
    render::{
        GAP, PANEL_RADIUS, PANEL_START, Program, SUPPORT_BLEND, TEXT_BODY, TEXT_COLOR, TEXT_DISPLAY, TEXT_HEADING,
        TEXT_SHADOW, TEXT_SMALL, TEXT_TITLE, UNIT, UiContext,
        sdf::{VISIBLE_ALPHA, deform, glass, hash, refract, sample_deformation, simplex_noise},
        support_row,
    },
    timer::Timer,
};
use arrayvec::{ArrayString, ArrayVec};
use core::f32::consts::PI;
use isthmus::prelude::*;
use isthmus_sdf::prelude::*;
use jiff::{
    Span, Timestamp, Zoned,
    civil::{DateTime, Time},
    tz::{Offset, TimeZone},
};
use std::{array::from_fn, fmt::Write};
use tracing::warn;

/// Number of conditions shown in the hourly forecast row.
const HOURLY_FORECASTS: usize = 6;
/// Hours between adjacent conditions in the hourly forecast row.
const HOURLY_STEP_HOURS: usize = 4;
pub const WIDTH: f32 = UNIT * 77.0;
pub const EXTENSION: f32 = UNIT * 61.0;
const FORECAST_X: f32 = WIDTH + GAP;
/// Depth of the timer readout, sized to hold its text with a little air.
const TIMER_DEPTH: f32 = TEXT_SMALL + 2.0 * UNIT;
/// Space around the timer text inside its support.
const TIMER_PADDING: f32 = 12.0;
const TIMER_COLOR: Vec3 = vec3(1.00, 0.62, 0.60);
/// Exterior reach of the shadow painted behind weather panel text.
const TEXT_SHADOW_REACH: f32 = 1.6;
/// Multiplier deepening the shared text shadow for weather panel text.
const TEXT_SHADOW_STRENGTH: f32 = 1.7;

const WEEKDAY_COUNT: usize = 7;
const TITLE: Vec2 = Vec2::new(WIDTH * 0.5, UNIT * 10.0);
const WEEKDAYS: [&str; WEEKDAY_COUNT] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const ORDINALS: [&str; 10] = ["th", "st", "nd", "rd", "th", "th", "th", "th", "th", "th"];

#[derive(Clone, Copy, Default, ShaderData)]
#[shader_data(unorm16)]
pub struct WeatherCondition {
    pub fog: f32,
    pub cloud: f32,
    pub rain: f32,
    pub snow: f32,
    pub lightning: f32,
    pub hail: f32,
}

#[derive(Clone, Copy, Default, ShaderData)]
pub struct StatusSky {
    pub sun_height: f32,
    pub conditions: WeatherCondition,
}

impl WeatherCondition {
    fn lerp(self, to: Self, amount: f32) -> Self {
        Self {
            fog: self.fog.lerp(to.fog, amount),
            cloud: self.cloud.lerp(to.cloud, amount),
            rain: self.rain.lerp(to.rain, amount),
            snow: self.snow.lerp(to.snow, amount),
            lightning: self.lightning.lerp(to.lightning, amount),
            hail: self.hail.lerp(to.hail, amount),
        }
    }
}

fn grid_cell(index: usize) -> Vec2 {
    let column_width = WIDTH / WEEKDAY_COUNT as f32;
    vec2(
        (index % WEEKDAY_COUNT) as f32 * column_width + column_width * 0.5,
        UNIT * (24.0 + (index / WEEKDAY_COUNT) as f32 * 6.0),
    )
}

/// The pill, its expanded calendar, and the timer readout attached below.
fn weather_panel(pill: Rect, expansion: f32, timer: Vec2) -> impl Sdf {
    let popup_size = vec2(WIDTH + FORECAST_X * expansion, ((EXTENSION - GAP) * expansion).max(0.001));
    let popup_center =
        pill.min + vec2(-FORECAST_X * expansion * 0.5, pill.size().y + GAP * expansion) + popup_size * 0.5;
    let support = Shape::pill(Rect::from_center_size(support_row(pill), timer));
    Shape::pill(pill)
        .smooth_union(
            Shape::rounded_rect(Rect::from_center_size(popup_center, popup_size), PANEL_RADIUS),
            56.0,
            expansion,
        )
        .smooth_union(support, SUPPORT_BLEND, timer.y / TIMER_DEPTH)
}

/// City name from an IANA timezone, such as `Los Angeles` for `America/Los_Angeles`.
fn city(timezone: &str) -> String {
    timezone.rsplit('/').next().unwrap_or(timezone).replace('_', " ")
}

fn forecast_center(height: f32, row: f32) -> f32 {
    UNIT * 14.0 + height * 0.5 + row * (height + GAP)
}

fn reveal_progress(expansion: f32, y: f32) -> f32 {
    let delay = 0.5 + (y / EXTENSION) * 0.18;
    expansion.smoothstep(delay, delay + 0.24)
}

/// Sun phase (0 at sunrise, 1 at sunset) and height (-1 to 1) for the given hour.
fn sun_position(hour: f32, [sunrise, sunset]: [f32; 2]) -> [f32; 2] {
    let daylight = sunset - sunrise;
    if hour >= sunrise && hour <= sunset {
        let phase = (hour - sunrise) / daylight;
        [phase, (phase * PI).sin()]
    } else {
        let night = 24.0 - daylight;
        let phase = if hour < sunrise { (hour + 24.0 - sunset) / night } else { (hour - sunset) / night };
        [if hour >= sunset { 1.0 } else { 0.0 }, -(phase * PI).sin()]
    }
}

/// One layer; `kind` is a literal at every call site, so the tables below fold away.
fn precipitation(background: Vec3, p: Vec2, time: f32, kind: i32, strength: f32) -> Vec3 {
    if strength <= VISIBLE_ALPHA {
        return background;
    }
    let rain = kind == 0;
    let snow = kind == 1;
    let (velocity, cell_size, radius, density, trail) = if rain {
        // Rain intensity changes the number of drops, not their falling speed.
        (vec2(33.0, 220.0), vec2(15.0, 25.0), 0.55, strength.saturate() * 0.9, 14.0)
    } else if snow {
        (vec2(5.0, 14.0), Vec2::splat(20.0), 1.15, 0.7, 0.4)
    } else {
        (vec2(18.0, 85.0), Vec2::splat(23.0), 0.35, 0.3, 1.2)
    };
    let q = p - velocity * time;
    let cell = (q / cell_size).floor();
    let random = hash(cell + kind as f32 * 31.7);
    let center = (cell + 0.15 + random * 0.7) * cell_size;
    let angle = if rain { 0.15 + (random.x - 0.5) * 0.025 } else { 0.08 + random.x * 0.3 };
    let length = trail * if rain { 0.9 + random.y * 0.2 } else { 0.8 + random.y * 0.4 };
    let drift = if rain {
        Vec2::ZERO
    } else {
        vec2(
            (time * (0.55 + random.x * 0.45) + random.y * PI * 2.0).sin() * 1.5,
            (time * (3.5 + random.y * 1.5) + random.x * PI * 2.0).sin() * 8.0,
        )
    };
    // Keep rain streaks inside their cells so they do not clip as they move.
    let center = if rain { (cell + 0.12 + random * vec2(0.65, 0.24)) * cell_size } else { center };
    let local = q - center - drift;
    let distance = Shape::segment(Vec2::ZERO, vec2(angle, 1.0) * length, 0.0).distance_at(local);
    let presence = if rain {
        hash(cell + 19.3).x.smoothstep(1.0 - density, 1.025 - density)
    } else {
        hash(cell + 19.3).x.smoothstep(1.0 - density, 1.0)
    };
    let taper = if rain { 0.3 + local.y.smoothstep(0.0, length) * 0.7 } else { 1.0 };
    let particle = distance.smoothstep(radius + 0.45, radius - 0.15) * presence * taper;
    let color = if rain {
        vec3(0.18, 0.3, 0.43)
    } else if snow {
        Vec3::splat(0.96)
    } else {
        vec3(0.75, 0.86, 0.94)
    };
    background.lerp(
        color,
        (particle
            * if rain { strength.smoothstep(0.0, 0.025) } else { strength }
            * if snow {
                0.92
            } else if rain {
                0.72
            } else {
                0.7
            })
        .saturate(),
    )
}

/// Three cloud octaves.
fn cloud_noise(mut p: Vec2) -> f32 {
    let mut density = simplex_noise(p) * 0.5;
    for octave in 0..2 {
        p = vec2(p.x * 1.6 + p.y * 1.2, p.y * 1.6 - p.x * 1.2);
        let amplitude = if octave == 0 { 0.25 } else { 0.125 };
        density += simplex_noise(p) * amplitude;
    }
    0.5 + density * 0.5
}

/// Daylight, blue-hour and twilight palette weights for a sun height.
pub fn sky_phase(sun_y: f32) -> Vec3 {
    let daylight = sun_y.smoothstep(-0.04, 0.2);
    vec3(
        daylight,
        sun_y.smoothstep(-0.32, -0.08) * (1.0 - daylight),
        sun_y.smoothstep(-0.18, 0.0) * sun_y.smoothstep(0.2, 0.02),
    )
}

pub fn scene(time: f32, cloud_scale: f32, p: Vec2, width: f32, phase: Vec3, weather: WeatherCondition) -> Vec3 {
    let sky_y = p.y / cloud_scale;
    let vertical = sky_y.smoothstep(1.0, 0.0);
    let mut color = vec3(0.006, 0.012, 0.035)
        .lerp(vec3(0.025, 0.04, 0.095), vertical)
        .lerp(vec3(0.08, 0.34, 0.62).lerp(vec3(0.32, 0.67, 0.87), vertical), phase.x)
        .lerp(vec3(0.10, 0.16, 0.30).lerp(vec3(0.22, 0.25, 0.45), vertical), phase.y * 0.8)
        .lerp(vec3(0.78, 0.30, 0.20).lerp(vec3(0.38, 0.22, 0.42), vertical), phase.z * 0.9);

    if phase.x < 0.99 && weather.cloud < 0.99 {
        let star_cell = (p / 18.0).floor();
        let star_center = (star_cell + 0.2 + hash(star_cell) * 0.6) * 18.0;
        let stars = p.distance(star_center).smoothstep(1.0, 0.4)
            * hash(star_cell + 31.7).x.smoothstep(0.75, 1.0)
            * (1.0 - phase.x);
        color += Vec3::splat(stars * (1.0 - weather.cloud) * (0.3 + vertical * 0.7));
    }

    if weather.cloud > VISIBLE_ALPHA {
        let mass = cloud_noise(p / cloud_scale * 0.14 + vec2(time * 0.012, 6.1));
        let billows = cloud_noise(p / cloud_scale * 0.8 + vec2(time * 0.018, -3.7));
        let threshold = 0.68 - weather.cloud * 0.4;
        let cloud_shape = (mass + (billows - 0.5) * 0.18).smoothstep(threshold - 0.04, threshold + 0.2);
        // Broad, low-contrast lighting keeps the billows soft instead of mottled.
        let cloud_light = (billows * 0.35 + mass * 0.65).smoothstep(0.28, 0.82);
        let cloud_color = vec3(0.20, 0.24, 0.31)
            .lerp(vec3(0.30, 0.34, 0.41), cloud_light)
            .lerp(vec3(0.74, 0.79, 0.84).lerp(vec3(0.88, 0.91, 0.94), cloud_light), phase.x)
            .lerp(vec3(0.58, 0.43, 0.46).lerp(vec3(0.73, 0.58, 0.57), cloud_light), phase.z * 0.45);
        let cloud_coverage = weather.rain.smoothstep(0.0, 0.025);
        let cloud_amount = (cloud_shape * (0.75 + weather.cloud * 0.2)).lerp(0.98, cloud_coverage);
        color = color.lerp(cloud_color, cloud_amount);
    }

    color = color.lerp(vec3(0.1, 0.17, 0.25), weather.rain * 0.2);
    color = precipitation(color, p, time, 0, weather.rain);
    color = precipitation(color, p, time, 1, weather.snow);
    color = precipitation(color, p, time, 2, weather.hail);

    let flash = (time * 2.7).sin().smoothstep(0.92, 1.0) * weather.lightning;
    color = color.lerp(vec3(0.65, 0.74, 0.96), flash * 0.55);

    if weather.fog > VISIBLE_ALPHA {
        let fog = 0.5 + simplex_noise(vec2(p.x / width * 0.9 + time * 0.008, sky_y * 0.32 + 12.0)) * 0.5;
        color = color.lerp(vec3(0.63, 0.69, 0.73), weather.fog * (0.58 + fog.smoothstep(0.35, 0.7) * 0.18));
    }
    color
}

#[derive(Default)]
struct ForecastItem {
    text: [String; 2],
    hover_text: String,
    conditions: WeatherCondition,
    hour: f32,
}

struct WorldClock {
    label: String,
    timezone: TimeZone,
    weather: String,
}

#[derive(Default)]
pub struct WeatherPanel {
    expansion: f32,
    sun_hours: [f32; 2] = [6.0, 18.0],
    temperature: String,
    utc_offset: Option<Offset>,
    details: String,
    hourly: [ForecastItem; HOURLY_FORECASTS],
    daily: [ForecastItem; 5],
    timezones: ArrayVec<WorldClock, MAX_WORLD_CLOCKS>,
    month_offset: i32,
    month_hover: f32,
    arrow_hover: [f32; 2],
    /// Animated presence of the timer readout.
    timer_presence: f32,
}

/// Geometry shared by the panel and its text during one draw.
struct WeatherLayout {
    pill: Rect,
    expansion: f32,
    timer_support: Vec2,
}

mod monitor {
    use super::{ForecastItem, HOURLY_STEP_HOURS, ORDINALS, WeatherCondition, WeatherPanel};
    use crate::{
        app::{Background, fetch_json, send_update},
        platform,
    };
    use futures_util::future::join_all;
    use jiff::{
        civil::DateTime,
        tz::{Offset, TimeZone},
    };
    use reqwest::Client;
    use serde::Deserialize;
    use std::{array::from_fn, iter::once, time::Duration};
    use tracing::warn;

    const WEATHER_FIELDS: &str = "temperature_2m,weather_code";

    #[derive(Deserialize)]
    pub(super) struct Forecast {
        utc_offset_seconds: i32,
        current: Current,
        hourly: Hourly,
        daily: Daily,
    }

    #[derive(Deserialize)]
    struct Current {
        weather_code: u8,
        temperature_2m: f32,
        relative_humidity_2m: u8,
        wind_speed_10m: f32,
    }

    #[derive(Deserialize)]
    struct Hourly {
        weather_code: [u8; 24],
        time: [DateTime; 24],
        temperature_2m: [f32; 24],
    }

    #[derive(Deserialize)]
    struct Daily {
        weather_code: [u8; 6],
        temperature_2m_max: [f32; 6],
        temperature_2m_min: [f32; 6],
        sunrise: [DateTime; 6],
        sunset: [DateTime; 6],
    }

    #[derive(Deserialize)]
    struct SearchResults {
        #[serde(default)]
        results: Vec<Place>,
    }

    #[derive(Deserialize)]
    struct Place {
        latitude: f32,
        longitude: f32,
        timezone: String,
    }

    macro_rules! weather_codes {
($($code:literal => $name:literal { $($field:ident: $value:literal),* };)*) => {
    pub(super) fn weather(code: u8) -> (&'static str, WeatherCondition) {
        match code {
            $($code => ($name, WeatherCondition {
                $($field: $value,)*
                ..Default::default()
            }),)*
            _ => ("Unknown weather", Default::default()),
        }
    }
};
}

    // Visual defaults inferred from the reported phenomenon, not measured cloud
    // cover/visibility. Showers retain breaks; continuous precipitation is layered
    // beneath stratus. Reduced visibility in heavy precipitation is a veil rather
    // than an assertion that meteorological fog has also been observed.
    weather_codes! {
        0 => "Clear" { };
        1 => "Mainly Clear" { cloud: 0.25 };
        2 => "Partly Cloudy" { cloud: 0.55 };
        3 => "Overcast" { cloud: 1.0 };
        45 => "Fog" { fog: 0.6, cloud: 0.45 };
        48 => "Rime Fog" { fog: 0.75, cloud: 0.5 };
        51 => "Light Drizzle" { rain: 0.15, cloud: 0.85, fog: 0.08 };
        53 => "Moderate Drizzle" { rain: 0.3, cloud: 0.95, fog: 0.14 };
        55 => "Dense Drizzle" { rain: 0.45, cloud: 1.0, fog: 0.22 };
        56 => "Light Freezing Drizzle" { rain: 0.2, cloud: 0.9, fog: 0.12 };
        57 => "Dense Freezing Drizzle" { rain: 0.4, cloud: 1.0, fog: 0.24 };
        61 => "Light Rain" { rain: 0.3, cloud: 0.8, fog: 0.04 };
        63 => "Moderate Rain" { rain: 0.6, cloud: 0.95, fog: 0.1 };
        65 => "Heavy Rain" { rain: 1.0, cloud: 1.0, fog: 0.25 };
        66 => "Light Freezing Rain" { rain: 0.35, cloud: 0.9, fog: 0.08 };
        67 => "Heavy Freezing Rain" { rain: 0.9, cloud: 1.0, fog: 0.24 };
        71 => "Light Snow" { snow: 0.3, cloud: 0.85, fog: 0.08 };
        73 => "Moderate Snow" { snow: 0.6, cloud: 0.95, fog: 0.2 };
        75 => "Heavy Snow" { snow: 1.0, cloud: 1.0, fog: 0.45 };
        77 => "Snow Grains" { snow: 0.25, cloud: 0.9, fog: 0.16 };
        80 => "Light Rain Showers" { rain: 0.35, cloud: 0.6, fog: 0.02 };
        81 => "Moderate Rain Showers" { rain: 0.65, cloud: 0.75, fog: 0.08 };
        82 => "Violent Rain Showers" { rain: 1.0, cloud: 0.95, fog: 0.22 };
        85 => "Light Snow Showers" { snow: 0.35, cloud: 0.65, fog: 0.06 };
        86 => "Heavy Snow Showers" { snow: 0.9, cloud: 0.9, fog: 0.32 };
        95 => "Thunderstorm" { rain: 0.7, cloud: 1.0, fog: 0.12, lightning: 1.0 };
        96 => "Thunderstorm Light Hail" { rain: 0.75, cloud: 1.0, fog: 0.16, lightning: 1.0, hail: 0.6 };
        99 => "Thunderstorm Heavy Hail" { rain: 0.85, cloud: 1.0, fog: 0.3, lightning: 1.0, hail: 1.0 };
    }

    pub(super) async fn run(timezones: Vec<String>, background: Background) {
        let http = &background.http;
        let timezones: Vec<_> =
            once(TimeZone::system().iana_name().map(str::to_owned)).chain(timezones.into_iter().map(Some)).collect();
        let mut locations = vec![None; timezones.len()];
        locations[0] = tokio::select! {
            result = platform::current_location() => result
                .inspect_err(|error| warn!(%error, "Could not determine local weather location; using timezone"))
                .ok(),
            () = platform::sleep(Duration::from_secs(10)) => None,
        };
        loop {
            join_all(timezones.iter().zip(&mut locations).filter_map(|(timezone, slot)| {
                let timezone = timezone.as_ref().filter(|_| slot.is_none())?;
                Some(async move {
                    *slot = geocode(http, timezone)
                        .await
                        .inspect_err(|error| warn!(%error, timezone, "Failed to locate timezone"))
                        .ok();
                })
            }))
            .await;
            let ready = locations
                .iter()
                .enumerate()
                .filter_map(|(index, &location)| location.map(|point| (index, point)))
                .collect::<Vec<_>>();
            let mut retry = ready.len() != locations.len();
            let forecasts: Vec<_> = match fetch(http, &ready).await {
                Ok(results) => ready.into_iter().zip(results).map(|((index, _), forecast)| (index, forecast)).collect(),
                Err(error) => {
                    retry = true;
                    warn!(%error, "Failed to refresh weather");
                    Vec::new()
                }
            };
            if !forecasts.is_empty()
                && !send_update(&background.updater, move |app| {
                    if let Some(weather) = &mut app.bar.weather {
                        for (index, forecast) in forecasts {
                            apply_forecast(weather, index, &forecast);
                        }
                    }
                })
            {
                break;
            }
            let interval = if retry { Duration::from_secs(30) } else { Duration::from_mins(15) };
            platform::sleep(interval).await;
        }
    }

    fn apply_forecast(weather_model: &mut WeatherPanel, index: usize, forecast: &Forecast) {
        // Index 0 is the local forecast; the rest fill in each configured world clock.
        if let Some(timezone) = index.checked_sub(1).and_then(|index| weather_model.timezones.get_mut(index)) {
            timezone.weather = format!(
                "{} · {:.0}°/{:.0}°",
                weather(forecast.daily.weather_code[0]).0,
                forecast.daily.temperature_2m_max[0],
                forecast.daily.temperature_2m_min[0],
            );
            return;
        }
        if index != 0 {
            return;
        }
        weather_model.utc_offset = Offset::from_seconds(forecast.utc_offset_seconds).ok();
        weather_model.temperature = format!("{:.1}°C", forecast.current.temperature_2m);
        weather_model.sun_hours = [forecast.daily.sunrise[0], forecast.daily.sunset[0]].map(WeatherPanel::hour_of_day);
        weather_model.hourly = from_fn(|index| {
            let source = index * HOURLY_STEP_HOURS;
            let (description, conditions) = weather(forecast.hourly.weather_code[source]);
            let (time, degrees) = (
                forecast.hourly.time[source].strftime("%H:%M").to_string(),
                format!("{:.0}°", forecast.hourly.temperature_2m[source]),
            );
            ForecastItem {
                hover_text: format!("{time} {description} {degrees}"),
                text: [time, degrees],
                conditions,
                hour: WeatherPanel::hour_of_day(forecast.hourly.time[source]),
            }
        });
        weather_model.hourly[0].conditions = weather(forecast.current.weather_code).1;
        weather_model.daily = from_fn(|index| {
            let day = index + 1;
            let date = forecast.daily.sunrise[day];
            let number = date.day();
            let suffix = match number % 100 {
                11..=13 => "th",
                _ => ORDINALS[(number % 10) as usize],
            };
            let (description, conditions) = weather(forecast.daily.weather_code[day]);
            let range = format!(
                "{:.0}°/{:.0}°",
                forecast.daily.temperature_2m_max[day], forecast.daily.temperature_2m_min[day]
            );
            ForecastItem {
                hover_text: format!("{}{suffix} {description} {range}", date.strftime("%A %-d")),
                text: [date.strftime("%a").to_string(), range],
                conditions,
                hour: -1.0,
            }
        });
        weather_model.details = format!(
            "{} · Humidity {}% · Wind {:.0} km/h",
            weather(forecast.current.weather_code).0,
            forecast.current.relative_humidity_2m,
            forecast.current.wind_speed_10m
        );
    }

    async fn geocode(http: &Client, timezone: &str) -> Result<[f32; 2], String> {
        let city = super::city(timezone);
        let query: String = form_urlencoded::byte_serialize(city.as_bytes()).collect();
        let results: SearchResults =
            fetch_json(http.get(format!("https://geocoding-api.open-meteo.com/v1/search?name={query}&count=10")))
                .await
                .map_err(|error| error.to_string())?;
        let place = results
            .results
            .iter()
            .find(|place| place.timezone == timezone)
            .or_else(|| results.results.first())
            .ok_or_else(|| format!("no place found for {city}"))?;
        Ok([place.latitude, place.longitude])
    }

    async fn fetch(http: &Client, locations: &[(usize, [f32; 2])]) -> Result<Vec<Forecast>, String> {
        if locations.is_empty() {
            return Ok(Vec::new());
        }
        let latitude = locations.iter().map(|(_, [latitude, _])| latitude.to_string()).collect::<Vec<_>>().join(",");
        let longitude = locations.iter().map(|(_, [_, longitude])| longitude.to_string()).collect::<Vec<_>>().join(",");
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={latitude}&longitude={longitude}&current={WEATHER_FIELDS},relative_humidity_2m,wind_speed_10m&hourly={WEATHER_FIELDS}&forecast_hours=24&daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset&temperature_unit=celsius&timezone=auto&forecast_days=6"
        );
        if locations.len() == 1 {
            fetch_json(http.get(url)).await.map(|forecast| vec![forecast])
        } else {
            fetch_json(http.get(url)).await
        }
        .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::weather;

        #[test]
        fn precipitation_has_clouds_and_heavier_weather_reduces_visibility() {
            for code in [51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82, 85, 86, 95, 96, 99] {
                let (_, scene) = weather(code);
                assert!(scene.cloud >= 0.5, "precipitation without clouds: {code}");
            }
            for codes in [[51, 53, 55], [61, 63, 65], [71, 73, 75], [80, 81, 82]] {
                assert!(codes.windows(2).all(|pair| weather(pair[0]).1.fog < weather(pair[1]).1.fog));
            }
            assert!(weather(80).1.cloud < weather(61).1.cloud);
            assert!(weather(85).1.cloud < weather(71).1.cloud);
        }

        #[test]
        fn clear_fog_and_thunder_remain_distinct() {
            let clear = weather(0).1;
            assert!(clear.cloud + clear.fog + clear.rain + clear.snow <= f32::EPSILON);
            for code in [45, 48] {
                let fog = weather(code).1;
                assert!(fog.fog > 0.5 && fog.rain + fog.snow <= f32::EPSILON);
            }
            for code in [95, 96, 99] {
                let storm = weather(code).1;
                assert!(storm.cloud > 0.95 && storm.lightning > 0.5);
            }
        }
    }
}
impl WeatherLayout {
    fn paint_text(&self, context: &mut UiContext, line: Text, color: Vec4, clip: bool) {
        shader!(
            context
                .frame
                .upload({
                    let pill: Rect = self.pill;
                    let expansion: f32 = self.expansion;
                    let timer_support: Vec2 = self.timer_support;
                    let line: Text = line.outlined(TEXT_SHADOW_REACH);
                    let color: Vec4;
                    let clip: bool;
                })
                .primitive(|frame| refract(weather_panel(pill, expansion, timer_support), frame, line))
                .fragment(|_, surface| {
                    surface.paint(color.with_w(1.0), TEXT_SHADOW.with_w(TEXT_SHADOW.w * TEXT_SHADOW_STRENGTH))
                        * vec4(1.0, 1.0, 1.0, color.w * if clip { surface.parent_fill } else { 1.0 })
                })
        );
    }

    /// Paints a centred label, clipped to the panel's fill.
    fn label(&self, context: &mut UiContext, text: &str, size: f32, weight: f32, center: Vec2, color: Vec4) {
        let line = context.frame.resources.line(text, size, weight).centered(center);
        self.paint_text(context, line, color, true);
    }

    fn pair(&self, context: &mut UiContext, labels: [&str; 2], size: f32, center: Vec2, spacing: f32, alpha: f32) {
        for (index, label) in labels.into_iter().enumerate() {
            let center = center + vec2(0.0, (index as f32 * 2.0 - 1.0) * spacing);
            let color = TEXT_COLOR.extend(alpha * if index == 0 { 1.0 } else { 0.75 });
            self.label(context, label, size, 700.0, center, color);
        }
    }
}

impl WeatherPanel {
    pub(crate) fn new(timezones: &[String], background: &Background) -> Self {
        let mut forecast_timezones = Vec::with_capacity(timezones.len());
        let timezones: ArrayVec<_, MAX_WORLD_CLOCKS> = timezones
            .iter()
            .filter_map(|name| {
                let timezone = TimeZone::get(name)
                    .inspect_err(|error| warn!(timezone = name, %error, "Ignoring invalid timezone"))
                    .ok()?;
                forecast_timezones.push(name.clone());
                Some(WorldClock { label: city(name), timezone, weather: String::from("Weather unavailable") })
            })
            .collect();
        platform::spawn_task(monitor::run(forecast_timezones, background.clone()));
        Self { timezones, details: "Weather unavailable".into(), ..Default::default() }
    }

    pub fn show(&mut self, context: &mut UiContext, status_width: f32, timer: &Timer) -> StatusSky {
        let height = context.config.height;
        let x = context.frame.screen_size.x - WIDTH - GAP - status_width;
        let pill = Rect::new(vec2(x, PANEL_START), vec2(x + WIDTH, PANEL_START + height));
        let readout = timer.readout();
        let expired = timer.expired();
        self.timer_presence = self
            .timer_presence
            .move_towards(f32::from(readout.is_some()), context.frame.delta_time.min(1.0 / 30.0) * 6.0);
        let presence = self.timer_presence.smoothstep(0.0, 1.0);
        // Sized from a fixed run so the pulsing text never resizes its support.
        let text = readout.as_deref().unwrap_or_default();
        let width = context.frame.resources.shape(text, TEXT_SMALL, 900.0).text.width;
        let timer_support = vec2((width + TIMER_PADDING * 2.0) * presence, TIMER_DEPTH * presence);
        let hovered =
            context.interaction.pointer_in(weather_panel(pill, self.expansion.smoothstep(0.0, 1.0), timer_support));
        self.expansion =
            self.expansion.move_towards(f32::from(hovered), context.frame.delta_time.min(1.0 / 30.0) * 3.0);
        let (weather_label, hour) = self.collapsed_label();
        let current = self.hourly[0].conditions;
        let sun = Vec2::from(sun_position(hour, self.sun_hours));
        let expansion = self.expansion.smoothstep(0.0, 1.0);
        let layout = WeatherLayout { pill, expansion, timer_support };
        let panel = weather_panel(pill, expansion, timer_support);
        context.interaction.input_region(pill);
        if expansion > 0.0 {
            context.interaction.input_region(panel.bounds(0.0));
        }
        shader!(
            context
                .frame
                .upload({
                    let current: WeatherCondition;
                    let pill: Rect;
                    let expansion: f32;
                    let timer_support: Vec2;
                    let phase: Vec3 = sky_phase(sun.y);
                    let sun_center: Vec2 =
                        vec2(16.0 + sun.x * (pill.size().x - 32.0), pill.size().y * (0.72 - sun.y.saturate() * 0.45));
                    let sun_color: Vec4 =
                        vec3(0.96, 0.98, 1.0).lerp(vec3(0.98, 0.74, 0.66), sun.y.smoothstep(0.55, 0.02)).extend(
                            sun.y.smoothstep(-0.02, 0.04) * (1.0 - current.cloud * 0.95) * (1.0 - current.fog * 0.85),
                        );
                })
                .primitive(|frame| deform(weather_panel(pill, expansion, timer_support), frame))
                .fragment(|frame, surface| {
                    let body_local = surface.pixel - pill.min;
                    let mut color =
                        scene(frame.time, frame.globals.bar_height, body_local, pill.size().x, phase, current);
                    let distance = body_local.distance(sun_center);
                    color = color.lerp(
                        sun_color.truncate(),
                        (distance.smoothstep(62.0, 4.0) * 0.24 + distance.smoothstep(11.0, 1.0) * 0.7) * sun_color.w,
                    );
                    glass(surface, color)
                })
        );
        let line = context
            .frame
            .resources
            .line(&weather_label, TEXT_DISPLAY, 600.0)
            .centered(vec2(x + WIDTH * 0.5, PANEL_START + height * 0.46));
        layout.paint_text(context, line, TEXT_COLOR.extend(1.0), false);
        if presence > 0.0 {
            // Quantized so the shaped run is reused rather than rebuilt every frame.
            let pulse = ((context.frame.time * 5.0).sin() * 0.5 + 0.5).powi(2);
            let size = TEXT_SMALL + if expired { (pulse * 2.0).round() * 0.5 } else { 0.0 };
            let alpha = if expired { 0.4 + 0.6 * pulse } else { 1.0 };
            layout.label(context, text, size, 900.0, support_row(pill), TIMER_COLOR.extend(alpha));
        }
        self.show_calendar(context, &layout);
        StatusSky { sun_height: sun.y, conditions: current }
    }

    fn collapsed_label(&self) -> (ArrayString<64>, f32) {
        let time = Zoned::now();
        let hour = self.utc_offset.map_or_else(
            || Self::hour_of_day(time.datetime()),
            |offset| Self::hour_of_day(offset.to_datetime(time.timestamp())),
        );
        let clock = time.strftime("%a %d %b  %H:%M:%S");
        let mut label = ArrayString::new();
        if self.temperature.is_empty() {
            write!(label, "{clock}").unwrap();
        } else {
            write!(label, "{}   {clock}", self.temperature).unwrap();
        }
        (label, hour)
    }

    fn show_calendar(&mut self, context: &mut UiContext, layout: &WeatherLayout) {
        let pill = layout.pill;
        if self.expansion <= 0.0 {
            return;
        }
        let origin = vec2(pill.min.x - FORECAST_X * 0.5, pill.max.y);
        let height = pill.size().y;
        let expansion = layout.expansion;
        let reveal = reveal_progress(expansion, TITLE.y);

        let title = context.interaction.interact(
            "calendar-month",
            Shape::pill(Rect::from_center_size(origin + TITLE, vec2(UNIT * 52.0, UNIT * 8.0))),
        );
        if title.clicked {
            self.month_offset = 0;
        }
        self.month_hover = self.month_hover.move_towards(f32::from(title.hovered), context.frame.delta_time / 0.12);

        let today = Zoned::now().date();
        let month = today.first_of_month().saturating_add(Span::new().months(self.month_offset));
        let (size, weight) = (TEXT_HEADING * (1.0 + self.month_hover * 0.2), 750.0 + self.month_hover * 150.0);
        let title = month.strftime("%B %Y").to_string();
        layout.label(context, &title, size, weight, origin + TITLE, TEXT_COLOR.extend(reveal));

        for (index, (side, glyph)) in [(-1.0f32, "<"), (1.0, ">")].into_iter().enumerate() {
            let position = Vec2::new(
                WIDTH * 0.5 + side * (WIDTH * 0.5 - UNIT * 7.0) * reveal,
                TITLE.y - (1.0 - reveal) * UNIT * 3.0,
            );
            let response =
                context.interaction.interact(("calendar-arrow", index), Shape::circle(origin + position, UNIT * 5.0));
            if response.clicked {
                self.month_offset = (self.month_offset + side as i32).clamp(-1200, 1200);
            }
            let hover = &mut self.arrow_hover[index];
            *hover = hover.move_towards(f32::from(response.hovered), context.frame.delta_time / 0.12);
            let (size, weight) = (TEXT_HEADING * (1.0 + *hover * 0.35), 750.0 + *hover * 150.0);
            let color = TEXT_COLOR.extend(reveal_progress(expansion, position.y));
            layout.label(context, glyph, size, weight, origin + position, color);
        }

        let mut hovered_detail = None;
        for (row, items) in [&self.hourly[..], &self.daily[..]].into_iter().enumerate() {
            let size = Vec2::new(WIDTH - GAP * 2.0, height);
            let row_origin = Vec2::new(FORECAST_X + WIDTH * 0.5, forecast_center(height, row as f32)) - size * 0.5;
            let step = size.x / items.len() as f32;
            let forecast_pill = Rect::new(origin + row_origin, origin + row_origin + size);
            let alpha = reveal_progress(expansion, row_origin.y + size.y * 0.5);
            if alpha > 0.0
                && context.interaction.pointer_in(Shape::pill(forecast_pill).intersection(weather_panel(
                    pill,
                    expansion,
                    layout.timer_support,
                )))
            {
                let column = ((context.interaction.mouse_pos().x - origin.x - row_origin.x) / step) as usize;
                hovered_detail = Some(items[column.min(items.len() - 1)].hover_text.as_str());
            }

            shader!(
                context
                    .frame
                    .upload({
                        let conditions: [WeatherCondition; HOURLY_FORECASTS] =
                            from_fn(|index| items[index.min(items.len() - 1)].conditions);
                        let count: u32 = items.len() as u32;
                        let start_hour: f32 = items[0].hour;
                        let sun_hours: [f32; 2] = self.sun_hours;
                        let alpha: f32;
                        let forecast_pill: Rect;
                        let pill: Rect;
                        let expansion: f32;
                        let timer_support: Vec2 = layout.timer_support;
                    })
                    .primitive(|frame| deform(Shape::pill(forecast_pill), frame))
                    .fragment(|frame, surface| {
                        let panel =
                            sample_deformation(weather_panel(pill, expansion, timer_support), frame, surface.pixel);
                        let position =
                            (forecast_pill.uv(surface.pixel).x * count as f32 - 0.5).clamp(0.0, count as f32 - 1.0);
                        let index = position.floor() as usize;
                        let conditions = conditions[index].lerp(
                            conditions[(index + 1).min(count as usize - 1)],
                            position.fract().smoothstep(0.0, 1.0),
                        );
                        let hour = if start_hour < 0.0 {
                            12.0
                        } else {
                            (start_hour + position * HOURLY_STEP_HOURS as f32) % 24.0
                        };
                        let pane_center = forecast_pill.center() - pill.min;
                        let p = pane_center + (surface.pixel - forecast_pill.center()) * 2.0;
                        let color = scene(
                            frame.time,
                            frame.globals.bar_height,
                            p,
                            forecast_pill.size().x * 2.0,
                            sky_phase(sun_position(hour, sun_hours)[1]),
                            conditions,
                        );
                        glass(surface, color) * vec4(1.0, 1.0, 1.0, alpha * panel.sdf.fill())
                    })
            );
            for (column, forecast) in items.iter().enumerate() {
                let center = origin + row_origin + vec2(step * (column as f32 + 0.5), size.y * 0.5);
                layout.pair(context, forecast.text.each_ref().map(String::as_str), TEXT_BODY, center, GAP, alpha);
            }
        }
        let detail = hovered_detail.unwrap_or(&self.details);
        let center = origin + vec2(FORECAST_X + WIDTH * 0.5, TITLE.y);
        layout.label(context, detail, TEXT_BODY, 700.0, center, TEXT_COLOR.extend(reveal));

        for (column, weekday) in WEEKDAYS.iter().enumerate() {
            let position = Vec2::new(grid_cell(column).x, UNIT * 17.0);
            let color = TEXT_COLOR.extend(reveal_progress(expansion, position.y) * 0.75);
            layout.label(context, weekday, TEXT_BODY, 700.0, origin + position, color);
        }

        let grid_start = month.saturating_sub(Span::new().days(month.weekday().to_monday_zero_offset()));
        for index in 0..WEEKDAY_COUNT * 6 {
            let date = grid_start.saturating_add(Span::new().days(index as i64));
            let mut label = ArrayString::<2>::new();
            write!(label, "{}", date.day()).unwrap();
            let is_today = date == today;
            let color = if is_today { vec3(1.0, 0.68, 0.68) } else { TEXT_COLOR };
            let alpha =
                reveal_progress(expansion, grid_cell(index).y) * if date.month() == month.month() { 1.0 } else { 0.32 };
            let weight = if is_today { 900.0 } else { 700.0 };
            layout.label(context, &label, TEXT_TITLE, weight, origin + grid_cell(index), color.extend(alpha));
        }

        let now = Timestamp::now();
        for (index, timezone) in self.timezones.iter().enumerate() {
            let local = now.to_zoned(timezone.timezone.clone());
            let mut clock = ArrayString::<64>::new();
            write!(clock, "{} · {}", timezone.label, local.strftime("%H:%M")).unwrap();
            let center = vec2(
                FORECAST_X + WIDTH * 0.5,
                forecast_center(height, 1.0) + height * 0.5 + UNIT * (3.5 + index as f32 * 7.0),
            );
            let alpha = reveal_progress(expansion, center.y);
            layout.pair(context, [&clock, &timezone.weather], TEXT_SMALL, origin + center, GAP * 0.7, alpha);
        }
    }

    fn hour_of_day(time: DateTime) -> f32 {
        time.time().duration_since(Time::midnight()).as_secs_f32() / 3600.0
    }
}
