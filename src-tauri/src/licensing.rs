use std::{collections::BTreeSet, io::Read, path::{Path, PathBuf}, process::Command, sync::{Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, time::Duration};
use reqwest::{Client, RequestBuilder, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use crate::error::AppError;

static TOKEN: Mutex<Option<String>> = Mutex::new(None);
static IDENTITY: OnceLock<Result<Identity, String>> = OnceLock::new();
static OPEN_RECORDED: AtomicBool = AtomicBool::new(false);
// Serialize bootstrap redemption with manual activation and release. In particular,
// a late status poll must never activate the computer again after release succeeds.
static ACTIVATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PROCESSED_TICKETS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
// A raster logo is bounded to 150 KiB before base64 encoding by the service.
const MAX_RESPONSE: usize = 256 * 1024;
const MAX_BOOTSTRAP: u64 = 512;

struct RequestFailure { error: AppError, retryable: bool }

impl RequestFailure {
    fn transient(message: impl Into<String>) -> Self { Self { error: failure(message), retryable: true } }
}

struct BootstrapTicket { path: PathBuf, value: String, fingerprint: String }

#[derive(Clone)]
struct Identity { id: String, name: String, platform: String }

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    id: String,
    label: String,
    expires_at: Option<String>,
    max_devices: u32,
    #[serde(default)]
    branding: Option<ResellerBranding>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResellerBranding {
    slug: String,
    name: String,
    tagline: String,
    login_title: String,
    login_message: String,
    accent: String,
    background: String,
    support_snapchat: String,
    logo_data_url: String,
    version: u64,
    updated_at: Option<String>,
}

fn valid_branding(brand: &ResellerBranding) -> bool {
    let text = |value: &str, limit: usize| value.chars().count() <= limit
        && !value.chars().any(|c| matches!(c, '<' | '>') || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')));
    let color = |value: &str| value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit());
    let logo = brand.logo_data_url.is_empty() || (brand.logo_data_url.len() <= 205_000 &&
        ["data:image/png;base64,", "data:image/jpeg;base64,", "data:image/webp;base64,"].iter().any(|prefix| {
            brand.logo_data_url.strip_prefix(prefix).is_some_and(|data| !data.is_empty()
                && data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')))
        }));
    brand.slug.len() <= 64 && !brand.slug.is_empty()
        && brand.slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !brand.name.trim().is_empty() && text(&brand.name, 100) && text(&brand.tagline, 160)
        && text(&brand.login_title, 100) && text(&brand.login_message, 500)
        && color(&brand.accent) && color(&brand.background) && logo
        && (2..=32).contains(&brand.support_snapchat.len())
        && brand.support_snapchat.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    active: bool,
    message: String,
    device_id: String,
    device_name: String,
    license: Option<LicenseInfo>,
}

pub struct InstallLicense { token: String, identity: Identity }

fn failure(message: impl Into<String>) -> AppError { AppError::Misc(message.into()) }

fn machine_hash(raw: &str) -> Result<String, String> {
    let value = raw.trim().to_ascii_lowercase();
    if value.len() < 8 || value.len() > 200 { return Err("Unable to identify this computer for licensing.".into()); }
    Ok(format!("{:x}", Sha256::digest(format!("accesshub-device-v1:{value}").as_bytes())))
}

fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().map_err(|_| "Unable to read this computer's licensing identifier.".to_string())?;
    if !output.status.success() { return Err("Unable to read this computer's licensing identifier.".into()); }
    String::from_utf8(output.stdout).map_err(|_| "Unable to read this computer's licensing identifier.".into())
}

fn read_identity() -> Result<Identity, String> {
    #[cfg(target_os = "windows")]
    let raw = {
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let output = command_output(&format!("{system}\\System32\\reg.exe"), &["query", "HKLM\\SOFTWARE\\Microsoft\\Cryptography", "/v", "MachineGuid", "/reg:64"])?;
        output.lines().find(|line| line.trim_start().starts_with("MachineGuid"))
            .and_then(|line| line.split_whitespace().last()).ok_or("This computer has no available licensing identifier.")?.to_string()
    };
    #[cfg(target_os = "macos")]
    let raw = {
        let output = command_output("/usr/sbin/ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
        output.lines().find(|line| line.contains("\"IOPlatformUUID\""))
            .and_then(|line| line.split('"').nth(3)).ok_or("This Mac has no available licensing identifier.")?.to_string()
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let raw = std::fs::read_to_string("/etc/machine-id").map_err(|_| "This computer has no available licensing identifier.")?;
    #[cfg(target_os = "macos")]
    let name = command_output("/usr/sbin/scutil", &["--get", "ComputerName"]).unwrap_or_else(|_| "Mac".into());
    #[cfg(not(target_os = "macos"))]
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Computer".into());
    Ok(Identity { id: machine_hash(&raw)?, name: name.trim().chars().filter(|c| !c.is_control()).take(80).collect(), platform: std::env::consts::OS.into() })
}

fn identity() -> Result<Identity, AppError> {
    IDENTITY.get_or_init(read_identity).clone().map_err(failure)
}

fn endpoint(path: &str) -> Result<String, AppError> {
    let config: Value = serde_json::from_str(include_str!("../license-config.json")).map_err(|_| failure("Licensing is not configured in this build."))?;
    let base = config.get("api_url").and_then(Value::as_str).unwrap_or("").trim_end_matches('/');
    let url = reqwest::Url::parse(base).map_err(|_| failure("Licensing is not configured in this build."))?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(failure("The licensing service address is invalid."));
    }
    Ok(format!("{base}{path}"))
}

fn retryable_status(status: StatusCode) -> bool {
    !status.is_client_error() || matches!(status, StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS)
}

async fn service_request(path: &str, token: Option<&str>, body: Value) -> Result<Value, RequestFailure> {
    let client = Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(15)).build()
        .map_err(|_| RequestFailure::transient("Unable to contact AccessHub licensing."))?;
    let url = endpoint(path).map_err(|error| RequestFailure { error, retryable: true })?;
    let mut req = client.post(url).header("Content-Type", "application/json");
    if let Some(token) = token { req = req.bearer_auth(token); }
    let mut response = req.body(body.to_string()).send().await
        .map_err(|_| RequestFailure::transient("AccessHub needs an internet connection to check your key. Please try again."))?;
    let status = response.status();
    if response.content_length().is_some_and(|size| size > MAX_RESPONSE as u64) { return Err(RequestFailure::transient("The licensing service returned an invalid response.")); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| RequestFailure::transient("Unable to read the licensing response."))? {
        if bytes.len() + chunk.len() > MAX_RESPONSE { return Err(RequestFailure::transient("The licensing response was too large.")); }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| RequestFailure::transient("The licensing service is temporarily unavailable."))?;
    if !status.is_success() {
        let fallback = if status == StatusCode::TOO_MANY_REQUESTS { "Too many attempts. Wait a little and try again." } else { "Your AccessHub key could not be verified." };
        let message = data.pointer("/error/message").and_then(Value::as_str).unwrap_or(fallback);
        return Err(RequestFailure { error: failure(message.chars().take(300).collect::<String>()), retryable: retryable_status(status) });
    }
    Ok(data)
}

async fn request(path: &str, token: Option<&str>, body: Value) -> Result<Value, AppError> {
    service_request(path, token, body).await.map_err(|result| result.error)
}

fn bootstrap_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    { std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("app.accesshub.resellerloader").join("activation-ticket")) }
    #[cfg(target_os = "macos")]
    { Some(PathBuf::from("/Library/Application Support/AccessHub Reseller/activation-ticket")) }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    { None }
}

