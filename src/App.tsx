import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { accentInk, readableColor, readBranding } from "./branding";
import type { ResellerBranding } from "./branding";
import { AppleID } from "./AppleID";
import { Device } from "./Device";
import { SetupGuide } from "./SetupGuide";
import type { DeviceInfo } from "./Device";
import type { OperationUpdate } from "./components/operations";
import { usePlatform } from "./PlatformContext";
import "./App.css";
import "./Reseller.css";

type Release = { id: string; name: string; description: string; apps: { name: string }[]; ready: boolean };
type InstallState = { release: string; appCount: number; phase: "running" | "success" | "error"; started: string[]; completed: string[]; error: string | null };
type LicenseStatus = {
  active: boolean;
  message: string;
  deviceId: string;
  deviceName: string;
  license: { id: string; label: string; expiresAt: string | null; maxDevices: number; branding?: ResellerBranding | null } | null;
};

function readLicenseStatus(value: LicenseStatus): LicenseStatus {
  if (!value || typeof value.active !== "boolean" || typeof value.message !== "string") {
    throw new Error("Couldn’t verify your access. Please try again.");
  }
  return value;
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return "Something went wrong. Please try again.";
}

// Lock only setup controls so upstream authentication and pairing dialogs remain usable.
function SetupControls({ locked, children }: { locked: boolean; children: ReactNode }) {
  const container = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const controls = container.current?.querySelectorAll<HTMLElement>(".credentials-container");
    controls?.forEach((element) => { element.inert = locked; });
    return () => { controls?.forEach((element) => { element.inert = false; }); };
  });
  return <div ref={container} className={locked ? "setup-controls locked" : "setup-controls"}>{children}</div>;
}

