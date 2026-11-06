//! OAuth 2.0 for desktop apps: authorization code + PKCE with a loopback
//! redirect (RFC 8252). The browser signs in and redirects to
//! http://localhost:<port>/ where a one-shot listener in Motiv picks up the
//! code. No server is involved.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Microsoft,
    Google,
}

impl Provider {
    pub fn key(self) -> &'static str {
        match self {
            Provider::Microsoft => "microsoft",
            Provider::Google => "google",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Microsoft => "Microsoft 365",
            Provider::Google => "Google",
        }
    }
}

/// What Motiv asks to read.
pub fn scopes(provider: Provider, teams_attendance: bool) -> String {
    match provider {
        Provider::Microsoft => {
            let mut s = vec!["offline_access", "openid", "profile", "email", "User.Read", "Calendars.Read", "OnlineMeetings.Read"];
            if teams_attendance {
                s.push("OnlineMeetingArtifact.Read.All");
            }
            s.join(" ")
        }
        Provider::Google => "openid email https://www.googleapis.com/auth/calendar.readonly".into(),
    }
}

pub fn authorize_endpoint(provider: Provider, tenant: &str) -> String {
    match provider {
        Provider::Microsoft => format!("https://login.microsoftonline.com/{}/oauth2/v2.0/authorize", tenant),
        Provider::Google => "https://accounts.google.com/o/oauth2/v2/auth".into(),
    }
}

pub fn token_endpoint(provider: Provider, tenant: &str) -> String {
    match provider {
        Provider::Microsoft => format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", tenant),
        Provider::Google => "https://oauth2.googleapis.com/token".into(),
    }
}

/// PKCE verifier (43+ chars) and its S256 challenge.
pub fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    (verifier.clone(), challenge_for(&verifier))
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn random_state() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub struct AuthRequest<'a> {
    pub provider: Provider,
    pub tenant: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub scope: &'a str,
    pub state: &'a str,
    pub challenge: &'a str,
}

pub fn authorize_url(r: &AuthRequest) -> String {
    let mut url = url::Url::parse(&authorize_endpoint(r.provider, r.tenant)).expect("static URL");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", r.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", r.redirect_uri)
            .append_pair("scope", r.scope)
            .append_pair("state", r.state)
            .append_pair("code_challenge", r.challenge)
            .append_pair("code_challenge_method", "S256");
        match r.provider {
            Provider::Microsoft => {
                q.append_pair("response_mode", "query").append_pair("prompt", "select_account");
            }
            Provider::Google => {
                q.append_pair("access_type", "offline").append_pair("prompt", "consent");
            }
        }
    }
    url.into()
}

/// Parses the request line of the redirect ("GET /?code=..&state=.. HTTP/1.1").
pub fn parse_redirect(request: &str) -> Result<(String, String), String> {
    let line = request.lines().next().unwrap_or_default();
    let target = line.split_whitespace().nth(1).ok_or("Bad redirect request")?;
    let url = url::Url::parse(&format!("http://localhost{}", target)).map_err(|e| e.to_string())?;
    let get = |k: &str| url.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
    if let Some(err) = get("error") {
        let desc = get("error_description").unwrap_or_default();
        return Err(if desc.is_empty() { err } else { format!("{}: {}", err, desc) });
    }
    Ok((get("code").ok_or("No authorization code in redirect")?, get("state").unwrap_or_default()))
}

/// Listens on 127.0.0.1 and ::1 on the same free port.
pub struct Loopback {
    v4: TcpListener,
    v6: Option<TcpListener>,
    pub port: u16,
}