fn valid_ticket(value: &str) -> bool {
    value.len() == 43 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn read_bootstrap(path: &Path) -> Option<BootstrapTicket> {
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() { return None; }
    let mut bytes = Vec::new();
    file.take(MAX_BOOTSTRAP + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > MAX_BOOTSTRAP as usize { return None; }
    let value = std::str::from_utf8(&bytes).ok()?.trim();
    if !valid_ticket(value) { return None; }
    let fingerprint = format!("{:x}", Sha256::digest(format!("accesshub-activation-ticket-v1:{value}").as_bytes()));
    Some(BootstrapTicket { path: path.to_owned(), value: value.to_owned(), fingerprint })
}

fn marker_name(device: &Identity, ticket: &BootstrapTicket) -> String {
    format!("{}:{}", device.id, ticket.fingerprint)
}

fn ticket_processed(device: &Identity, ticket: &BootstrapTicket) -> bool {
    let name = marker_name(device, ticket);
    if PROCESSED_TICKETS.lock().is_ok_and(|seen| seen.contains(&name)) { return true; }
    keyring::Entry::new("app.accesshub.resellerloader.activation-ticket", &name).ok()
        .and_then(|entry| entry.get_password().ok()).is_some_and(|value| value == "processed")
}

fn finish_bootstrap(device: &Identity, ticket: &BootstrapTicket, persistent: bool) {
    let name = marker_name(device, ticket);
    if let Ok(mut seen) = PROCESSED_TICKETS.lock() { seen.insert(name.clone()); }
    if !persistent { return; }
    // The marker contains only a scoped hash. It also covers the root-owned Mac
    // bootstrap, which a normal user cannot delete. Keep it after deactivation.
    if let Ok(entry) = keyring::Entry::new("app.accesshub.resellerloader.activation-ticket", &name) { let _ = entry.set_password("processed"); }
    remove_matching_bootstrap(ticket);
}

fn remove_matching_bootstrap(ticket: &BootstrapTicket) {
    // Do not remove a newer download that replaced this file during redemption.
    if read_bootstrap(&ticket.path).is_some_and(|current| current.fingerprint == ticket.fingerprint) {
        let _ = std::fs::remove_file(&ticket.path);
    }
}

fn pending_bootstrap() -> Option<BootstrapTicket> { bootstrap_path().and_then(|path| read_bootstrap(&path)) }

fn stored_token(device: &Identity) -> Option<String> {
    if let Ok(guard) = TOKEN.lock() { if let Some(token) = &*guard { return Some(token.clone()); } }
    let value = keyring::Entry::new("app.accesshub.resellerloader.licensing", &device.id).ok()?.get_password().ok()?;
    if value.is_empty() || value.len() > 2048 { return None; }
    if let Ok(mut guard) = TOKEN.lock() { *guard = Some(value.clone()); }
    Some(value)
}

fn is_protected_app_url(url: &Url, service: &Url) -> bool {
    if url.scheme() != "https" || url.origin() != service.origin()
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return false;
    }
    let Some(id) = url.path().strip_prefix("/v1/apps/") else { return false; };
    id == "7e6d7e51-dfc2-4b38-97ce-52f1414d8ef0"
}

fn protected_download_request(url: Url, token: &str, device_id: &str) -> Result<RequestBuilder, AppError> {
    // This client never follows redirects: neither header may leave the approved route.
    let client = Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30)).timeout(Duration::from_secs(15 * 60)).build()
        .map_err(|_| failure("Unable to prepare the app download."))?;
    Ok(client.get(url).bearer_auth(token).header("X-AccessHub-Device", device_id))
}

