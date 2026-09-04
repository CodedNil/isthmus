use crate::app::Background;
use reqwest::Client;
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf};

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_arch = "wasm32")]
pub mod web;

/// One launchable application entry exposed to the launcher.
pub struct DesktopApp {
    pub name: String,
    pub exec: Vec<String>,
    pub comment: String,
    pub icon_path: Option<PathBuf>,
    pub action: Option<(String, Vec<String>)>,
    pub icon: Option<isthmus::Image>,
}

/// Platform-specific services used by the shared Cantus UI.
pub struct Platform;

impl Platform {
    pub fn start_exchange_rates(background: &Background, http: Client, update: fn(HashMap<String, f64>)) {
        background.spawn(async move {
            if let Ok(response) = http
                .get("https://open.er-api.com/v6/latest/USD")
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                && let Ok(body) = response.json::<Value>().await
                && let Some(rates) = body.get("rates").and_then(Value::as_object)
            {
                update(rates.iter().filter_map(|(currency, rate)| Some((currency.clone(), rate.as_f64()?))).collect());
            }
        });
    }
}
