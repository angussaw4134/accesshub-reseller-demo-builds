import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "./components/Modal";
import type { AppError } from "./errors";
import "./Device.css";

export type DeviceInfo = {
  name: string;
  id: number;
  udid: string;
  connectionType: "USB" | "Network" | "Unknown";
  version: string;
};

function sameConnection(first: DeviceInfo | null, second: DeviceInfo): boolean {
  return first?.id === second.id && first.udid === second.udid;
}

function messageFor(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return typeof error === "string" ? error : "Couldn’t connect to your iPhone. Reconnect it and try again.";
}

// One phone can appear over both USB and Wi-Fi. Keep its selected connection,
// otherwise prefer USB and use the device identifier for a stable order.
function orderedDevices(available: DeviceInfo[], current: DeviceInfo | null): DeviceInfo[] {
  const priority = (device: DeviceInfo) => device.connectionType === "USB" ? 0 : device.connectionType === "Network" ? 1 : 2;
  const sorted = [...available].sort((first, second) => {
    if (sameConnection(current, first)) return -1;
    if (sameConnection(current, second)) return 1;
    return priority(first) - priority(second) || first.udid.localeCompare(second.udid) || first.id - second.id;
  });
  const seen = new Set<string>();
  return sorted.filter((device) => {
    if (seen.has(device.udid)) return false;
    seen.add(device.udid);
    return true;
  });
}