pub fn app_download_request(external_client: &Client, url: Url) -> Result<RequestBuilder, AppError> {
    let service = Url::parse(&endpoint("")?).map_err(|_| failure("The licensing service address is invalid."))?;
    if !is_protected_app_url(&url, &service) {
        // Public and third-party HTTPS hosts never receive the activation token or device ID.
        return Ok(external_client.get(url));
    }
    let device = identity()?;
    let token = stored_token(&device).ok_or_else(|| failure("Activate AccessHub before downloading this app."))?;
    protected_download_request(url, &token, &device.id)
}

fn status(device: &Identity, active: bool, message: impl Into<String>, license: Option<LicenseInfo>) -> LicenseStatus {
    LicenseStatus { active, message: message.into(), device_id: device.id.clone(), device_name: device.name.clone(), license }
}

fn verified_info(data: &Value, device: &Identity) -> Result<LicenseInfo, AppError> {
    if data.get("status").and_then(Value::as_str) != Some("active") || data.pointer("/device/id").and_then(Value::as_str) != Some(device.id.as_str()) {
        return Err(failure("This computer is not activated for AccessHub."));
    }
    let info: LicenseInfo = serde_json::from_value(data.get("license").cloned().unwrap_or(Value::Null))
        .map_err(|_| failure("The licensing response was invalid."))?;
    if info.branding.as_ref().is_some_and(|brand| !valid_branding(brand)) {
        return Err(failure("The reseller branding response was invalid."));
    }
    Ok(info)
}

fn save_activation(data: &Value, device: &Identity) -> Result<(LicenseInfo, String, bool), AppError> {
    let info = verified_info(data, device)?;
    let token = data.get("token").and_then(Value::as_str).filter(|token| token.len() >= 24 && token.len() <= 2048)
        .ok_or_else(|| failure("The licensing service did not return a valid session."))?.to_string();
    let saved = keyring::Entry::new("app.accesshub.resellerloader.licensing", &device.id).and_then(|entry| entry.set_password(&token)).is_ok();
    *TOKEN.lock().map_err(|_| failure("Unable to save the activation session."))? = Some(token.clone());
    OPEN_RECORDED.store(false, Ordering::Relaxed);
    Ok((info, token, saved))
}

