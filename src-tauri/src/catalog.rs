//! The webview chooses a release ID, never a file, URL, checksum or bundle ID.
//! This reseller loader accepts only the permanently marked fictional demo app.
use std::{
    collections::HashSet,
    fs::File,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State, Window};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;

use crate::{
    device::{DeviceInfoMutex, get_provider},
    error::AppError,
    operation::Operation,
    sideload::{SideloaderGuard, SideloaderMutex},
};

const EMBEDDED_CATALOG: &str = include_str!("../releases.json");
const TRUSTED_CATALOG_URL: &str = "https://accesshub-licensing.fejiofasj.workers.dev/reseller-catalog.json";
const RESELLER_APP_ID: &str = "app.accesshub.resellerdemo";
const RESELLER_RELEASE_ID: &str = "reseller-demo-ios-1";
const RESELLER_APP_URL: &str = "https://accesshub-licensing.fejiofasj.workers.dev/v1/apps/7e6d7e51-dfc2-4b38-97ce-52f1414d8ef0";
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_IPA_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PLIST_BYTES: u64 = 1024 * 1024;
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Deserialize)]
struct Catalog {
    schema_version: u32,
    #[serde(default)]
    remote_catalog_url: Option<String>,
    #[serde(default)]
    allowed_bundle_ids: Vec<String>,
    #[serde(default)]
    bundled_resources: Vec<BundledResource>,
    releases: Vec<Release>,
}

#[derive(Clone, Deserialize)]
struct BundledResource {
    id: String,
    file_name: String,
    bundle_id: String,
    sha256: String,
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Deserialize)]
struct Release {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    apps: Vec<ReleaseApp>,
}

#[derive(Clone, Deserialize)]
struct ReleaseApp {
    name: String,
    bundle_id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    resource: Option<String>,
    sha256: String,
}

#[derive(Serialize)]
pub struct PublicRelease {
    id: String,
    name: String,
    description: String,
    apps: Vec<PublicApp>,
    ready: bool,
}

#[derive(Serialize)]
struct PublicApp {
    name: String,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Misc(message.into())
}

fn install_error(mut error: AppError, app: &str, installed: &[&str]) -> AppError {
    let completed = if installed.is_empty() { "None".to_string() } else { installed.join(", ") };
    let context = if matches!(&error, AppError::InstallProhibited(_)) {
        if installed.is_empty() {
            format!("Could not install {app}. ")
        } else {
            format!("Could not install {app}. Successfully installed: {completed}. Completed apps were not removed. ")
        }
    } else {
        format!("Could not finish installing {app}. Successfully installed: {completed}. The failed app may need checking on your phone; completed apps were not removed. ")
    };
    // Retain upstream error categories so the existing help system remains useful.
    match &mut error {
        AppError::MaxApps(message)
        | AppError::NotEnoughAppIds(message)
        | AppError::DeviceComs(message)
        | AppError::InstallProhibited(message)
        | AppError::Underage(message)
        | AppError::AccountLocked(message)
        | AppError::Developer(message)
        | AppError::Auth(message)
        | AppError::Download(message)
        | AppError::RemotePairing(message)
        | AppError::Canceled(message)
        | AppError::OperationUpdate(message)
        | AppError::Anisette(message)
        | AppError::Keyring(message)
        | AppError::Misc(message)
        | AppError::HouseArrest(message, _)
        | AppError::LockdownPairing(message, _)
        | AppError::DeviceComsWithMessage(message, _)
        | AppError::Usbmuxd(message, _)
        | AppError::KeyringWithMessage(message, _)
        | AppError::Storage(message, _)
        | AppError::Filesystem(message, _) => *message = format!("{context}{message}"),
        AppError::NotLoggedIn | AppError::NoDeviceSelected => return invalid(format!("{context}{error}")),
    }
    error
}

fn https_url(value: &str) -> Result<Url, AppError> {
    let url = Url::parse(value).map_err(|_| invalid("A release download URL is invalid."))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("Release URLs must use HTTPS without embedded credentials or fragments."));
    }
    Ok(url)
}