export const Device = ({ selectedDevice, setSelectedDevice, registerRefresh }: {
  selectedDevice: DeviceInfo | null;
  setSelectedDevice: (device: DeviceInfo | null) => void;
  registerRefresh?: (fn?: () => void) => void;
}) => {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [waitingToPair, setWaitingToPair] = useState<DeviceInfo | null>(null);
  const [showPairingModal, setShowPairingModal] = useState(false);
  const [scanning, setScanning] = useState(true);
  const [cancelling, setCancelling] = useState(false);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const currentDevice = useRef(selectedDevice);
  const publishDevice = useRef(setSelectedDevice);
  const mounted = useRef(true);
  const listingDevices = useRef(false);
  const pendingDevice = useRef<DeviceInfo | null>(null);
  const pairingRequestId = useRef(0);
  const pairingModalTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { currentDevice.current = selectedDevice; }, [selectedDevice]);
  useEffect(() => { publishDevice.current = setSelectedDevice; }, [setSelectedDevice]);

  const clearPairingModalTimer = useCallback(() => {
    if (pairingModalTimer.current !== null) clearTimeout(pairingModalTimer.current);
    pairingModalTimer.current = null;
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; clearPairingModalTimer(); };
  }, [clearPairingModalTimer]);

  const clearSelection = useCallback(() => {
    currentDevice.current = null;
    if (mounted.current) publishDevice.current(null);
  }, []);

  const selectDevice = useCallback(async (device: DeviceInfo) => {
    if (pendingDevice.current || sameConnection(currentDevice.current, device)) return;
    const requestId = ++pairingRequestId.current;
    pendingDevice.current = device;
    clearSelection();
    setWaitingToPair(device);
    setConnectionError(null);
    setCancelling(false);
    clearPairingModalTimer();
    pairingModalTimer.current = setTimeout(() => {
      if (mounted.current && pairingRequestId.current === requestId) setShowPairingModal(true);
    }, 600);

    try {
      // Clear the previous native selection before pairing, so failure cannot
      // leave the installer targeting an old phone.
      await invoke("set_selected_device", { device: null });
      if (!mounted.current || pairingRequestId.current !== requestId) return;
      await invoke("set_selected_device", { device });
      if (!mounted.current || pairingRequestId.current !== requestId) {
        await invoke("set_selected_device", { device: null });
        return;
      }
      currentDevice.current = device;
      publishDevice.current(device);
    } catch (error) {
      clearSelection();
      await invoke("set_selected_device", { device: null }).catch(() => {});
      if (mounted.current && pairingRequestId.current === requestId) setConnectionError(messageFor(error));
    } finally {
      pendingDevice.current = null;
      clearPairingModalTimer();
      if (mounted.current) {
        setWaitingToPair(null);
        setShowPairingModal(false);
        setCancelling(false);
      }
    }
  }, [clearPairingModalTimer, clearSelection]);

  const loadDevices = useCallback(async () => {
    if (listingDevices.current || pendingDevice.current) return;
    listingDevices.current = true;
    setScanning(true);
    setConnectionError(null);
    try {
      const results = await invoke<Array<{ Ok: DeviceInfo } | { Err: AppError }>>("list_devices");
      if (!mounted.current) return;
      const available = results.flatMap((result) => "Ok" in result ? [result.Ok] : []);
      const nextDevices = orderedDevices(available, currentDevice.current);
      setDevices(nextDevices);
      if (pendingDevice.current) return;
      const stillConnected = nextDevices.find((device) => sameConnection(currentDevice.current, device));
      if (stillConnected) {
        currentDevice.current = stillConnected;
        publishDevice.current(stillConnected);
        return;
      }
      // usbmuxd may give a reconnected phone a new connection ID. Prefer that
      // same physical phone over a different device that sorts first.
      const samePhone = nextDevices.find((device) => device.udid === currentDevice.current?.udid);
      clearSelection();
      if (nextDevices.length > 0) {
        await selectDevice(samePhone ?? nextDevices[0]);
      } else {
        await invoke("set_selected_device", { device: null });
        const failedDevice = results.find((result) => "Err" in result);
        if (failedDevice && "Err" in failedDevice) throw failedDevice.Err;
      }
    } catch (error) {
      if (!mounted.current) return;
      setDevices([]);
      clearSelection();
      await invoke("set_selected_device", { device: null }).catch(() => {});
      if (mounted.current) setConnectionError(messageFor(error));
    } finally {
      listingDevices.current = false;
      if (mounted.current) setScanning(false);
    }
  }, [clearSelection, selectDevice]);

  useEffect(() => { void loadDevices(); }, [loadDevices]);
  useEffect(() => {
    registerRefresh?.(loadDevices);
    return () => registerRefresh?.(undefined);
  }, [registerRefresh, loadDevices]);

  const cancelPairing = async () => {
    if (!pendingDevice.current || cancelling) return;
    pairingRequestId.current += 1;
    clearPairingModalTimer();
    setShowPairingModal(false);
    setCancelling(true);
    clearSelection();
    setConnectionError("Connection cancelled. Tap Retry when you’re ready.");
    await invoke("cancel_pairing").catch((error) => {
      if (mounted.current) setConnectionError(messageFor(error));
    });
  };

  const displayedDevice = waitingToPair ?? selectedDevice ?? devices[0];
  const working = scanning || waitingToPair !== null;
  const status = cancelling ? "Stopping connection…"
    : waitingToPair ? "Connecting automatically…"
      : selectedDevice ? `Connected · ${selectedDevice.connectionType === "Network" ? "Wi-Fi" : selectedDevice.connectionType}`
        : scanning ? "Looking for your iPhone…"
          : devices.length > 0 ? "Not connected. Tap Retry."
            : "Connect your iPhone by USB.";

  return <>
    <Modal isOpen={showPairingModal && waitingToPair !== null} close={() => void cancelPairing()} sizeFit>
      <div className="pairing-modal-content">
        <span className="device-spinner" aria-hidden="true" />
        <h2>Connect {waitingToPair?.name ?? "your iPhone"}</h2>
        <p>Unlock your iPhone, tap Trust, and enter your passcode if asked.</p>
        <button onClick={() => void cancelPairing()}>Cancel</button>
      </div>
    </Modal>
    <h2 style={{ marginTop: 0 }}>iPhone</h2>
    <div className="credentials-container device-controls">
      <div className="device-summary" role="status" aria-live="polite">
        <span className={working ? "device-spinner" : `device-status-dot${selectedDevice ? " connected" : ""}`} aria-hidden="true" />
        <div className="device-summary-text">
          <strong>{displayedDevice?.name ?? "Your iPhone"}</strong>
          <span>{status}</span>
        </div>
        <button className="device-refresh" onClick={() => void loadDevices()} disabled={working}>
          {connectionError || (!selectedDevice && devices.length > 0 && !working) ? "Retry" : "Refresh"}
        </button>
      </div>
      {devices.length > 1 && <label className="device-picker">Choose iPhone
        <select value={selectedDevice?.id ?? waitingToPair?.id ?? ""} disabled={working} onChange={(event) => {
          const device = devices.find((candidate) => candidate.id === Number(event.target.value));
          if (device) void selectDevice(device);
        }}>
          {!selectedDevice && !waitingToPair && <option value="">Choose a connected iPhone</option>}
          {devices.map((device) => <option value={device.id} key={device.udid}>{device.name} · {device.connectionType === "Network" ? "Wi-Fi" : device.connectionType}</option>)}
        </select>
      </label>}
      {connectionError && <p className="device-error" role="alert">{connectionError}</p>}
    </div>
  </>;
};
