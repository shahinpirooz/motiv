//! Motiv: connected accounts (Microsoft 365 and Google) for calendars and
//! meeting participants.
//!
//! Motiv itself has no account. "Connect" runs the provider's normal browser
//! sign-in (OAuth, see `oauth.rs`) against an app registration the user owns;
//! the resulting tokens are kept in the OS keychain (macOS Keychain, Windows
//! Credential Manager) and used from this machine only.

pub mod oauth;

use log::{info, warn};
use oauth::{Provider, Tokens};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

const KEYCHAIN_SERVICE: &str = "Motiv";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProviderConfig {
    /// Application (client) ID of the user's app registration.
    pub client_id: String,
    /// Google "Desktop app" clients come with a (non-confidential) secret.
    pub client_secret: String,
    /// Microsoft only: "organizations" (work accounts), "common", or a tenant ID.
    pub tenant: String,
    /// Microsoft only: also read Teams attendance reports (needs admin consent).
    pub teams_attendance: bool,
    /// Signed-in account, shown in Settings.
    pub account: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AccountsConfig {
    pub microsoft: ProviderConfig,
    pub google: ProviderConfig,
}

impl AccountsConfig {
    pub fn get(&self, p: Provider) -> &ProviderConfig {
        match p {
            Provider::Microsoft => &self.microsoft,
            Provider::Google => &self.google,
        }
    }
    pub fn get_mut(&mut self, p: Provider) -> &mut ProviderConfig {
        match p {
            Provider::Microsoft => &mut self.microsoft,
            Provider::Google => &mut self.google,
        }
    }
}

pub fn tenant(cfg: &ProviderConfig) -> String {
    let t = cfg.tenant.trim();
    if t.is_empty() { "organizations".into() } else { t.to_string() }
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("accounts.json"))
}

pub fn load_config<R: Runtime>(app: &AppHandle<R>) -> AccountsConfig {
    let mut c: AccountsConfig = config_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    if c.microsoft.tenant.is_empty() {
        c.microsoft.tenant = "organizations".into();
    }
    c
}

fn save_config<R: Runtime>(app: &AppHandle<R>, c: &AccountsConfig) -> Result<(), String> {
    let p = config_path(app).ok_or("No config folder")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(c).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Token storage
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod store {
    use super::*;
    fn entry(p: Provider) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &format!("oauth-{}", p.key())).map_err(|e| e.to_string())
    }
    pub fn read<R: Runtime>(_app: &AppHandle<R>, p: Provider) -> Option<Tokens> {
        entry(p).ok()?.get_password().ok().and_then(|j| serde_json::from_str(&j).ok())
    }
    pub fn write<R: Runtime>(_app: &AppHandle<R>, p: Provider, t: &Tokens) -> Result<(), String> {
        entry(p)?.set_password(&serde_json::to_string(t).map_err(|e| e.to_string())?).map_err(|e| format!("Could not save to the keychain: {}", e))
    }
    pub fn delete<R: Runtime>(_app: &AppHandle<R>, p: Provider) {
        if let Ok(e) = entry(p) {
            let _ = e.delete_credential();
        }
    }
}

