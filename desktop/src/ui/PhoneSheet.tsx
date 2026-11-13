import { Camera, Mic, MonitorSmartphone, Moon, Speaker, Wrench, type LucideIcon } from "lucide-react";
import { useState } from "react";

import { call, useCall, type PeerStatus } from "../core/link";
import { useSetting } from "../core/settings";
import { Button, bytes, Toggle } from "./kit";
import { routeWords } from "./Orrery";

interface About {
  model: string;
  android: string;
  storage: { free: number; total: number };
}

interface Device {
  id: "microphone" | "speaker" | "camera";
  on: boolean;
  /** Why it cannot be used on this computer, if it cannot. */
  blocked: string | null;
  /** Whether a one-time setup here would make it work. */
  setup?: boolean;
}

const DEVICES: Record<Device["id"], { title: string; icon: LucideIcon; detail: string }> = {
  microphone: { title: "Microphone", icon: Mic, detail: "“Orither Microphone”. Listens only while an app here records." },
  speaker: { title: "Speaker", icon: Speaker, detail: "“Orither Speaker”. Plays this computer's sound on the phone." },
  camera: { title: "Camera", icon: Camera, detail: "“Orither Camera”. Films only while an app here uses it." },
};

function Devices() {
  const devices = useCall<Device[]>("hardware", "devices");
  const [busy, setBusy] = useState<string>();
  const [problem, setProblem] = useState<string>();
  const set = (id: string, on: boolean) => void call("hardware", "set", { id, on }).finally(devices.reload);
  const setUp = async (id: string) => {
    setBusy(id);
    setProblem(undefined);
    await call("hardware", "setup", { id }).catch(setProblem);
    setBusy(undefined);
    devices.reload();
  };
  return (
    <section className="sheet-section">
      <h3>Lend to this computer</h3>
      {devices.data?.map(({ id, on, blocked, setup }) => {
        const { icon: Icon, title, detail } = DEVICES[id];
        return (
          <div key={id} className="option">
            <Icon size={18} />
            <strong>{title}</strong>
            <p>{blocked ?? detail}</p>
            {setup ? (
              <Button onClick={() => void setUp(id)} disabled={busy === id}>
                <Wrench size={15} />
                {busy === id ? "Setting up…" : "Set up"}
              </Button>
            ) : (
              <Toggle on={on} label={title} disabled={blocked !== null} onChange={(next) => set(id, next)} />
            )}
          </div>
        );
      })}
      {problem && <p className="sheet-problem">{problem}</p>}
    </section>
  );
}

/** What the phone in focus is, and what it can do for this computer. */
export function PhoneSheet({ phone, onClose }: { phone: PeerStatus; onClose: () => void }) {
  const about = useCall<About>("system", "about");
  const [dark, setDark] = useSetting("screen.off", true);
  const [error, setError] = useState<string>();
  const [starting, setStarting] = useState(false);

  const view = async () => {
    setStarting(true);
    setError(undefined);
    const opened = await call("screen", "view", { screenOff: dark }).then(
      () => true,
      (problem: string) => (setError(problem), false),
    );
    setStarting(false);
    if (opened) onClose();
  };

  const model = about.data && about.data.model !== phone.device.name ? `${about.data.model} · ` : "";
  const facts = about.data ? `${model}Android ${about.data.android} · ${bytes(about.data.storage.free)} free` : routeWords(phone);

  return (
    <div className="sheet">
      <header className="sheet-head">
        <strong>{phone.device.name}</strong>
        <p className="dim num">{facts}</p>
      </header>
      {phone.online ? (
        <>
          <section className="sheet-section">
            <div className="option">
              <Moon size={18} />
              <strong>Keep the phone's screen off</strong>
              <p>The picture shows here, and the phone stays dark.</p>
              <Toggle on={dark} label="Keep the phone's screen off" onChange={setDark} />
            </div>
            <Button kind="primary" size="large" onClick={() => void view()} disabled={starting} className="sheet-wide">
              <MonitorSmartphone size={17} />
              {starting ? "Opening…" : "View and control the screen"}
            </Button>
            {error && <p className="sheet-problem">{error}</p>}
          </section>
          <Devices />
        </>
      ) : (
        <p className="sheet-section dim">Orither reconnects by itself as soon as this phone has a connection.</p>
      )}
    </div>
  );
}