fn client() -> Result<Client, AppError> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("Too many download redirects")
            } else if attempt.url().scheme() != "https"
                || !attempt.url().username().is_empty()
                || attempt.url().password().is_some()
            {
                attempt.error("Unsafe download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| AppError::Download(e.to_string()))
}

fn trusted_catalog_url(value: &str) -> Result<Url, AppError> {
    let url = https_url(value)?;
    if url.as_str() != TRUSTED_CATALOG_URL {
        return Err(invalid("The release catalog must come from the configured AccessHub service."));
    }
    Ok(url)
}

fn valid_bundle_id(value: &str) -> bool {
    value.len() <= 255
        && value.contains('.')
        && value.split('.').all(|part| !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
}

fn validate_allowed_bundle_ids(allowed: &[String]) -> Result<(), AppError> {
    let mut unique = HashSet::new();
    if allowed.len() > 100 || allowed.iter().any(|id| !valid_bundle_id(id) || !unique.insert(id)) {
        return Err(invalid("The release catalog has an invalid or oversized app identity list."));
    }
    Ok(())
}

fn parse_catalog(bytes: &[u8]) -> Result<Catalog, AppError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(invalid("The release catalog exceeds the size limit."));
    }
    let catalog: Catalog = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("The release catalog is invalid: {e}")))?;
    if catalog.schema_version != 1 || catalog.releases.len() > 100 || catalog.bundled_resources.len() > 100 {
        return Err(invalid("Unsupported release catalog format."));
    }
    validate_allowed_bundle_ids(&catalog.allowed_bundle_ids)?;
    let mut resource_ids = HashSet::new();
    for resource in &catalog.bundled_resources {
        if resource.id.is_empty() || !resource_ids.insert(&resource.id) || !safe_resource_name(&resource.file_name) {
            return Err(invalid("The release catalog contains an invalid bundled resource."));
        }
    }
    let mut ids = HashSet::new();
    for release in &catalog.releases {
        if release.id.is_empty()
            || release.id.len() > 80
            || !release.id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || !ids.insert(release.id.as_str())
            || release.name.trim().is_empty()
            || release.name.len() > 120
            || release.description.len() > 2000
            || release.apps.is_empty()
            || release.apps.len() > 2
            || release.apps.iter().any(|app| app.name.trim().is_empty() || app.name.len() > 120)
        {
            return Err(invalid("The release catalog contains an invalid or duplicate release."));
        }
    }
    Ok(catalog)
}

fn safe_resource_name(file_name: &str) -> bool {
    file_name.len() > 4
        && file_name.len() <= 120
        && file_name.ends_with(".ipa")
        && file_name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !file_name.contains("..")
        && !file_name.starts_with('.')
}

fn approved_resource<'a>(app: &ReleaseApp, resources: &'a [BundledResource]) -> Result<&'a BundledResource, AppError> {
    let id = app.resource.as_deref().ok_or_else(|| invalid("The app has no bundled resource."))?;
    let resource = resources.iter().find(|resource| resource.id == id)
        .ok_or_else(|| invalid("This bundled app is not approved by the installed AccessHub version."))?;
    if !app.url.is_empty()
        || !safe_resource_name(&resource.file_name)
        || resource.bundle_id != app.bundle_id
        || !resource.sha256.eq_ignore_ascii_case(&app.sha256)
    {
        return Err(invalid("The bundled app does not match its compiled authorization."));
    }
    Ok(resource)
}

/// A downloaded release may reuse the identical file compiled into this loader.
/// This is only a transport choice: the current owner catalog must still contain
/// and authorize the release before this helper is called. A changed checksum or
/// app identity keeps the ordinary verified-download path. Explicit resource IDs
/// retain their strict existing validation and never silently fall back.
fn resource_for_app<'a>(app: &ReleaseApp, resources: &'a [BundledResource]) -> Result<Option<&'a BundledResource>, AppError> {
    if app.resource.is_some() {
        return approved_resource(app, resources).map(Some);
    }
    https_url(&app.url)?;
    Ok(resources.iter().find(|resource| {
        safe_resource_name(&resource.file_name)
            && resource.bundle_id == app.bundle_id
            && resource.sha256.eq_ignore_ascii_case(&app.sha256)
    }))
}

fn validate_release(release: &Release, allowed: &[String], resources: &[BundledResource]) -> Result<(), AppError> {
    if !release.enabled {
        return Err(invalid("This release is not available for installation yet."));
    }
    let mut ids = HashSet::new();
    for app in &release.apps {
        if app.bundle_id.is_empty() || (app.url.is_empty() && app.resource.is_none()) || app.sha256.is_empty() {
            return Err(invalid(format!("{} is not available yet. The publisher still needs to add its app file.", app.name)));
        }
        if !allowed.contains(&app.bundle_id) || !ids.insert(app.bundle_id.as_str()) {
            return Err(invalid(format!("{} is not an approved, unique app in this installer.", app.name)));
        }
        if app.sha256.len() != 64 || !app.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid(format!("{} is missing a valid SHA-256 checksum.", app.name)));
        }
        if app.resource.is_some() {
            approved_resource(app, resources)?;
        } else {
            https_url(&app.url)?;
        }
    }
    Ok(())
}

