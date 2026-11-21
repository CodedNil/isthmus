use crate::app::{AppUpdater, send_update};
use futures_util::StreamExt as _;
use std::{
    collections::HashMap,
    env,
    error::Error,
    fs, io,
    os::unix::net::UnixDatagram,
    path::PathBuf,
    process::{Command, Stdio},
};
use strum_macros::IntoStaticStr;
use tokio::net;
use tracing::warn;
use zbus::{OwnedObjectPath, OwnedVariant, Variant, proxy::CacheProperties};

const LAUNCHER_SOCKET_NAME: &str = "cantus-launcher.sock";

#[derive(Clone, Copy, IntoStaticStr)]
pub enum PowerAction {
    PowerOff,
    Reboot,
}

/// Calls logind, as `systemctl poweroff` does.
pub fn run_power_action(action: PowerAction) {
    let method: &str = action.into();
    tokio::spawn(async move {
        let result = async {
            zbus::Connection::system()
                .await?
                .call_method(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    Some("org.freedesktop.login1.Manager"),
                    method,
                    &(false,),
                )
                .await
        }
        .await;
        if let Err(error) = result {
            warn!(%error, method, "Failed to run held power action");
        }
    });
}

pub fn spawn(command: &[String]) {
    let Some((program, args)) = command.split_first() else {
        return;
    };
    if let Err(error) = Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet", "--property=ExitType=cgroup", "--"])
        .arg(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        warn!(%error, program, "Failed to launch application");
    }
}

pub fn open_url(url: &str) {
    if let Err(error) = try_open_url(url) {
        warn!(%error, %url, "Failed to open URL");
    }
}

pub fn try_open_url(url: &str) -> io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

pub fn start_launcher_listener(updater: &AppUpdater) {
    let path = launcher_socket_path();
    if UnixDatagram::unbound().and_then(|socket| socket.send_to(&[0], &path)).is_ok() {
        warn!(?path, "Another Cantus instance owns the launcher socket");
        return;
    }
    let _stale_socket = fs::remove_file(&path);
    let updater = updater.clone();
    tokio::spawn(async move {
        let socket = match net::UnixDatagram::bind(&path) {
            Ok(socket) => socket,
            Err(error) => {
                warn!(%error, ?path, "Failed to bind launcher toggle socket");
                return;
            }
        };
        let mut buffer = [0u8; 1];
        while socket.recv(&mut buffer).await.is_ok() {
            if !send_update(&updater, |app| app.launcher.toggle()) {
                warn!("Launcher toggle update was discarded");
                break;
            }
        }
    });
}

/// Returns an error when a native Cantus instance cannot be reached.
pub fn trigger_launcher() -> io::Result<()> {
    let path = launcher_socket_path();
    UnixDatagram::unbound()?.send_to(&[0], &path).map_err(|error| {
        io::Error::new(error.kind(), format!("Could not reach Cantus at {}: {error}", path.display()))
    })?;
    Ok(())
}

pub async fn current_location() -> Result<[f32; 2], Box<dyn Error + Send + Sync>> {
    let connection = zbus::Connection::session().await?;
    let proxy = |path: OwnedObjectPath, interface: &'static str| {
        let connection = &connection;
        async move {
            zbus::proxy::Builder::new(connection)
                .destination("org.freedesktop.portal.Desktop")
                .path(path)
                .interface(interface)
                .cache_properties(CacheProperties::No)
                .build()
                .await
        }
    };
    let location: zbus::Proxy<'_> =
        proxy(OwnedObjectPath::try_from("/org/freedesktop/portal/desktop")?, "org.freedesktop.portal.Location").await?;
    let token = "cantus";
    let options = HashMap::from([("session_handle_token", Variant::from(token)), ("accuracy", Variant::from(2u32))]);
    let session: OwnedObjectPath = location.call("CreateSession", &(options,)).await?;
    let mut updates = location.receive_signal("LocationUpdated").await?;

    // Subscribe before Start: the response can arrive before the method returns.
    let name = connection
        .unique_name()
        .ok_or("Session bus has no unique name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = OwnedObjectPath::try_from(format!("/org/freedesktop/portal/desktop/request/{name}/{token}"))?;
    let request = proxy(path.clone(), "org.freedesktop.portal.Request").await?;
    let mut responses = request.receive_signal("Response").await?;
    let options = HashMap::from([("handle_token", Variant::from(token))]);
    let handle: OwnedObjectPath = location.call("Start", &(&session, "", options)).await?;
    if handle != path {
        return Err("Location portal returned an unexpected request handle".into());
    }
    let response = responses.next().await.ok_or("Location portal closed before responding")?;
    let (status, _): (u32, HashMap<String, OwnedVariant>) = response.body().deserialize()?;
    if status != 0 {
        return Err(
            format!("Location request was {}", if status == 1 { "cancelled" } else { "denied or failed" }).into()
        );
    }
    while let Some(update) = updates.next().await {
        let (handle, values): (OwnedObjectPath, HashMap<String, OwnedVariant>) = update.body().deserialize()?;
        if handle == session {
            let latitude = f64::try_from(values.get("Latitude").ok_or("Location has no latitude")?)?;
            let longitude = f64::try_from(values.get("Longitude").ok_or("Location has no longitude")?)?;
            return Ok([latitude as f32, longitude as f32]);
        }
    }
    Err("Location portal closed before providing a position".into())
}

fn launcher_socket_path() -> PathBuf {
    PathBuf::from(env::var_os("XDG_RUNTIME_DIR").expect("Wayland session requires XDG_RUNTIME_DIR"))
        .join(LAUNCHER_SOCKET_NAME)
}