/// Linux: no system keychain dependency; a user-only file in the config folder.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod store {
    use super::*;
    fn path<R: Runtime>(app: &AppHandle<R>, p: Provider) -> Option<PathBuf> {
        app.path().app_config_dir().ok().map(|d| d.join(format!("oauth-{}.json", p.key())))
    }
    pub fn read<R: Runtime>(app: &AppHandle<R>, p: Provider) -> Option<Tokens> {
        std::fs::read_to_string(path(app, p)?).ok().and_then(|j| serde_json::from_str(&j).ok())
    }
    pub fn write<R: Runtime>(app: &AppHandle<R>, p: Provider, t: &Tokens) -> Result<(), String> {
        let f = path(app, p).ok_or("No config folder")?;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&f, serde_json::to_string(t).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
    pub fn delete<R: Runtime>(app: &AppHandle<R>, p: Provider) {
        if let Some(f) = path(app, p) {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// One refresh at a time per provider.
static REFRESH_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
/// Access tokens kept in memory to avoid a keychain read per request.
static CACHE: LazyLock<Mutex<std::collections::HashMap<Provider, Tokens>>> = LazyLock::new(Default::default);

pub fn is_connected<R: Runtime>(app: &AppHandle<R>, p: Provider) -> bool {
    CACHE.lock().unwrap().contains_key(&p) || store::read(app, p).is_some()
}

/// A valid access token for the provider, refreshing it if needed.
pub async fn access_token<R: Runtime>(app: &AppHandle<R>, p: Provider) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp();
    if let Some(t) = CACHE.lock().unwrap().get(&p).filter(|t| t.is_fresh(now)) {
        return Ok(t.access_token.clone());
    }
    let _guard = REFRESH_LOCK.lock().await;
    let stored = store::read(app, p).ok_or_else(|| format!("{} is not connected.", p.label()))?;
    if stored.is_fresh(now) {
        CACHE.lock().unwrap().insert(p, stored.clone());
        return Ok(stored.access_token);
    }
    let refresh = stored.refresh_token.clone().ok_or_else(|| format!("{} sign-in expired. Connect again in Settings.", p.label()))?;
    let cfg = load_config(app);
    let pc = cfg.get(p);
    let scope = stored.scope.clone().unwrap_or_else(|| oauth::scopes(p, pc.teams_attendance));
    let tenant = tenant(pc);
    let client = oauth::Client { provider: p, tenant: &tenant, client_id: &pc.client_id, client_secret: Some(&pc.client_secret) };
    match client.refresh(&refresh, &scope).await {
        Ok(t) => {
            store::write(app, p, &t)?;
            CACHE.lock().unwrap().insert(p, t.clone());
            Ok(t.access_token)
        }
        Err(e) => {
            warn!("accounts: {} token refresh failed: {}", p.key(), e);
            Err(format!("{} sign-in expired ({}). Connect again in Settings.", p.label(), e))
        }
    }
}

async fn whoami(p: Provider, token: &str) -> Option<String> {
    let url = match p {
        Provider::Microsoft => "https://graph.microsoft.com/v1.0/me?$select=mail,userPrincipalName,displayName",
        Provider::Google => "https://openidconnect.googleapis.com/v1/userinfo",
    };
    let v: serde_json::Value = reqwest::Client::new().get(url).bearer_auth(token).send().await.ok()?.json().await.ok()?;
    ["mail", "email", "userPrincipalName"].iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub provider: Provider,
    pub label: &'static str,
    pub config: ProviderConfig,
    pub connected: bool,
}

#[tauri::command]
pub fn accounts_status<R: Runtime>(app: AppHandle<R>) -> Vec<ProviderStatus> {
    let cfg = load_config(&app);
    [Provider::Microsoft, Provider::Google]
        .into_iter()
        .map(|p| ProviderStatus { provider: p, label: p.label(), config: cfg.get(p).clone(), connected: is_connected(&app, p) })
        .collect()
}

#[tauri::command]
pub fn accounts_save_config<R: Runtime>(app: AppHandle<R>, provider: Provider, config: ProviderConfig) -> Result<(), String> {
    let mut cfg = load_config(&app);
    let account = cfg.get(provider).account.clone();
    let slot = cfg.get_mut(provider);
    *slot = ProviderConfig {
        client_id: config.client_id.trim().to_string(),
        client_secret: config.client_secret.trim().to_string(),
        tenant: config.tenant.trim().to_string(),
        account,
        ..config
    };
    save_config(&app, &cfg)
}

/// Opens the browser sign-in and waits (up to 5 minutes) for it to finish.
#[tauri::command]
pub async fn accounts_connect<R: Runtime>(app: AppHandle<R>, provider: Provider) -> Result<ProviderStatus, String> {
    let mut cfg = load_config(&app);
    let pc = cfg.get(provider).clone();
    if pc.client_id.is_empty() {
        return Err(format!("Enter the {} application (client) ID first.", provider.label()));
    }
    if provider == Provider::Google && pc.client_secret.is_empty() {
        return Err("Enter the Google client secret first.".into());
    }
    let tenant = tenant(&pc);
    let scope = oauth::scopes(provider, pc.teams_attendance);
    let (verifier, challenge) = oauth::pkce_pair();
    let state = oauth::random_state();
    let loopback = oauth::Loopback::bind().await?;
    let redirect = loopback.redirect_uri();
    let url = oauth::authorize_url(&oauth::AuthRequest {
        provider,
        tenant: &tenant,
        client_id: &pc.client_id,
        redirect_uri: &redirect,
        scope: &scope,
        state: &state,
        challenge: &challenge,
    });
    open_in_browser(&url)?;
    let (code, got_state) = loopback.wait(Duration::from_secs(300)).await?;
    if got_state != state {
        return Err("Sign-in response did not match this request. Try again.".into());
    }
    let client = oauth::Client { provider, tenant: &tenant, client_id: &pc.client_id, client_secret: Some(&pc.client_secret) };
    let mut tokens = client.exchange_code(&code, &verifier, &redirect, &scope).await?;
    if tokens.scope.is_none() {
        tokens.scope = Some(scope);
    }
    store::write(&app, provider, &tokens)?;
    CACHE.lock().unwrap().insert(provider, tokens.clone());
    let account = whoami(provider, &tokens.access_token).await;
    info!("accounts: {} connected", provider.key());
    cfg.get_mut(provider).account = account;
    save_config(&app, &cfg)?;
    crate::calendar::invalidate_cache();
    Ok(ProviderStatus { provider, label: provider.label(), config: cfg.get(provider).clone(), connected: true })
}

#[tauri::command]
pub fn accounts_disconnect<R: Runtime>(app: AppHandle<R>, provider: Provider) -> Result<(), String> {
    store::delete(&app, provider);
    CACHE.lock().unwrap().remove(&provider);
    let mut cfg = load_config(&app);
    cfg.get_mut(provider).account = None;
    save_config(&app, &cfg)?;
    crate::calendar::invalidate_cache();
    Ok(())
}

/// Opens a URL in the default browser. (Windows: avoids `cmd start`, which
/// splits URLs at `&`.)
pub fn open_in_browser(url: &str) -> Result<(), String> {
    use std::process::Command;
    let r = if cfg!(target_os = "windows") {
        Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    r.map(|_| ()).map_err(|e| format!("Could not open the browser: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_tenant() {
        let c: AccountsConfig = serde_json::from_str(r#"{"google":{"client_id":"g"}}"#).unwrap();
        assert_eq!(c.google.client_id, "g");
        assert_eq!(tenant(&c.microsoft), "organizations");
        let pc = ProviderConfig { tenant: " contoso.onmicrosoft.com ".into(), ..Default::default() };
        assert_eq!(tenant(&pc), "contoso.onmicrosoft.com");
    }
}