async fn load_catalog() -> Result<Catalog, AppError> {
    let embedded = parse_catalog(EMBEDDED_CATALOG.as_bytes())?;
    let url = embedded.remote_catalog_url.as_deref()
        .ok_or_else(|| invalid("This demo loader requires its online reseller catalog."))?;
    let catalog_client = Client::builder().https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20))
        .build().map_err(|e| AppError::Download(e.to_string()))?;
    let mut response = catalog_client.get(trusted_catalog_url(url)?)
        .header(reqwest::header::CACHE_CONTROL, "no-cache").send().await
        .map_err(|e| AppError::Download(format!("Unable to load AccessHub releases: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::Download(format!("Unable to load AccessHub releases: HTTP {}", response.status())));
    }
    let fetched_from = response.url().clone();
    if response.content_length().is_some_and(|size| size > MAX_CATALOG_BYTES as u64) {
        return Err(invalid("The release catalog exceeds the size limit."));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| AppError::Download(e.to_string()))? {
        if bytes.len() + chunk.len() > MAX_CATALOG_BYTES {
            return Err(invalid("The release catalog exceeds the size limit."));
        }
        bytes.extend_from_slice(&chunk);
    }
    let remote = parse_catalog(&bytes)?;
    apply_compiled_authorization(remote, &embedded, &fetched_from)
}

fn apply_compiled_authorization(mut remote: Catalog, embedded: &Catalog, fetched_from: &Url) -> Result<Catalog, AppError> {
    let expected = embedded.remote_catalog_url.as_deref()
        .ok_or_else(|| invalid("This build does not accept remote release catalogs."))?;
    if fetched_from != &trusted_catalog_url(expected)? {
        return Err(invalid("The release catalog came from an untrusted address."));
    }
    if !embedded.bundled_resources.is_empty() || !embedded.releases.is_empty()
        || !remote.bundled_resources.is_empty()
        || remote.remote_catalog_url.as_deref().is_some_and(|url| url != TRUSTED_CATALOG_URL)
        || remote.allowed_bundle_ids.iter().any(|id| id != RESELLER_APP_ID)
        || remote.allowed_bundle_ids.len() > 1 || remote.releases.len() > 1 {
        return Err(invalid("This loader only accepts the AccessHub fictional reseller demo."));
    }
    for release in &remote.releases {
        if release.id != RESELLER_RELEASE_ID || release.apps.len() != 1 {
            return Err(invalid("This is not the approved fictional demo release."));
        }
        let app = &release.apps[0];
        if app.bundle_id != RESELLER_APP_ID || app.resource.is_some() || app.url != RESELLER_APP_URL
            || !remote.allowed_bundle_ids.iter().any(|id| id == RESELLER_APP_ID) {
            return Err(invalid("This app is not the approved fictional reseller demo."));
        }
    }
    // There are no compiled IPAs and no alternate catalog or local fallback.
    remote.bundled_resources.clear();
    remote.remote_catalog_url = embedded.remote_catalog_url.clone();
    Ok(remote)
}

#[tauri::command]
pub async fn get_releases() -> Result<Vec<PublicRelease>, AppError> {
    let catalog = load_catalog().await?;
    Ok(catalog.releases.into_iter().map(|release| {
        let ready = validate_release(&release, &catalog.allowed_bundle_ids, &catalog.bundled_resources).is_ok();
        PublicRelease {
            id: release.id,
            name: release.name,
            description: release.description,
            apps: release.apps.into_iter().map(|app| PublicApp { name: app.name }).collect(),
            ready,
        }
    }).collect())
}

fn verify_digest(actual: &[u8], expected: &str) -> Result<(), AppError> {
    let actual_hex: String = actual.iter().map(|byte| format!("{byte:02x}")).collect();
    if !actual_hex.eq_ignore_ascii_case(expected) {
        return Err(invalid("The app file failed its SHA-256 check. No apps were installed."));
    }
    Ok(())
}

fn validate_ipa(path: &Path, expected_bundle_id: &str) -> Result<(), AppError> {
    let file = File::open(path).map_err(|e| invalid(format!("Unable to read the app file: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| invalid(format!("The app file is not a valid IPA archive: {e}")))?;
    let mut main_plist = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| invalid(e.to_string()))?;
        if entry.enclosed_name().is_none() {
            return Err(invalid("The app archive contains an unsafe file path."));
        }
        let parts: Vec<_> = entry.name().split('/').collect();
        if parts.len() == 3 && parts[0] == "Payload" && parts[1].ends_with(".app") && parts[2] == "Info.plist" {
            if main_plist.replace(index).is_some() {
                return Err(invalid("The IPA must contain exactly one main application."));
            }
        }
    }
    let index = main_plist.ok_or_else(|| invalid("The IPA is missing its main application's Info.plist."))?;
    let entry = archive.by_index(index).map_err(|e| invalid(e.to_string()))?;
    if entry.size() > MAX_PLIST_BYTES {
        return Err(invalid("The app's Info.plist exceeds the size limit."));
    }
    let mut bytes = Vec::new();
    entry.take(MAX_PLIST_BYTES + 1).read_to_end(&mut bytes).map_err(|e| invalid(e.to_string()))?;
    if bytes.len() as u64 > MAX_PLIST_BYTES {
        return Err(invalid("The app's Info.plist exceeds the size limit."));
    }
    let plist = plist::Value::from_reader(Cursor::new(bytes)).map_err(|e| invalid(format!("Invalid app Info.plist: {e}")))?;
    let actual = plist.as_dictionary().and_then(|dict| dict.get("CFBundleIdentifier")).and_then(plist::Value::as_string);
    if actual != Some(expected_bundle_id) {
        return Err(invalid("The app's bundle ID does not match this approved release. No apps were installed."));
    }
    Ok(())
}

async fn download_app(client: &Client, app: &ReleaseApp, path: &Path) -> Result<(), AppError> {
    let mut response = crate::licensing::app_download_request(client, https_url(&app.url)?)?.send().await
        .map_err(|e| AppError::Download(e.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Download(format!("Unable to download this app: HTTP {}", response.status())));
    }
    if response.content_length().is_some_and(|size| size > MAX_IPA_BYTES) {
        return Err(invalid("The app download exceeds the 1 GB limit."));
    }
    let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(path).await
        .map_err(|e| invalid(format!("Unable to create an app download: {e}")))?;
    let mut size = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| AppError::Download(e.to_string()))? {
        size += chunk.len() as u64;
        if size > MAX_IPA_BYTES {
            return Err(invalid("The app download exceeds the 1 GB limit."));
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| invalid(format!("Unable to save an app download: {e}")))?;
    }
    file.flush().await.map_err(|e| invalid(e.to_string()))?;
    drop(file);
    verify_digest(&hash.finalize(), &app.sha256)?;
    let path = path.to_path_buf();
    let bundle_id = app.bundle_id.clone();
    tokio::task::spawn_blocking(move || validate_ipa(&path, &bundle_id)).await
        .map_err(|e| invalid(format!("Unable to verify the app: {e}")))?
}

struct PreparedRelease {
    _directory: TempDir,
    release: Release,
    paths: Vec<PathBuf>,
}

fn resource_path(resource_dir: &Path, resource: &BundledResource) -> Result<PathBuf, AppError> {
    if !safe_resource_name(&resource.file_name) {
        return Err(invalid("The bundled app filename is invalid."));
    }
    let directory = resource_dir.join("resources").canonicalize()
        .map_err(|e| invalid(format!("The installer's bundled apps are missing: {e}")))?;
    let path = directory.join(&resource.file_name).canonicalize()
        .map_err(|e| invalid(format!("The bundled app is missing. Reinstall AccessHub: {e}")))?;
    if !path.starts_with(&directory) || !path.is_file() {
        return Err(invalid("The bundled app is outside the installer's resource folder."));
    }
    Ok(path)
}

fn stage_bundled_app(source: &Path, destination: &Path, app: &ReleaseApp) -> Result<(), AppError> {
    let mut source = File::open(source).map_err(|e| invalid(format!("Unable to open the bundled app: {e}")))?;
    if source.metadata().map_err(|e| invalid(e.to_string()))?.len() > MAX_IPA_BYTES {
        return Err(invalid("The bundled app exceeds the 1 GB limit."));
    }
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(destination)
        .map_err(|e| invalid(format!("Unable to prepare the bundled app: {e}")))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let count = source.read(&mut buffer).map_err(|e| invalid(e.to_string()))?;
        if count == 0 { break; }
        size += count as u64;
        if size > MAX_IPA_BYTES {
            return Err(invalid("The bundled app exceeds the 1 GB limit."));
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(|e| invalid(e.to_string()))?;
    }
    output.flush().map_err(|e| invalid(e.to_string()))?;
    drop(output);
    verify_digest(&hash.finalize(), &app.sha256)?;
    validate_ipa(destination, &app.bundle_id)
}

async fn prepare(handle: &AppHandle, release_id: &str) -> Result<PreparedRelease, AppError> {
    let catalog = load_catalog().await?;
    let release = catalog.releases.into_iter().find(|release| release.id == release_id)
        .ok_or_else(|| invalid("That AccessHub release is no longer available. Refresh the release list."))?;
    validate_release(&release, &catalog.allowed_bundle_ids, &catalog.bundled_resources)?;
    let parent = handle.path().app_cache_dir().map_err(|e| invalid(e.to_string()))?;
    tokio::fs::create_dir_all(&parent).await.map_err(|e| invalid(e.to_string()))?;
    let directory = tempfile::Builder::new().prefix("accesshub-install-").tempdir_in(parent)
        .map_err(|e| invalid(format!("Unable to prepare the download folder: {e}")))?;
    let client = client()?;
    let mut paths = Vec::new();
    // Validate every file before signing or installing even the first app.
    for (index, app) in release.apps.iter().enumerate() {
        let path = directory.path().join(format!("app-{index}.ipa"));
        if let Some(resource) = resource_for_app(app, &catalog.bundled_resources)? {
            let resource_dir = handle.path().resource_dir().map_err(|e| invalid(e.to_string()))?;
            let source = resource_path(&resource_dir, resource)?;
            let destination = path.clone();
            let app = app.clone();
            tokio::task::spawn_blocking(move || stage_bundled_app(&source, &destination, &app)).await
                .map_err(|e| invalid(format!("Unable to prepare the bundled app: {e}")))??;
        } else {
            download_app(&client, app, &path).await?;
        }
        paths.push(path);
    }
    Ok(PreparedRelease { _directory: directory, release, paths })
}

struct InstallGuard;
impl InstallGuard {
    fn acquire() -> Result<Self, AppError> {
        INSTALLING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| invalid("An installation is already in progress."))?;
        Ok(Self)
    }
}
impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn install_release_operation(
    handle: AppHandle,
    window: Window,
    device_state: State<'_, DeviceInfoMutex>,
    sideloader_state: State<'_, SideloaderMutex>,
    release_id: String,
) -> Result<(), AppError> {
    let _install_guard = InstallGuard::acquire()?;
    let op = Operation::new("install_release".into(), &window);
    op.start("prepare")?;
    let license = op.fail_if_err("prepare", crate::licensing::authorize_install(&release_id).await)?;
    // Snapshot the selected phone once so both apps always target the same phone.
    let device = op.fail_if_err("prepare", device_state.lock()
        .map_err(|_| invalid("Device state is unavailable."))?
        .clone().ok_or(AppError::NoDeviceSelected))?;
    let logged_in = sideloader_state.lock().map_err(|_| invalid("Account state is unavailable."))?.is_some();
    if !logged_in {
        return op.fail("prepare", AppError::NotLoggedIn);
    }
    crate::licensing::record_event(&license, "install_started", &release_id, None).await;
    let prepared = match prepare(&handle, &release_id).await {
        Ok(prepared) => prepared,
        Err(error) => {
            crate::licensing::record_event(&license, "install_failed", &release_id, Some(error.as_ref())).await;
            return op.fail("prepare", error);
        }
    };
    let provider = match get_provider(&device.info).await {
        Ok(provider) => provider,
        Err(error) => {
            crate::licensing::record_event(&license, "install_failed", &release_id, Some(error.as_ref())).await;
            return op.fail("prepare", error);
        }
    };
    let mut sideloader = match SideloaderGuard::take(&sideloader_state) {
        Ok(sideloader) => sideloader,
        Err(error) => {
            crate::licensing::record_event(&license, "install_failed", &release_id, Some(error.as_ref())).await;
            return op.fail("prepare", error);
        }
    };
    op.move_on("prepare", "install")?;
    let mut installed: Vec<&str> = Vec::new();
    for (app, path) in prepared.release.apps.iter().zip(&prepared.paths) {
        if let Err(error) = crate::licensing::authorize_install(&release_id).await {
            crate::licensing::record_event(&license, "install_failed", &release_id, Some("license_denied")).await;
            return op.fail("install", install_error(error, &app.name, &installed));
        }
        // This is the same isideload engine and options used by upstream iLoader.
        let result = sideloader.get_mut().install_app(
            &provider,
            path.clone(),
            true,
            None::<fn(f32) -> std::future::Ready<()>>,
            Some(&device.pairing),
        ).await;
        if let Err(error) = result {
            let error = AppError::from(error);
            crate::licensing::record_event(&license, "install_failed", &release_id, Some(error.as_ref())).await;
            return op.fail("install", install_error(error, &app.name, &installed));
        }
        installed.push(&app.name);
        crate::licensing::record_event(&license, "install_app_done", &release_id, None).await;
    }
    crate::licensing::record_event(&license, "install_completed", &release_id, None).await;
    op.complete("install")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn release() -> Release {
        Release {
            id: "beta-1".into(), name: "Beta 1".into(), description: String::new(), enabled: true,
            apps: vec![ReleaseApp {
                name: "AccessHub".into(), bundle_id: "com.example.accesshub".into(),
                url: "https://example.com/accesshub.ipa".into(), resource: None, sha256: "a".repeat(64),
            }],
        }
    }

    #[test]
    fn unfinished_release_is_visible_but_not_installable() {
        let mut release = release();
        release.apps[0].url.clear();
        assert!(validate_release(&release, &["com.example.accesshub".into()], &[]).is_err());
    }

    #[test]
    fn embedded_catalog_has_no_installable_legacy_resources() {
        let catalog = parse_catalog(EMBEDDED_CATALOG.as_bytes()).unwrap();
        assert!(catalog.releases.is_empty());
        assert!(catalog.bundled_resources.is_empty());
        assert_eq!(catalog.allowed_bundle_ids, vec![RESELLER_APP_ID]);
        assert_eq!(catalog.remote_catalog_url.as_deref(), Some(TRUSTED_CATALOG_URL));
    }

    #[test]
    fn catalog_accepts_version_numbers_in_release_ids() {
        let fixture = br#"{"schema_version":1,"releases":[{"id":"accesshub-beta-1.0","name":"Beta 1.0","enabled":false,"apps":[{"name":"App","bundle_id":"","sha256":""}]}]}"#;
        let catalog = parse_catalog(fixture)
            .unwrap_or_else(|error| panic!("Versioned release ID was rejected: {error}"));
        assert_eq!(catalog.releases[0].id, "accesshub-beta-1.0");
    }

    #[test]
    fn partial_install_keeps_error_category_and_completed_app_names() {
        let error = install_error(AppError::MaxApps("No free slots".into()), "Companion", &["AccessHub"]);
        assert!(matches!(&error, AppError::MaxApps(_)));
        assert!(error.to_string().contains("Successfully installed: AccessHub"));
    }

    #[test]
    fn prohibited_install_retains_device_help_and_only_actual_completed_apps() {
        let make_error = || AppError::from(rootcause::Report::new(
            isideload::SideloadError::IdeviceError(idevice::IdeviceError::UnknownErrorType("InstallProhibited".into()))
        ).into_dynamic());
        let first = install_error(make_error(), "AccessHub Casino", &[]);
        assert!(matches!(&first, AppError::InstallProhibited(_)));
        assert!(first.to_string().starts_with("Could not install AccessHub Casino. The iPhone refused app installation (InstallProhibited)."));
        assert!(first.to_string().contains("Screen Time"));
        assert!(!first.to_string().contains("Successfully installed:"));
        let partial = install_error(make_error(), "Companion", &["AccessHub"]);
        assert!(matches!(&partial, AppError::InstallProhibited(_)));
        assert!(partial.to_string().contains("Successfully installed: AccessHub. Completed apps were not removed."));
        assert!(partial.to_string().contains("contact its administrator"));
    }

    #[test]
    fn release_must_use_approved_allowlist_https_and_valid_digest() {
        let approved = vec!["com.example.accesshub".into()];
        let mut release = release();
        assert!(validate_release(&release, &approved, &[]).is_ok());
        assert!(validate_release(&release, &[], &[]).is_err());
        release.apps[0].url = "http://example.com/app.ipa".into();
        assert!(validate_release(&release, &approved, &[]).is_err());
        release.apps[0].url = "https://example.com/app.ipa".into();
        release.apps[0].sha256 = "not-a-checksum".into();
        assert!(validate_release(&release, &approved, &[]).is_err());
    }

    #[test]
    fn cannot_install_same_bundle_twice_in_a_release() {
        let mut release = release();
        release.apps.push(release.apps[0].clone());
        assert!(validate_release(&release, &["com.example.accesshub".into()], &[]).is_err());
    }

    #[test]
    fn downloaded_bytes_must_match_published_checksum() {
        let hash = Sha256::digest(b"approved app");
        assert!(verify_digest(&hash, &format!("{hash:x}")).is_ok());
        assert!(verify_digest(&Sha256::digest(b"different app"), &format!("{hash:x}")).is_err());
    }

    #[test]
    fn ipa_must_contain_exactly_the_approved_main_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ipa");
        let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
        archive.start_file("Payload/AccessHub.app/Info.plist", zip::write::SimpleFileOptions::default()).unwrap();
        archive.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.accesshub</string></dict></plist>"#).unwrap();
        archive.finish().unwrap();
        assert!(validate_ipa(&path, "com.example.accesshub").is_ok());
        assert!(validate_ipa(&path, "com.example.unapproved").is_err());
    }

    #[test]
    fn ipa_with_multiple_main_apps_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multiple.ipa");
        let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
        for name in ["Payload/One.app/Info.plist", "Payload/Two.app/Info.plist"] {
            archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(b"unused").unwrap();
        }
        archive.finish().unwrap();
        assert!(validate_ipa(&path, "com.example.accesshub").is_err());
    }

    fn bundled_resource() -> BundledResource {
        BundledResource {
            id: "demo".into(), file_name: "Demo.ipa".into(),
            bundle_id: "com.example.accesshub".into(), sha256: "a".repeat(64),
        }
    }

    fn local_release() -> Release {
        let mut release = release();
        release.apps[0].url.clear();
        release.apps[0].resource = Some("demo".into());
        release
    }

    #[test]
    fn bundled_source_requires_exact_compiled_identity_and_hash() {
        let resources = vec![bundled_resource()];
        let approved = vec!["com.example.accesshub".into()];
        let mut release = local_release();
        assert!(validate_release(&release, &approved, &resources).is_ok());
        assert!(validate_release(&release, &approved, &[]).is_err());
        release.apps[0].resource = Some("unapproved-file.ipa".into());
        assert!(validate_release(&release, &approved, &resources).is_err());
        release.apps[0].resource = Some("demo".into());
        release.apps[0].sha256 = "b".repeat(64);
        assert!(validate_release(&release, &approved, &resources).is_err());
        release.apps[0].sha256 = "a".repeat(64);
        release.apps[0].bundle_id = "com.other.approved".into();
        assert!(validate_release(&release, &["com.other.approved".into()], &resources).is_err());
        release.apps[0].bundle_id = "com.example.accesshub".into();
        release.apps[0].url = "https://example.com/other.ipa".into();
        assert!(validate_release(&release, &approved, &resources).is_err());
    }

    #[test]
    fn bundled_source_rejects_paths_and_disabled_releases() {
        for name in ["../Demo.ipa", "..\\Demo.ipa", "/tmp/Demo.ipa", "C:\\Demo.ipa", "folder/Demo.ipa", "Demo.ipa:stream", ".hidden.ipa"] {
            assert!(!safe_resource_name(name), "accepted unsafe resource: {name}");
        }
        let mut release = local_release();
        release.enabled = false;
        assert!(validate_release(&release, &["com.example.accesshub".into()], &[bundled_resource()]).is_err());
    }

    #[test]
    fn approved_remote_app_prefers_only_an_identical_compiled_resource() {
        let resources = vec![bundled_resource()];
        let mut remote = release();
        validate_release(&remote, &["com.example.accesshub".into()], &resources).unwrap();
        assert_eq!(resource_for_app(&remote.apps[0], &resources).unwrap().unwrap().id, "demo");
        remote.apps[0].sha256 = "A".repeat(64);
        assert!(resource_for_app(&remote.apps[0], &resources).unwrap().is_some());
        remote.apps[0].sha256 = "b".repeat(64);
        assert!(resource_for_app(&remote.apps[0], &resources).unwrap().is_none());
        remote.apps[0].sha256 = "a".repeat(64);
        remote.apps[0].bundle_id = "com.other.approved".into();
        assert!(resource_for_app(&remote.apps[0], &resources).unwrap().is_none());
        remote.apps[0].bundle_id = "com.example.accesshub".into();
        let mut unsafe_resources = resources.clone();
        unsafe_resources[0].file_name = "../Demo.ipa".into();
        assert!(resource_for_app(&remote.apps[0], &unsafe_resources).unwrap().is_none());
        remote.apps[0].url = "http://example.com/app.ipa".into();
        assert!(resource_for_app(&remote.apps[0], &resources).is_err());
    }

    #[test]
    fn explicit_invalid_resource_never_falls_back_to_a_matching_file_or_url() {
        let resources = vec![bundled_resource()];
        let mut release = local_release();
        release.apps[0].resource = Some("unknown".into());
        assert!(resource_for_app(&release.apps[0], &resources).is_err());
        release.apps[0].resource = Some("demo".into());
        release.apps[0].url = "https://example.com/app.ipa".into();
        assert!(resource_for_app(&release.apps[0], &resources).is_err());
    }

    fn reseller_catalog() -> Catalog {
        Catalog { schema_version: 1, remote_catalog_url: None,
            allowed_bundle_ids: vec![RESELLER_APP_ID.into()], bundled_resources: vec![],
            releases: vec![Release { id: RESELLER_RELEASE_ID.into(), name: "Fictional reseller demo".into(),
                description: "DEMO — NOT VALID ID".into(), enabled: true,
                apps: vec![ReleaseApp { name: "Reseller demo".into(), bundle_id: RESELLER_APP_ID.into(),
                    url: RESELLER_APP_URL.into(), resource: None, sha256: "a".repeat(64) }] }] }
    }

    #[test]
    fn fixed_reseller_catalog_accepts_only_the_demo_app() {
        let embedded = parse_catalog(EMBEDDED_CATALOG.as_bytes()).unwrap();
        let trusted = apply_compiled_authorization(reseller_catalog(), &embedded, &Url::parse(TRUSTED_CATALOG_URL).unwrap()).unwrap();
        assert!(trusted.bundled_resources.is_empty());
        validate_release(&trusted.releases[0], &trusted.allowed_bundle_ids, &trusted.bundled_resources).unwrap();
        for id in ["dev.accesshub.sidestoretest", "com.example.NativeStorefront", "app.accesshub.casino"] {
            let mut remote = reseller_catalog();
            remote.allowed_bundle_ids = vec![id.into()];
            remote.releases[0].apps[0].bundle_id = id.into();
            assert!(apply_compiled_authorization(remote, &embedded, &Url::parse(TRUSTED_CATALOG_URL).unwrap()).is_err());
        }
    }

    #[test]
    fn reseller_catalog_rejects_unapproved_release_urls_resources_and_fallbacks() {
        let embedded = parse_catalog(EMBEDDED_CATALOG.as_bytes()).unwrap();
        let source = Url::parse(TRUSTED_CATALOG_URL).unwrap();
        let mut changed = reseller_catalog(); changed.releases[0].id = "original-release".into();
        assert!(apply_compiled_authorization(changed, &embedded, &source).is_err());
        let mut changed = reseller_catalog(); changed.releases[0].apps[0].url = "https://other.example/app.ipa".into();
        assert!(apply_compiled_authorization(changed, &embedded, &source).is_err());
        let mut changed = reseller_catalog(); changed.releases[0].apps[0].resource = Some("legacy".into());
        assert!(apply_compiled_authorization(changed, &embedded, &source).is_err());
        let mut changed = reseller_catalog(); changed.bundled_resources.push(bundled_resource());
        assert!(apply_compiled_authorization(changed, &embedded, &source).is_err());
        let mut changed = reseller_catalog(); changed.remote_catalog_url = Some("https://other.example/catalog.json".into());
        assert!(apply_compiled_authorization(changed, &embedded, &source).is_err());
        let mut fallback = embedded.clone(); fallback.remote_catalog_url = None;
        assert!(apply_compiled_authorization(reseller_catalog(), &fallback, &source).is_err());
        let mut legacy = embedded.clone(); legacy.bundled_resources.push(bundled_resource());
        assert!(apply_compiled_authorization(reseller_catalog(), &legacy, &source).is_err());
        for address in [
            "https://accesshub-licensing.fejiofasj.workers.dev/catalog.json",
            "https://accesshub-licensing.fejiofasj.workers.dev/reseller-catalog.json?other=1",
            "https://accesshub-licensing.fejiofasj.workers.dev.evil.example/reseller-catalog.json",
            "http://accesshub-licensing.fejiofasj.workers.dev/reseller-catalog.json",
        ] {
            assert!(trusted_catalog_url(address).is_err());
            assert!(apply_compiled_authorization(reseller_catalog(), &embedded, &Url::parse(address).unwrap()).is_err());
        }
    }

    #[test]
    fn empty_reseller_catalog_does_not_resurrect_any_local_release() {
        let embedded = parse_catalog(EMBEDDED_CATALOG.as_bytes()).unwrap();
        let mut remote = reseller_catalog(); remote.releases.clear(); remote.allowed_bundle_ids.clear();
        let trusted = apply_compiled_authorization(remote, &embedded, &Url::parse(TRUSTED_CATALOG_URL).unwrap()).unwrap();
        assert!(trusted.releases.is_empty()); assert!(trusted.bundled_resources.is_empty());
    }

    #[test]
    fn owner_allowlist_rejects_malformed_duplicate_and_unbounded_ids() {
        for id in ["", "app", "com..app", ".com.app", "com.app.", "com.*", "com.app/name", "com.äpp", "com.app_name", "com app.name"] {
            assert!(validate_allowed_bundle_ids(&[id.into()]).is_err(), "accepted {id}");
        }
        assert!(validate_allowed_bundle_ids(&[format!("com.{}", "a".repeat(252))]).is_err());
        assert!(validate_allowed_bundle_ids(&["com.example.app".into(), "com.example.app".into()]).is_err());
        let oversized = (0..101).map(|index| format!("com.example.app{index}")).collect::<Vec<_>>();
        assert!(validate_allowed_bundle_ids(&oversized).is_err());
        assert!(validate_allowed_bundle_ids(&["com.AccessHub.main-2".into(), "com.AccessHub.companion".into()]).is_ok());
    }

    #[test]
    fn catalog_releases_are_limited_to_one_or_two_apps() {
        let mut fixture = serde_json::json!({"schema_version":1,"releases":[{
            "id":"future-1.0","name":"Future release","enabled":false,
            "apps":[{"name":"Main","bundle_id":"","sha256":""},{"name":"Companion","bundle_id":"","sha256":""}]
        }]});
        assert!(parse_catalog(&serde_json::to_vec(&fixture).unwrap()).is_ok());
        fixture["releases"][0]["apps"].as_array_mut().unwrap().push(serde_json::json!({"name":"Third","bundle_id":"","sha256":""}));
        assert!(parse_catalog(&serde_json::to_vec(&fixture).unwrap()).is_err());
    }

    #[test]
    fn bundled_files_are_checked_after_copying_to_private_staging() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.ipa");
        let mut archive = zip::ZipWriter::new(File::create(&source).unwrap());
        archive.start_file("Payload/AccessHub.app/Info.plist", zip::write::SimpleFileOptions::default()).unwrap();
        archive.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.accesshub</string></dict></plist>"#).unwrap();
        archive.finish().unwrap();
        let hash = Sha256::digest(std::fs::read(&source).unwrap());
        let mut app = local_release().apps.remove(0);
        app.sha256 = format!("{hash:x}");
        assert!(stage_bundled_app(&source, &dir.path().join("valid.ipa"), &app).is_ok());
        assert!(stage_bundled_app(&source, &dir.path().join("valid.ipa"), &app).is_err());
        app.bundle_id = "com.unapproved.app".into();
        assert!(stage_bundled_app(&source, &dir.path().join("wrong-identity.ipa"), &app).is_err());
        app.bundle_id = "com.example.accesshub".into();
        app.sha256 = "b".repeat(64);
        assert!(stage_bundled_app(&source, &dir.path().join("tampered.ipa"), &app).is_err());
    }
}
