import { CircleCheck, Smartphone, TriangleAlert, Usb } from "lucide-react";
import { useEffect, useState } from "react";

import type { Feature } from "../../core/feature";
import { announce, dismiss } from "../../core/island";
import { call, useNotice } from "../../core/link";
import { Button } from "../../ui/kit";
import "./setup.css";

const ID = "setup";
const POLL_MS = 2000;
const DONE_LASTS = 12_000;

interface Device {
  serial: string;
  model: string;
  state: "ready" | "unauthorized" | "offline";
  app: string | null;
}

type Step = "download" | "install" | "confirm" | "link" | "helper" | "allow";

const STEP_WORDS: Record<Step, string> = {
  download: "Downloading the Android app…",
  install: "Installing Orither on the phone…",
  confirm: "Check your phone and confirm the install. If Play Protect asks, let it scan the app, then install.",
  link: "Linking the phone to this computer…",
  helper: "Starting Orither's helper on the phone…",
  allow: "Turning everything on…",
};

type Progress = { step: Step } | { done: string[] } | { error: string };

let progress: Record<string, Progress> = {};
const listeners = new Set<() => void>();

function record(serial: string, next: Progress) {
  progress = { ...progress, [serial]: next };
  listeners.forEach((notify) => notify());
}

/** Setups under way or finished, by serial, kept while the window lives so they survive moving between screens. */
function useProgress(): Record<string, Progress> {
  const [current, setCurrent] = useState(progress);
  useEffect(() => {
    const update = () => setCurrent(progress);
    listeners.add(update);
    return () => void listeners.delete(update);
  }, []);
  return current;
}

/** Keeps the progress the computer reports, wherever setup is shown. */
function useRecording() {
  useNotice<{ serial: string; step: Step }>(ID, "progress", ({ serial, step }) => record(serial, { step }));
  useNotice<{ serial: string; missing: string[] }>(ID, "done", ({ serial, missing }) => record(serial, { done: missing }));
  useNotice<{ serial: string; error: string }>(ID, "failed", ({ serial, error }) => record(serial, { error }));
}

function useDevices(): { adb: boolean; devices: Device[] } | null {
  const [found, setFound] = useState<{ adb: boolean; devices: Device[] } | null>(null);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const look = async () => {
      const next = await call<{ adb: boolean; devices: Device[] }>(ID, "devices").catch(() => null);
      if (!live) return;
      if (next) setFound(next);
      timer = setTimeout(look, POLL_MS);
    };
    void look();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, []);
  return found;
}

function DeviceRow({ device, invite }: { device: Device; invite?: string }) {
  const state = useProgress()[device.serial];
  const start = () => {
    record(device.serial, { step: "install" });
    void call(ID, "start", { serial: device.serial, invite }).catch((error: string) => record(device.serial, { error: String(error) }));
  };
  let words: string;
  let action = null;
  if (device.state === "unauthorized") {
    words = "Check your phone: allow USB debugging, with Always allow from this computer ticked.";
  } else if (device.state === "offline") {
    words = "Not ready yet. Unplug the phone and plug it in again.";
  } else if (!state) {
    words = device.app ? `Orither ${device.app} is on it.` : "Orither is not on it yet.";
    action = (
      <Button kind="primary" onClick={start}>
        Set up
      </Button>
    );
  } else if ("step" in state) {
    words = STEP_WORDS[state.step];
  } else if ("done" in state) {
    words = state.done.length ? `Set up. Android kept these off: ${state.done.join(", ")}. Allow them in Orither on the phone.` : "Set up, with everything on.";
  } else {
    words = state.error;
    action = <Button onClick={start}>Try again</Button>;
  }
  const busy = state && "step" in state;
  return (
    <li className="usb-device" data-busy={busy || undefined} data-problem={(state && "error" in state) || undefined}>
      <Smartphone size={20} />
      <div>
        <strong>{device.model}</strong>
        <p>{words}</p>
      </div>
      {action}
    </li>
  );
}

/** Setting up a phone over a USB cable: Orither installs its app, links it and turns everything on. */
export function UsbSetup({ invite }: { invite?: string }) {
  useRecording();
  const found = useDevices();
  if (!found || !found.adb) return null;
  const { devices } = found;
  return (
    <div className="usb-setup">
      <div className="usb-head">
        <Usb size={18} />
        <strong>Set up over USB</strong>
        <span className="usb-easiest">Easiest</span>
      </div>
      {devices.length === 0 ? (
        <ol className="usb-steps">
          <li>
            On the phone, turn on <b>USB debugging</b>: tap <b>Build number</b> seven times in <b>Settings › About phone</b>, then find
            USB debugging in <b>Developer options</b>.
          </li>
          <li>Connect the phone to this computer with a USB cable, and allow this computer when the phone asks.</li>
        </ol>
      ) : (
        <ul className="usb-devices">
          {devices.map((device) => (
            <DeviceRow key={device.serial} device={device} invite={invite} />
          ))}
        </ul>
      )}
      <p className="dim usb-note">Orither installs its app on the phone, links it and turns everything on. Nothing to download on the phone.</p>
    </div>
  );
}

/** Keeps the setup's progress, and shows the steps that happen after the phone links as toasts in the main window. */
function Ambient() {
  useRecording();
  useNotice<{ serial: string; step: Step }>(ID, "progress", ({ serial, step }) => {
    if (step === "helper" || step === "allow") announce({ id: `${ID}-${serial}`, icon: Usb, title: "Setting up your phone", detail: STEP_WORDS[step] });
  });
  useNotice<{ serial: string; missing: string[] }>(ID, "done", ({ serial, missing }) => {
    announce({
      id: `${ID}-${serial}`,
      icon: CircleCheck,
      title: "Your phone is set up",
      detail: missing.length ? `Android kept these off: ${missing.join(", ")}. Allow them in Orither on the phone.` : "Everything is on.",
      lasts: DONE_LASTS,
    });
  });
  useNotice<{ serial: string; error: string }>(ID, "failed", ({ serial, error }) => {
    announce({ id: `${ID}-${serial}`, icon: TriangleAlert, title: "Setting up your phone stopped", detail: error, tone: "problem" });
  });
  useEffect(() => () => Object.keys(progress).forEach((serial) => dismiss(`${ID}-${serial}`)), []);
  return null;
}

export const setupFeature: Feature = { id: ID, title: "Set up", icon: Usb, Ambient };
