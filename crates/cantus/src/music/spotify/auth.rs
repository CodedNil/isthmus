use super::{MusicResult, REQUEST_TIMEOUT, session::Session};
use crate::{config, platform};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    error::Error as StdError,
    fmt::{Display, Formatter, Result as FmtResult},
    fs, io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{Mutex, Notify},
    task::spawn_blocking,
    time::timeout,
};
use tracing::{info, warn};

pub(super) const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const ORIGIN: &str = "http://127.0.0.1:8898";
const REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

const TOKEN_ENDPOINT: &str = "https://accounts.spotify.com/api/token";
const OAUTH_CACHE: &str = "spotify-oauth.json";

pub(super) async fn connect(http: Client, reauthorize: bool) -> MusicResult<Arc<Session>> {
    if !reauthorize && let Some(credentials) = spawn_blocking(load_credentials).await? {
        let session = authorized_session(http.clone(), credentials, None)?;
        match session.auth_token().await {
            Ok(_) => return Ok(session),
            Err(error) if error.is::<AuthorizationExpired>() => {}
            Err(error) => return Err(error),
        }
    }

    let (credentials, token) = browser_login(&http).await?;
    authorized_session(http, credentials.persist().await?, Some(token))
}

fn authorized_session(
    http: Client,
    credentials: Credentials,
    access_token: Option<Token>,
) -> MusicResult<Arc<Session>> {
    let tokens = Tokens { refresh_token: credentials.refresh_token, access_token };
    Session::new(http, credentials.username, Authorization {
        tokens: Arc::new(Mutex::new(tokens)),
        expired: Arc::default(),
    })
    .map(Arc::new)
}

async fn browser_login(http: &Client) -> MusicResult<(Credentials, Token)> {
    let random_secret = || -> MusicResult<String> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)?;
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    };
    let state = random_secret()?;
    let verifier = random_secret()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse("https://accounts.spotify.com/authorize")?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("scope", "streaming app-remote-control"),
        ("state", &state),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge),
    ]);

    let listener = TcpListener::bind("127.0.0.1:8898").await?;
    platform::try_open_url(url.as_str())?;
    info!("Waiting for Spotify sign-in in your browser. Playback and playlists will appear after authorization.");
    loop {
        let (socket, _) = listener.accept().await?;
        let mut socket = BufReader::new(socket);
        let mut request = String::new();
        if !matches!(timeout(REQUEST_TIMEOUT, (&mut socket).take(8192).read_line(&mut request)).await, Ok(Ok(_))) {
            continue;
        }
        let callback = callback_code(&request, &state);
        let Some(Ok(code)) = callback else {
            let response = if callback.is_some() {
                warn!("Spotify browser authorization was declined");
                b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            };
            socket.get_mut().write_all(response).await?;
            continue;
        };
        let result = exchange_code(http, &code, &verifier).await;
        let (status, message) = if result.is_ok() {
            ("200 OK", "Cantus connected successfully. You can close this tab.")
        } else {
            ("502 Bad Gateway", "Cantus could not complete Spotify authorization.")
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{message}",
            message.len()
        );
        if let Err(error) = socket.get_mut().write_all(response.as_bytes()).await {
            warn!(%error, "Could not send Spotify login result to the browser");
        }
        return result;
    }
}

#[derive(Deserialize)]
struct OAuthToken {
    access_token: String,
    expires_in: u64,
    username: Option<String>,
    refresh_token: Option<String>,
}

impl OAuthToken {
    async fn decode(response: Response) -> MusicResult<Self> {
        let token = response.error_for_status()?.json::<Self>().await?;
        if token.access_token.is_empty() {
            return Err("Spotify did not return an access token".into());
        }
        Ok(token)
    }
}

async fn exchange_code(http: &Client, code: &str, verifier: &str) -> MusicResult<(Credentials, Token)> {
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .send()
        .await?;
    let token = OAuthToken::decode(response).await?;
    let username = token.username.filter(|username| !username.is_empty()).ok_or("Spotify omitted the username")?;
    let refresh_token =
        token.refresh_token.filter(|token| !token.is_empty()).ok_or("Spotify omitted the refresh token")?;
    Ok((Credentials { username, refresh_token }, Token::new(token.access_token, Duration::from_secs(token.expires_in))))
}

async fn refresh_access_token(http: &Client, endpoint: &str, refresh_token: &str) -> MusicResult<OAuthToken> {
    let response = http
        .post(endpoint)
        .form(&[("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", refresh_token)])
        .send()
        .await?;
    if response.status() == StatusCode::BAD_REQUEST {
        #[derive(Deserialize)]
        struct OAuthError {
            error: String,
        }
        if response.json::<OAuthError>().await.is_ok_and(|body| body.error == "invalid_grant") {
            return Err(AuthorizationExpired.into());
        }
        return Err("Spotify rejected the token renewal request".into());
    }
    OAuthToken::decode(response).await
}

#[derive(Debug)]
pub(super) struct AuthorizationExpired;

impl Display for AuthorizationExpired {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str("Spotify authorization expired")
    }
}

impl StdError for AuthorizationExpired {}

fn callback_code(request: &str, expected_state: &str) -> Option<Result<String, ()>> {
    let mut parts = request.split_ascii_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?.strip_prefix("/login?")?;
    let callback = Url::parse(&format!("{ORIGIN}/login?{target}")).ok()?;
    let mut pairs = callback.query_pairs();
    let state = pairs.find(|(key, _)| key == "state")?.1;
    if state != expected_state {
        return None;
    }
    if callback.query_pairs().any(|(key, _)| key == "error") {
        return Some(Err(()));
    }
    callback.query_pairs().find(|(key, code)| key == "code" && !code.is_empty()).map(|(_, code)| Ok(code.into_owned()))
}