async fn redeem_bootstrap(device: &Identity, ticket: &BootstrapTicket) -> Result<LicenseStatus, AppError> {
    let data = match service_request("/v1/activate-ticket", None, json!({
        "ticket":ticket.value,"deviceId":device.id,"deviceName":device.name,
        "platform":device.platform,"appVersion":env!("CARGO_PKG_VERSION")
    })).await {
        Ok(data) => data,
        Err(result) => {
            // Offline/server errors keep the ticket for the next status check.
            // Explicit denials fall back to entering a key or a fresh download.
            if !result.retryable { finish_bootstrap(device, ticket, true); }
            return Ok(status(device, false, result.error.to_string(), None));
        }
    };
    let (info, token, saved) = save_activation(&data, device)?;
    finish_bootstrap(device, ticket, saved);
    record_open(&token, device).await;
    Ok(status(device, true, if saved { "Activated automatically" } else {
        "Activated for this session. Secure storage is unavailable; open your personal download again after reopening."
    }, Some(info)))
}

#[tauri::command]
pub async fn license_status() -> Result<LicenseStatus, AppError> {
    let _activation = ACTIVATION.lock().await;
    let device = identity()?;
    // A freshly downloaded ticket deliberately takes priority over an old key.
    if let Some(ticket) = pending_bootstrap() {
        if !ticket_processed(&device, &ticket) { return redeem_bootstrap(&device, &ticket).await; }
    }
    let Some(token) = stored_token(&device) else { return Ok(status(&device, false, "Enter your AccessHub key to activate this computer.", None)); };
    match request("/v1/check", Some(&token), json!({"deviceId":device.id,"appVersion":env!("CARGO_PKG_VERSION")})).await {
        Ok(data) => match verified_info(&data, &device) {
            Ok(info) => {
                record_open(&token, &device).await;
                Ok(status(&device, true, "Activated", Some(info)))
            },
            Err(error) => Ok(status(&device, false, error.to_string(), None)),
        },
        Err(error) => Ok(status(&device, false, error.to_string(), None)),
    }
}

#[tauri::command]
pub async fn license_activate(key: String) -> Result<LicenseStatus, AppError> {
    let _activation = ACTIVATION.lock().await;
    let device = identity()?;
    let key = key.trim();
    if key.len() < 12 || key.len() > 160 { return Err(failure("Enter a valid AccessHub key.")); }
    let data = request("/v1/activate", None, json!({"key":key,"deviceId":device.id,"deviceName":device.name,"platform":device.platform,"appVersion":env!("CARGO_PKG_VERSION")})).await?;
    let (info, token, saved) = save_activation(&data, &device)?;
    // An explicit key wins over any unredeemed installer ticket.
    if let Some(ticket) = pending_bootstrap() { finish_bootstrap(&device, &ticket, saved); }
    record_open(&token, &device).await;
    Ok(status(&device, true, if saved { "Activated" } else { "Activated for this session. Secure storage is unavailable; enter your key again after reopening." }, Some(info)))
}

#[tauri::command]
pub async fn license_deactivate() -> Result<LicenseStatus, AppError> {
    let _activation = ACTIVATION.lock().await;
    let device = identity()?;
    if let Some(token) = stored_token(&device) { request("/v1/deactivate", Some(&token), json!({"deviceId":device.id})).await?; }
    if let Some(ticket) = pending_bootstrap() { finish_bootstrap(&device, &ticket, true); }
    if let Ok(entry) = keyring::Entry::new("app.accesshub.resellerloader.licensing", &device.id) { let _ = entry.delete_credential(); }
    *TOKEN.lock().map_err(|_| failure("Unable to clear the activation session."))? = None;
    Ok(status(&device, false, "This computer has been released. Enter a key to activate again.", None))
}

pub async fn authorize_install(release_id: &str) -> Result<InstallLicense, AppError> {
    if release_id != "reseller-demo-ios-1" {
        return Err(failure("This loader installs only the fictional reseller demo."));
    }
    let device = identity()?;
    let token = stored_token(&device).ok_or_else(|| failure("Activate AccessHub with your key before installing."))?;
    let data = request("/v1/authorize-install", Some(&token), json!({"deviceId":device.id,"releaseId":release_id,"appVersion":env!("CARGO_PKG_VERSION")})).await?;
    if data.get("allowed").and_then(Value::as_bool) != Some(true) { return Err(failure("Your key does not allow this installation.")); }
    Ok(InstallLicense { token, identity: device })
}