impl Loopback {
    pub async fn bind() -> Result<Self, String> {
        let v4 = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("Could not open a local port: {}", e))?;
        let port = v4.local_addr().map_err(|e| e.to_string())?.port();
        let v6 = TcpListener::bind(("::1", port)).await.ok();
        Ok(Self { v4, v6, port })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}/", self.port)
    }

    /// Waits for the browser's redirect, answers it, and returns (code, state).
    pub async fn wait(self, timeout: Duration) -> Result<(String, String), String> {
        let accept = async {
            loop {
                let (mut stream, _) = match &self.v6 {
                    Some(v6) => tokio::select! { r = self.v4.accept() => r, r = v6.accept() => r },
                    None => self.v4.accept().await,
                }
                .map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                // Browsers sometimes ask for /favicon.ico first; ignore anything without a query.
                if !req.lines().next().unwrap_or_default().contains('?') {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
                let result = parse_redirect(&req);
                let (title, msg) = match &result {
                    Ok(_) => ("Connected", "Motiv is connected. You can close this tab and return to Motiv."),
                    Err(_) => ("Not connected", "Sign-in did not complete. Return to Motiv for details."),
                };
                let body = format!(
                    "<!doctype html><meta charset=utf-8><title>Motiv</title><body style=\"font:16px -apple-system,Segoe UI,sans-serif;display:grid;place-items:center;height:90vh;color:#111\"><div style=\"text-align:center\"><h2>{}</h2><p>{}</p></div>",
                    title, msg
                );
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return result;
            }
        };
        tokio::time::timeout(timeout, accept).await.map_err(|_| "Sign-in timed out. Try again.".to_string())?
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scope: Option<String>,
}

impl Tokens {
    pub fn is_fresh(&self, now: i64) -> bool {
        now + 120 < self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

pub fn parse_token_response(body: &str, now: i64, previous_refresh: Option<String>) -> Result<Tokens, String> {
    if let Ok(t) = serde_json::from_str::<TokenResponse>(body) {
        return Ok(Tokens {
            access_token: t.access_token,
            // Google omits the refresh token on refresh; keep the old one.
            refresh_token: t.refresh_token.or(previous_refresh),
            expires_at: now + t.expires_in.unwrap_or(3600),
            scope: t.scope,
        });
    }
    match serde_json::from_str::<TokenError>(body) {
        Ok(e) => Err(match e.error_description {
            Some(d) => format!("{}: {}", e.error, d.lines().next().unwrap_or_default()),
            None => e.error,
        }),
        Err(_) => Err("Unexpected response from the sign-in service".into()),
    }
}

pub struct Client<'a> {
    pub provider: Provider,
    pub tenant: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
}

impl Client<'_> {
    async fn post(&self, mut form: Vec<(&str, String)>, previous_refresh: Option<String>) -> Result<Tokens, String> {
        form.push(("client_id", self.client_id.to_string()));
        if let Some(secret) = self.client_secret.filter(|s| !s.is_empty()) {
            form.push(("client_secret", secret.to_string()));
        }
        let resp = reqwest::Client::new()
            .post(token_endpoint(self.provider, self.tenant))
            .form(&form)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Could not reach {}: {}", self.provider.label(), e))?;
        let body = resp.text().await.map_err(|e| e.to_string())?;
        parse_token_response(&body, chrono::Utc::now().timestamp(), previous_refresh)
    }

    pub async fn exchange_code(&self, code: &str, verifier: &str, redirect_uri: &str, scope: &str) -> Result<Tokens, String> {
        self.post(
            vec![
                ("grant_type", "authorization_code".into()),
                ("code", code.into()),
                ("code_verifier", verifier.into()),
                ("redirect_uri", redirect_uri.into()),
                ("scope", scope.into()),
            ],
            None,
        )
        .await
    }

    pub async fn refresh(&self, refresh_token: &str, scope: &str) -> Result<Tokens, String> {
        self.post(
            vec![("grant_type", "refresh_token".into()), ("refresh_token", refresh_token.into()), ("scope", scope.into())],
            Some(refresh_token.to_string()),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let (v, c) = pkce_pair();
        assert!(v.len() >= 43);
        assert_eq!(challenge_for(&v), c);
    }

    #[test]
    fn builds_authorize_urls() {
        let u = authorize_url(&AuthRequest {
            provider: Provider::Microsoft,
            tenant: "organizations",
            client_id: "abc",
            redirect_uri: "http://localhost:5555/",
            scope: &scopes(Provider::Microsoft, true),
            state: "st",
            challenge: "ch",
        });
        assert!(u.starts_with("https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize?client_id=abc&response_type=code"));
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A5555%2F"));
        assert!(u.contains("OnlineMeetingArtifact.Read.All"));
        assert!(u.contains("code_challenge_method=S256"));
        let g = authorize_url(&AuthRequest {
            provider: Provider::Google,
            tenant: "",
            client_id: "g",
            redirect_uri: "http://localhost:1/",
            scope: &scopes(Provider::Google, false),
            state: "s",
            challenge: "c",
        });
        assert!(g.contains("access_type=offline") && g.contains("calendar.readonly"));
        assert!(!scopes(Provider::Microsoft, false).contains("Artifact"));
    }

    #[test]
    fn parses_redirects() {
        assert_eq!(parse_redirect("GET /?code=A%2FB&state=xyz HTTP/1.1\r\nHost: x").unwrap(), ("A/B".into(), "xyz".into()));
        let e = parse_redirect("GET /?error=access_denied&error_description=User+said+no HTTP/1.1").unwrap_err();
        assert_eq!(e, "access_denied: User said no");
        assert!(parse_redirect("GET /?state=1 HTTP/1.1").is_err());
    }

    #[test]
    fn parses_token_responses() {
        let t = parse_token_response(r#"{"access_token":"at","refresh_token":"rt","expires_in":3599}"#, 1000, None).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.expires_at), ("at", Some("rt"), 4599));
        let t = parse_token_response(r#"{"access_token":"at2","expires_in":10}"#, 0, Some("old".into())).unwrap();
        assert_eq!(t.refresh_token.as_deref(), Some("old"));
        assert!(!t.is_fresh(0));
        let e = parse_token_response(r#"{"error":"invalid_grant","error_description":"AADSTS70008: expired\r\nTrace"}"#, 0, None).unwrap_err();
        assert_eq!(e, "invalid_grant: AADSTS70008: expired");
    }

    #[tokio::test]
    async fn loopback_receives_code() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port;
        let waiter = tokio::spawn(lb.wait(Duration::from_secs(5)));
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").await.unwrap();
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /?code=C1&state=S1 HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).await.unwrap();
        assert!(resp.contains("Motiv is connected"));
        assert_eq!(waiter.await.unwrap().unwrap(), ("C1".into(), "S1".into()));
    }
}
