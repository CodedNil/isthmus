use arrayvec::ArrayVec;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::io;
use std::{env, fs, path::PathBuf};
use tracing::warn;

pub const MAX_PLAYLIST_TARGETS: usize = 8;
pub const MAX_WORLD_CLOCKS: usize = 3;

#[derive(Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "independent user-facing toggles")]
#[cfg_attr(target_os = "linux", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct Config {
    /// The monitor to display on.
    pub monitor: Option<String> = None,
    /// The layer the app should be on.
    pub layer: Layer = Layer::Top,
    /// The corner/edge the application should anchor to.
    pub layer_anchor: LayerAnchor = LayerAnchor::Top,
    /// The height of the bar in logical pixels.
    pub height: f32 = 50.0,

    /// How many minutes in the future to display in the timeline.
    pub timeline_future_minutes: f32 = 12.0,
    /// How many minutes before the current time to display in the timeline.
    pub timeline_past_minutes: f32 = 1.5,
    /// The width in logical pixels on the left where previous tracks are displayed.
    pub history_width: f32 = 100.0,
    /// Favourite playlists to display as buttons.
    pub playlists: ArrayVec<String, MAX_PLAYLIST_TARGETS> = ArrayVec::new_const(),
    /// Whether star ratings should be enabled.
    pub ratings_enabled: bool = false,
    /// Whether to show synchronized lyrics.
    pub lyrics_enabled: bool = true,

    /// Whether to show the weather and calendar module.
    pub weathertime_enabled: bool = true,
    /// Up to three IANA timezones shown with approximate city weather.
    pub timezones: ArrayVec<String, MAX_WORLD_CLOCKS>,

    /// Whether to show the system status module.
    pub status_enabled: bool = true,

    /// Web search providers; the first is the unprefixed fallback.
    pub search_providers: Vec<SearchProvider>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timezones: ["Europe/London", "America/Los_Angeles", "Australia/Sydney"].map(String::from).into(),
            search_providers: vec![SearchProvider {
                name: "DuckDuckGo".into(),
                url: "https://duckduckgo.com/?q={searchTerms}".into(),
                icon: "https://duckduckgo.com/assets/logo_header.v109.svg".into(),
                alias: "!ddg".into(),
            }],
            ..
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(target_os = "linux", derive(schemars::JsonSchema))]
pub struct SearchProvider {
    /// Display name, such as `DuckDuckGo` or `GitHub`.
    pub name: String,
    /// URL containing one `{searchTerms}` placeholder.
    pub url: String,
    /// URL of the provider icon displayed by the launcher.
    pub icon: String,
    /// Prefix which selects this provider, such as `!gh`.
    pub alias: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(target_os = "linux", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(target_os = "linux", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum LayerAnchor {
    Top,
    Bottom,
}

pub fn directory() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
        .join("cantus")
}

pub fn load() -> Config {
    let path = directory().join("cantus.toml");
    fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|contents| toml::from_str(&contents).map_err(|error| error.to_string()))
        .unwrap_or_else(|error| {
            warn!("Falling back to default config for {path:?}: {error}");
            Config::default()
        })
}

#[cfg(target_os = "linux")]
/// # Errors
/// Returns errors writing `generated-options.nix` in the current directory.
/// # Panics
/// Panics if the configuration schema contains unsupported types or missing defaults.
pub fn generate_nix_options() -> io::Result<()> {
    use schemars::generate::SchemaSettings;
    use serde_json::Value;
    use std::fmt::Write as _;

    fn nix_string(value: &str) -> String {
        serde_json::to_string(value).unwrap().replace("${", "\\${")
    }

    fn nix_type(schema: &Value) -> String {
        if let Some(values) = schema["enum"].as_array() {
            let values = values
                .iter()
                .map(|value| format!("      {}", nix_string(value.as_str().unwrap())))
                .collect::<Vec<_>>()
                .join("\n");
            return format!("lib.types.enum [\n{values}\n    ]");
        }
        match schema["type"].as_str() {
            None => {
                let types = schema["type"].as_array().expect("config type must be explicit");
                assert_eq!(types.len(), 2, "only nullable unions are supported");
                assert!(types.iter().any(|kind| kind == "null"), "union must contain null");
                let mut inner = schema.clone();
                inner["type"] = types.iter().find(|kind| *kind != "null").unwrap().clone();
                format!("lib.types.nullOr ({})", nix_type(&inner))
            }
            Some("string") => "lib.types.str".into(),
            Some("number") => "lib.types.number".into(),
            Some("boolean") => "lib.types.bool".into(),
            Some("array") => {
                let list = format!("lib.types.listOf ({})", nix_type(&schema["items"]));
                if let Some(max) = schema["maxItems"].as_u64() {
                    let operator = if schema["minItems"] == schema["maxItems"] { "==" } else { "<=" };
                    format!("lib.types.addCheck ({list}) (xs: builtins.length xs {operator} {max})")
                } else {
                    list
                }
            }
            Some("object") => "lib.types.attrs".into(),
            Some(kind) => panic!("unsupported config type: {kind}"),
        }
    }

    let schema = SchemaSettings::default()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<Config>();
    let mut output = String::from("# Generated from Cantus configuration; do not edit.\n\n{ lib }: {\n");
    for (name, property) in schema.as_value()["properties"].as_object().unwrap() {
        writeln!(
            output,
            "  {name} = lib.mkOption {{\n    type = {};\n    default = builtins.fromJSON {};\n    description = {};\n  }};",
            nix_type(property),
            nix_string(&property["default"].to_string()),
            nix_string(property["description"].as_str().unwrap()),
        ).unwrap();
    }
    output.push_str("}\n");
    let path = "generated-options.nix";
    if !fs::read_to_string(path).is_ok_and(|contents| contents == output) {
        fs::write(path, output)?;
    }
    Ok(())
}