pub async fn record_event(session: &InstallLicense, event_type: &str, release_id: &str, code: Option<&str>) {
    // Only app-owned event names, release IDs and error categories go to the service.
    // Raw Apple errors, passwords, verification codes and account identifiers never do.
    let body = json!({"deviceId":session.identity.id,"type":event_type,"releaseId":release_id,"code":code});
    if !matches!(tokio::time::timeout(Duration::from_secs(3), request("/v1/events", Some(&session.token), body)).await, Ok(Ok(_))) {
        tracing::warn!("An AccessHub activity event could not be recorded.");
    }
}

async fn record_open(token: &str, device: &Identity) {
    if !OPEN_RECORDED.swap(true, Ordering::Relaxed) {
        let session = InstallLicense { token: token.to_string(), identity: device.clone() };
        record_event(&session, "app_open", "accesshub-installer", None).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_is_bounded_ascii_and_exactly_one_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("activation-ticket");
        let value = format!("{}_B-", "a".repeat(40));
        assert_eq!(value.len(), 43);
        for valid in [value.clone(), format!("{value}\r\n")] {
            std::fs::write(&path, valid).unwrap();
            let ticket = read_bootstrap(&path).unwrap();
            assert_eq!(ticket.value, value);
            assert_eq!(ticket.fingerprint.len(), 64);
            assert!(!ticket.fingerprint.contains(&value));
        }
        for invalid in ["a".repeat(42), "a".repeat(44), format!("{}!", "a".repeat(42)),
            format!("{}é", "a".repeat(41)), format!("{value}\n{value}"),
            format!("{value}{}", " ".repeat(MAX_BOOTSTRAP as usize))] {
            std::fs::write(&path, invalid).unwrap();
            assert!(read_bootstrap(&path).is_none());
        }
        assert!(read_bootstrap(directory.path()).is_none());
    }

    #[test]
    fn bootstrap_cleanup_preserves_a_new_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("activation-ticket");
        std::fs::write(&path, "A".repeat(43)).unwrap();
        let old = read_bootstrap(&path).unwrap();
        std::fs::write(&path, "B".repeat(43)).unwrap();
        let new = read_bootstrap(&path).unwrap();
        assert_ne!(old.fingerprint, new.fingerprint);
        remove_matching_bootstrap(&old);
        assert_eq!(read_bootstrap(&path).unwrap().value, new.value);
        remove_matching_bootstrap(&new);
        assert!(!path.exists());
    }

    #[test]
    fn processed_tickets_are_scoped_and_keep_recovery_without_secure_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("activation-ticket");
        std::fs::write(&path, "C".repeat(43)).unwrap();
        let ticket = read_bootstrap(&path).unwrap();
        let first = Identity { id:"bootstrap-test-device-a".into(),name:"Test".into(),platform:"windows".into() };
        let second = Identity { id:"bootstrap-test-device-b".into(),name:"Test".into(),platform:"windows".into() };
        assert_ne!(marker_name(&first, &ticket), marker_name(&second, &ticket));
        assert!(!marker_name(&first, &ticket).contains(&ticket.value));
        finish_bootstrap(&first, &ticket, false);
        assert!(ticket_processed(&first, &ticket));
        assert!(path.exists()); // A session-only success remains recoverable next launch.
    }

    #[test]
    fn transient_service_errors_preserve_ticket_retry() {
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(retryable_status(StatusCode::from_u16(code).unwrap()));
        }
        for code in [400, 401, 403, 404, 409, 410, 422] {
            assert!(!retryable_status(StatusCode::from_u16(code).unwrap()));
        }
    }

    #[test]
    fn identifiers_are_normalized_scoped_and_hashed() {
        assert_eq!(machine_hash(" ABCDEF12-3456 ").unwrap(), machine_hash("abcdef12-3456").unwrap());
        let digest = machine_hash("abcdef12-3456").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("abcdef12-3456"));
        assert_ne!(digest, machine_hash("abcdef12-7890").unwrap());
        assert!(machine_hash("").is_err());
    }
    #[test]
    fn session_metadata_must_match_device_and_active_status() {
        let device = Identity { id:"device-a".into(),name:"Test".into(),platform:"windows".into() };
        let valid = json!({"status":"active","device":{"id":"device-a"},"license":{"id":"key-1","label":"Test","expiresAt":null,"maxDevices":1}});
        assert!(verified_info(&valid,&device).is_ok());
        let mut invalid = valid.clone(); invalid["device"]["id"] = json!("device-b");
        assert!(verified_info(&invalid,&device).is_err());
        let mut revoked = valid; revoked["status"] = json!("revoked");
        assert!(verified_info(&revoked,&device).is_err());
    }

    #[test]
    fn reseller_branding_is_bounded_plain_text_and_raster_only() {
        let fixture = json!({"slug":"partner-0123456789ab","name":"Demo Studio","tagline":"Fictional demo",
            "loginTitle":"Welcome","loginMessage":"Demo only","accent":"#f7a65b","background":"#17131e",
            "supportSnapchat":"demo.studio","logoDataUrl":"","version":1,"updatedAt":null});
        let valid: ResellerBranding = serde_json::from_value(fixture.clone()).unwrap();
        assert!(valid_branding(&valid));
        for (field, value) in [
            ("name", "<script>"), ("accent", "red;display:none"),
            ("logoDataUrl", "data:image/svg+xml,<svg></svg>"),
            ("logoDataUrl", "https://external.example/logo.png"),
            ("supportSnapchat", "user/redirect"), ("slug", "../../other"),
        ] {
            let mut invalid = fixture.clone(); invalid[field] = json!(value);
            assert!(!valid_branding(&serde_json::from_value(invalid).unwrap()), "accepted {field}");
        }
        let mut large = fixture.clone(); large["logoDataUrl"] = json!(format!("data:image/png;base64,{}", "A".repeat(200_000)));
        assert!(valid_branding(&serde_json::from_value(large.clone()).unwrap()));
        assert!(serde_json::to_vec(&large).unwrap().len() > 64 * 1024);
        assert!(serde_json::to_vec(&large).unwrap().len() < MAX_RESPONSE);
        large["logoDataUrl"] = json!(format!("data:image/png;base64,{}", "A".repeat(205_000)));
        assert!(!valid_branding(&serde_json::from_value(large).unwrap()));
    }

    #[test]
    fn activation_headers_only_belong_to_the_exact_service_app_route() {
        let service = Url::parse("https://accesshub-licensing.fejiofasj.workers.dev").unwrap();
        let path = "/v1/apps/7e6d7e51-dfc2-4b38-97ce-52f1414d8ef0";
        assert!(is_protected_app_url(&service.join(path).unwrap(), &service));
        for address in [
            format!("https://external.example{path}"),
            format!("https://accesshub-licensing.fejiofasj.workers.dev.evil.example{path}"),
            format!("https://accesshub-licensing.fejiofasj.workers.dev:444{path}"),
            format!("http://accesshub-licensing.fejiofasj.workers.dev{path}"),
            format!("https://user@accesshub-licensing.fejiofasj.workers.dev{path}"),
            format!("https://accesshub-licensing.fejiofasj.workers.dev{path}?redirect=elsewhere"),
            format!("https://accesshub-licensing.fejiofasj.workers.dev{path}/extra"),
            "https://accesshub-licensing.fejiofasj.workers.dev/catalog.json".into(),
            "https://accesshub-licensing.fejiofasj.workers.dev/v1/apps/not-an-id".into(),
            "https://accesshub-licensing.fejiofasj.workers.dev/v1/apps/12345678-1234-4234-8234-123456789abc".into(),
        ] {
            assert!(!is_protected_app_url(&Url::parse(&address).unwrap(), &service), "accepted {address}");
        }
    }

    #[test]
    fn public_download_requests_contain_no_license_credentials() {
        let client = Client::new();
        for address in ["https://external.example/main.ipa", "https://accesshub-licensing.fejiofasj.workers.dev/catalog.json"] {
            let request = app_download_request(&client, Url::parse(address).unwrap()).unwrap().build().unwrap();
            assert!(!request.headers().contains_key(reqwest::header::AUTHORIZATION));
            assert!(!request.headers().contains_key("X-AccessHub-Device"));
        }
        let request = protected_download_request(
            Url::parse("https://accesshub-licensing.fejiofasj.workers.dev/v1/apps/12345678-1234-4234-8234-123456789abc").unwrap(),
            "dummy-token-for-test", "dummy-device-for-test"
        ).unwrap().build().unwrap();
        assert_eq!(request.headers()[reqwest::header::AUTHORIZATION], "Bearer dummy-token-for-test");
        assert_eq!(request.headers()["X-AccessHub-Device"], "dummy-device-for-test");
    }
}