fn load_credentials() -> Option<Credentials> {
    let read = || -> MusicResult<Credentials> {
        let credentials: Credentials = serde_json::from_slice(&fs::read(config::directory().join(OAUTH_CACHE))?)?;
        if credentials.username.is_empty() || credentials.refresh_token.is_empty() {
            return Err("Invalid cached Spotify credentials".into());
        }
        Ok(credentials)
    };
    read()
        .inspect_err(|error| {
            if error.downcast_ref::<io::Error>().is_none_or(|error| error.kind() != io::ErrorKind::NotFound) {
                warn!(%error, "Could not read cached Spotify credentials");
            }
        })
        .ok()
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    username: String,
    refresh_token: String,
}

impl Credentials {
    async fn persist(self) -> MusicResult<Self> {
        Ok(spawn_blocking(move || {
            let write = || -> MusicResult<()> {
                let directory = config::directory();
                fs::create_dir_all(&directory)?;
                let mut file = tempfile::NamedTempFile::new_in(&directory)?;
                serde_json::to_writer(file.as_file_mut(), &self)?;
                file.persist(directory.join(OAUTH_CACHE))?;
                Ok(())
            };
            if let Err(error) = write() {
                warn!(%error, "Could not save Spotify credentials");
            }
            self
        })
        .await?)
    }
}

#[derive(Default)]
pub(super) struct Authorization {
    tokens: Arc<Mutex<Tokens>>,
    pub(super) expired: Arc<Notify>,
}

#[derive(Default)]
struct Tokens {
    refresh_token: String,
    access_token: Option<Token>,
}

impl Session {
    pub(super) async fn auth_token(&self) -> MusicResult<String> {
        self.auth.token(&self.http, &self.username, TOKEN_ENDPOINT).await
    }
}

impl Authorization {
    #[expect(
        clippy::significant_drop_tightening,
        reason = "The refresh task takes over the lock to serialize token renewal"
    )]
    async fn token(&self, http: &Client, username: &str, endpoint: &str) -> MusicResult<String> {
        let mut auth = Arc::clone(&self.tokens).lock_owned().await;
        if let Some(token) = auth.access_token.as_ref().filter(|token| Instant::now() < token.expires_at) {
            return Ok(token.access_token.clone());
        }
        if auth.refresh_token.is_empty() {
            return Err(AuthorizationExpired.into());
        }
        let expired = Arc::clone(&self.expired);
        let http = http.clone();
        let username = username.to_owned();
        let endpoint = endpoint.to_owned();
        // Spotify can rotate the refresh token even if the caller stops waiting for the response.
        tokio::spawn(async move {
            let response = match refresh_access_token(&http, &endpoint, &auth.refresh_token).await {
                Err(error) if error.is::<AuthorizationExpired>() => {
                    auth.refresh_token.clear();
                    auth.access_token = None;
                    expired.notify_one();
                    return Err(error);
                }
                result => result?,
            };
            if let Some(refresh_token) =
                response.refresh_token.filter(|next| !next.is_empty() && next != &auth.refresh_token)
            {
                auth.refresh_token.clone_from(&refresh_token);
                Credentials { username, refresh_token }.persist().await?;
            }
            let access_token = response.access_token;
            auth.access_token = Some(Token::new(access_token.clone(), Duration::from_secs(response.expires_in)));
            Ok(access_token)
        })
        .await?
    }
}

struct Token {
    access_token: String,
    expires_at: Instant,
}

impl Token {
    fn new(access_token: String, lifetime: Duration) -> Self {
        Self { access_token, expires_at: Instant::now() + lifetime.saturating_sub(Duration::from_secs(10)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejected_refresh_signals_expiry_even_if_caller_is_cancelled() -> MusicResult<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let auth = Authorization::default();
        auth.tokens.lock().await.refresh_token = "cached-refresh-token".into();
        let http = Client::builder().no_proxy().build()?;

        timeout(REQUEST_TIMEOUT, async {
            let mut refresh = Box::pin(auth.token(&http, "", &endpoint));
            let (mut socket, _) = tokio::select! {
                connection = listener.accept() => connection?,
                _ = &mut refresh => return Err("refresh finished before Spotify responded".into()),
            };
            // Read the full request before responding so closing the socket delivers the response.
            let mut reader = BufReader::new(&mut socket);
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await? == 0 {
                    return Err("refresh request ended before the headers were complete".into());
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("content-length: ") {
                    content_length = value.trim().parse()?;
                }
            }
            reader.read_exact(&mut vec![0; content_length]).await?;
            drop(refresh);
            socket.write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 25\r\nConnection: close\r\n\r\n{\"error\":\"invalid_grant\"}",
            ).await?;
            auth.expired.notified().await;
            match auth.token(&http, "", &endpoint).await {
                Err(error) if error.is::<AuthorizationExpired>() => Ok(()),
                Err(error) => Err(error),
                Ok(_) => Err("rejected refresh token was still usable".into()),
            }
        })
        .await?
    }

    #[test]
    fn oauth_callback_requires_matching_state() {
        let request = "GET /login?code=a%2Bb&state=expected HTTP/1.1\r\n";
        assert_eq!(callback_code(request, "expected"), Some(Ok("a+b".into())));
        let denied = "GET /login?error=access_denied&state=expected HTTP/1.1\r\n";
        assert_eq!(callback_code(denied, "expected"), Some(Err(())));
        assert!(callback_code(request, "different").is_none());
        assert!(callback_code("GET /other?code=a&state=expected HTTP/1.1", "expected").is_none());
    }
}