function App() {
  const { platform } = usePlatform();
  const [loggedInAs, setLoggedInAs] = useState<string | null>(null);
  const [selectedDevice, setSelectedDevice] = useState<DeviceInfo | null>(null);
  const [noKeyringAvailable, setNoKeyringAvailable] = useState(true);
  const [releases, setReleases] = useState<Release[]>([]);
  const [releaseId, setReleaseId] = useState("");
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [installation, setInstallation] = useState<InstallState | null>(null);
  const [licenseStatus, setLicenseStatus] = useState<LicenseStatus | null>(null);
  const [everActivated, setEverActivated] = useState(false);
  const [licenseChecking, setLicenseChecking] = useState(true);
  const [licenseAction, setLicenseAction] = useState<"activate" | "deactivate" | null>(null);
  const [licenseKey, setLicenseKey] = useState("");
  const [showLicenseKey, setShowLicenseKey] = useState(false);
  const licenseRequest = useRef(0);
  const licenseActionRef = useRef<"activate" | "deactivate" | null>(null);
  const installing = useRef(false);
  const mounted = useRef(true);
  const catalogRequest = useRef(0);
  const catalogFetching = useRef(false);
  const unlistenOperation = useRef<(() => void) | null>(null);
  const selectedRelease = releases.find((release) => release.id === releaseId);
  const busy = installation?.phase === "running";
  const licensed = licenseStatus?.active === true;
  const branding = readBranding(licenseStatus?.license?.branding);
  const brandName = branding?.name || "AccessHub Reseller";
  const brandAccent = branding?.accent || "#f7a65b";
  const brandBackground = branding?.background || "#17131e";
  useEffect(() => {
    document.documentElement.style.setProperty("--reseller-accent", brandAccent);
    document.documentElement.style.setProperty("--reseller-bg", brandBackground);
    document.documentElement.style.setProperty("--reseller-ink", accentInk(brandAccent));
    for (const [property, preferred] of [["--reseller-text", "#fff8f1"], ["--reseller-muted", "#d5c4df"], ["--reseller-highlight", "#e2c6ef"], ["--reseller-link", "#bfdfca"], ["--reseller-status", "#d7f2df"]]) {
      document.documentElement.style.setProperty(property, readableColor(brandBackground, preferred));
    }
    document.documentElement.dataset.brandLight = readableColor(brandBackground) === "#000000" ? "true" : "false";

    document.title = brandName + " · Fictional demo";
    void getCurrentWindow().setTitle(brandName + " · DEMO — NOT VALID ID").catch(() => {});
  }, [brandName, brandAccent, brandBackground]);
  // Keep setup mounted after its first activation so license checks cannot
  // abandon an Apple login, pairing request, or signing callback in progress.
  const showInstaller = licensed || busy;

  useEffect(() => {
    mounted.current = true;
    invoke<boolean>("keyring_available")
      .then((available) => { if (mounted.current) setNoKeyringAvailable(!available); })
      .catch(() => { if (mounted.current) setNoKeyringAvailable(true); });
    return () => {
      mounted.current = false;
      catalogRequest.current += 1;
      catalogFetching.current = false;
      licenseRequest.current += 1;
      unlistenOperation.current?.();
      unlistenOperation.current = null;
    };
  }, []);

  const licenseFailed = useCallback((error: unknown) => {
    setLicenseStatus((previous) => ({
      active: false,
      message: errorMessage(error),
      deviceId: previous?.deviceId ?? "",
      deviceName: previous?.deviceName ?? "",
      license: previous?.license ?? null,
    }));
  }, []);

  const checkLicense = useCallback(async () => {
    if (installing.current || licenseActionRef.current) return;
    const request = ++licenseRequest.current;
    setLicenseChecking(true);
    try {
      const status = readLicenseStatus(await invoke<LicenseStatus>("license_status"));
      if (mounted.current && request === licenseRequest.current) setLicenseStatus(status);
    } catch (error) {
      if (mounted.current && request === licenseRequest.current) licenseFailed(error);
    } finally {
      if (mounted.current && request === licenseRequest.current) setLicenseChecking(false);
    }
  }, [licenseFailed]);

  useEffect(() => {
    void checkLicense();
    const timer = window.setInterval(() => { void checkLicense(); }, 60_000);
    return () => window.clearInterval(timer);
  }, [checkLicense]);

  useEffect(() => {
    if (licensed) setEverActivated(true);
  }, [licensed]);

  const changeLicense = async (action: "activate" | "deactivate") => {
    if (installing.current || licenseActionRef.current || (action === "activate" && !licenseKey.trim())) return;
    licenseActionRef.current = action;
    setLicenseAction(action);
    setLicenseChecking(false);
    const request = ++licenseRequest.current;
    try {
      const status = readLicenseStatus(await invoke<LicenseStatus>(
        action === "activate" ? "license_activate" : "license_deactivate",
        action === "activate" ? { key: licenseKey.trim() } : undefined,
      ));
      if (!mounted.current || request !== licenseRequest.current) return;
      setLicenseStatus(status);
      if (status.active || action === "deactivate") {
        setLicenseKey("");
        setShowLicenseKey(false);
      }
    } catch (error) {
      if (mounted.current && request === licenseRequest.current) licenseFailed(error);
    } finally {
      if (request === licenseRequest.current) {
        licenseActionRef.current = null;
        if (mounted.current) setLicenseAction(null);
      }
    }
  };

  const loadReleases = useCallback(async () => {
    if (installing.current || catalogFetching.current) return;
    catalogFetching.current = true;
    const request = ++catalogRequest.current;
    setCatalogLoading(true);
    setCatalogError(null);
    try {
      const catalog = await invoke<Release[]>("get_releases");
      if (!mounted.current || request !== catalogRequest.current) return;
      setReleases(catalog);
      setReleaseId((current) => catalog.some((release) => release.id === current) ? current : (catalog.find((release) => release.ready)?.id ?? catalog[0]?.id ?? ""));
    } catch (error) {
      if (!mounted.current || request !== catalogRequest.current) return;
      // Keep the chosen release through a temporary connection failure. It stays
      // unavailable until another successful check; native installation refetches too.
      setCatalogError(errorMessage(error));
    } finally {
      if (request === catalogRequest.current) {
        catalogFetching.current = false;
        if (mounted.current) setCatalogLoading(false);
      }
    }
  }, []);
  useEffect(() => {
    void loadReleases();
    const refresh = () => { void loadReleases(); };
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener("focus", refresh);
    return () => { window.clearInterval(timer); window.removeEventListener("focus", refresh); };
  }, [loadReleases]);

  const install = async () => {
    if (installing.current || catalogFetching.current || licenseActionRef.current || !licensed || catalogLoading || catalogError || !selectedRelease?.ready || !loggedInAs || !selectedDevice) return;
    installing.current = true;
    const release = selectedRelease;
    setInstallation({ release: release.name, appCount: release.apps.length, phase: "running", started: [], completed: [], error: null });
    let stopListening: (() => void) | undefined;
    try {
      stopListening = await listen<OperationUpdate>("operation_install_release", ({ payload }) => {
        if (!mounted.current) return;
        setInstallation((previous) => {
          if (!previous || previous.phase !== "running") return previous;
          if (payload.updateType === "started") return { ...previous, started: [...new Set([...previous.started, payload.stepId])] };
          if (payload.updateType === "finished") return { ...previous, completed: [...new Set([...previous.completed, payload.stepId])] };
          if (payload.updateType === "failed") return { ...previous, error: payload.extraDetails.message };
          return previous;
        });
      });
      if (!mounted.current) return;
      unlistenOperation.current = stopListening;
      await invoke("install_release_operation", { releaseId: release.id });
      if (mounted.current) setInstallation((previous) => previous && ({ ...previous, phase: "success", completed: ["prepare", "install"], error: null }));
    } catch (error) {
      if (mounted.current) setInstallation((previous) => previous && ({ ...previous, phase: "error", error: errorMessage(error) }));
    } finally {
      stopListening?.(); unlistenOperation.current = null; installing.current = false;
      if (mounted.current) { void checkLicense(); void loadReleases(); }
    }
  };

  const installHint = catalogLoading ? "Checking available releases…"
    : catalogError ? "Refresh the release list to check availability."
    : !selectedRelease ? "No release is available right now."
      : !selectedRelease.ready ? "The app files are still being prepared. Installation opens when this release is ready."
        : !loggedInAs ? "Sign in with your Apple Account to continue."
          : !selectedDevice ? "Connect and unlock your iPhone to continue."
            : `Ready to install ${selectedRelease.apps.length} ${selectedRelease.apps.length === 1 ? "app" : "apps"} on ${selectedDevice.name}.`;

  return (
    <main className="accesshub">
      <div className="reseller-demo-banner" role="note"><strong>DEMO — NOT VALID ID</strong><span>Fictional demonstration only · Not proof of identity or age</span></div>
      <header className="app-header">
        <div className="brand">
          {branding?.logoDataUrl ? <img className="reseller-brand-logo" src={branding.logoDataUrl} alt="Brand logo" /> : <div className="brand-mark" aria-hidden="true"><svg viewBox="0 0 32 32" fill="none"><path d="M7 23 16 7l9 16M11 17h10" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" /></svg></div>}
          <h1>{brandName}</h1>
        </div>
        <div className="header-status">{licensed && <span className="license-pill">Activated</span>}<span className="beta-pill">RESELLER DEMO · 1.1.0</span></div>
      </header>
      <aside className="reseller-welcome" aria-label="Fictional demo installer">
        <span aria-hidden="true">✦</span>
        <div><strong>{brandName}</strong><p>{branding?.tagline || "Your branded fictional demo, ready to set up."}</p></div>
      </aside>
      <div className="app-content">
        <SetupGuide platform={platform} />
        {!showInstaller && <section className="license-gate" aria-labelledby="license-title">
          <h2 id="license-title">Enter your demo access key</h2>
          <p className="license-intro">Activate your fictional demo access on this computer.</p>
          <form onSubmit={(event) => { event.preventDefault(); void changeLicense("activate"); }}>
            <label className="field-label" htmlFor="access-key">Access key</label>
            <div className="license-input-row">
              <input id="access-key" type={showLicenseKey ? "text" : "password"} value={licenseKey} onChange={(event) => setLicenseKey(event.target.value)} disabled={licenseAction !== null} autoComplete="off" autoCapitalize="none" spellCheck={false} placeholder="Paste your private demo key" aria-describedby="license-key-note" />
              <button type="button" className="text-button key-visibility" onClick={() => setShowLicenseKey((previous) => !previous)} disabled={licenseAction !== null} aria-label={showLicenseKey ? "Hide access key" : "Show access key"} aria-pressed={showLicenseKey}>{showLicenseKey ? "Hide" : "Show"}</button>
            </div>
            <p id="license-key-note" className="license-key-note">1 computer per key</p>
            <button type="submit" className="install-button" disabled={licenseChecking || licenseAction !== null || !licenseKey.trim()}>{licenseAction === "activate" ? "Activating…" : "Activate"}</button>
          </form>
          <div className="license-feedback" role="status" aria-live="polite">
            <p>{licenseChecking ? "Checking your access…" : licenseStatus?.message}</p>
            {!licenseChecking && <button type="button" className="text-button" disabled={licenseAction !== null} onClick={() => void checkLicense()}>Check again</button>}
          </div>
          <p className="license-disclosure">The demo licensing service records this computer’s name, a device identifier, app version and installation activity for licensing and support.</p>
        </section>}
        {(everActivated || licensed) && <div className="installer-view" hidden={!showInstaller}>
        {!licensed && <p className="license-warning" role="status">{licenseStatus?.message || "Your access needs to be checked again."} This installation’s prompts will stay open until it finishes.</p>}
        <section className="release-panel" aria-label="Choose a release">
          <div className="release-label-row"><label className="field-label" htmlFor="release">Release</label><button className="text-button" onClick={() => void loadReleases()} disabled={catalogLoading || busy} aria-label="Refresh available releases">Refresh</button></div>
          {catalogError ? <div className="notice error-notice" role="alert"><strong>Couldn’t load releases</strong><p>{catalogError}</p><button onClick={() => void loadReleases()}>Try again</button></div> : <>
            <select id="release" value={releaseId} onChange={(event) => { setReleaseId(event.target.value); setInstallation(null); }} disabled={catalogLoading || busy || releases.length === 0}>
              {releases.length === 0 && <option value="">{catalogLoading ? "Loading releases…" : "No releases available"}</option>}
              {releases.map((release) => <option key={release.id} value={release.id}>{release.name}</option>)}
            </select>
            {selectedRelease && <p className="release-description">{selectedRelease.description}</p>}
          </>}
        </section>
        <div className="setup-grid">
          <section className="setup-card" aria-label="Apple Account setup">
            <SetupControls locked={busy || !licensed}><AppleID loggedInAs={loggedInAs} setLoggedInAs={setLoggedInAs} noKeyringAvailable={noKeyringAvailable} /></SetupControls>
          </section>
          <section className="setup-card" aria-label="iPhone connection setup">
            <SetupControls locked={busy || !licensed}><Device selectedDevice={selectedDevice} setSelectedDevice={setSelectedDevice} /></SetupControls>
          </section>
        </div>
        <div className="install-area">
          <button className="install-button" disabled={busy || !licensed || licenseAction !== null || catalogLoading || Boolean(catalogError) || !selectedRelease?.ready || !loggedInAs || !selectedDevice} onClick={() => void install()}>
            {busy ? <><span className="small-spinner" aria-hidden="true" /> Installing…</> : selectedRelease?.ready ? "Install" : "Coming soon"}
          </button>
          <p className="install-hint">{busy ? "Keep your iPhone connected. Follow any Apple prompts." : installHint}</p>
        </div>
        </div>}
        {installation && <div className={`install-status ${installation.phase}`} role="status" aria-live="polite">
          <strong>{installation.phase === "success" ? "Installed" : installation.phase === "error" ? "Installation stopped" : "Installing your release…"}</strong>
          {installation.phase === "success" ? <p>{installation.appCount === 1 ? "Your app is" : `All ${installation.appCount} apps are`} installed. Open {installation.appCount === 1 ? "it" : "them"} on your iPhone.</p> : <>
            <ol className="install-steps">
              {[{ id: "prepare", text: "Check app files" }, { id: "install", text: "Sign and install" }].map((step) => <li key={step.id} className={installation.completed.includes(step.id) ? "complete" : installation.started.includes(step.id) ? "current" : ""}>
                <span aria-hidden="true">{installation.completed.includes(step.id) ? "✓" : installation.started.includes(step.id) && installation.phase === "running" ? <span className="small-spinner" /> : "○"}</span>{step.text}
              </li>)}
            </ol>
            {installation.error && <p className="installation-error">{installation.error}</p>}
          </>}
        </div>}
        {showInstaller && <details className="help">
          <summary>Help with setup</summary>
          <ul>
            <li>Connect and unlock your iPhone, then tap Trust if asked. AccessHub selects a connected phone automatically.</li>
            <li>Sign in with your own Apple Account. Apple may ask for a verification code.</li>
            <li>Enable Developer Mode in Settings → Privacy &amp; Security, then restart if asked. Trust the developer in Settings → General → VPN &amp; Device Management if iOS requests it.</li>
            <li>Free Apple accounts need regular app renewal. This loader installs only the fictional reseller demo app.</li>
            {platform === "windows" && <li>On Windows, install Apple’s iTunes device drivers if your iPhone isn’t detected.</li>}
          </ul>
          <div className="license-management">
            <strong>{licensed ? "Access activated" : "Access needs attention"}</strong>
            {licenseStatus?.deviceName && <p>{licenseStatus.deviceName}</p>}
            {licenseStatus?.license?.label && <p>{licenseStatus.license.label}</p>}
            {licenseStatus?.license?.expiresAt && <p>Expires {new Date(licenseStatus.license.expiresAt).toLocaleDateString()}</p>}
            <div className="license-management-actions">
              <button type="button" className="text-button" disabled={busy || licenseChecking || licenseAction !== null} onClick={() => void checkLicense()}>{licenseChecking ? "Checking…" : "Check access"}</button>
              <button type="button" className="text-button" disabled={busy || licenseAction !== null} onClick={() => void changeLicense("deactivate")}>{licenseAction === "deactivate" ? "Releasing…" : "Release this computer"}</button>
            </div>
            <p>Releasing this computer removes its activation so your key can be used on another computer.</p>
          </div>
        </details>}
        {branding?.supportSnapchat && <div className="reseller-support"><span>Help from {brandName}</span><button type="button" onClick={() => { void openUrl("https://www.snapchat.com/add/" + branding.supportSnapchat); }}>Contact @{branding.supportSnapchat} ↗</button></div>}
        <p className="reseller-footer-demo">DEMO — NOT VALID ID · This fictional experience cannot be used as proof of identity or age.</p>
      </div>
    </main>
  );
}
export default App;
