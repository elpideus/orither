import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { Bell, MousePointer2, Plus, Power, Settings as Gear } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";

import type { Feature } from "../../core/feature";
import { call, select, unpair, useNotice, usePeers, usePhone, type PeerStatus } from "../../core/link";
import { ease, timing } from "../../core/motion";
import { Button, PhoneMark, Toggle, when } from "../../ui/kit";
import { routeWords } from "../../ui/Orrery";
import { Pair } from "../../ui/Pair";
import { UpdatesPane, useUpdates } from "../updates";
import "./settings.css";

function Autostart() {
  const [on, setOn] = useState(false);
  useEffect(() => void isEnabled().then(setOn, () => {}), []);
  return (
    <div className="option">
      <Power size={18} />
      <strong>Start with this computer</strong>
      <p>Orither waits in the tray, so your phones stay linked.</p>
      <Toggle on={on} label="Start with this computer" onChange={(next) => void (next ? enable() : disable()).then(() => setOn(next))} />
    </div>
  );
}

function PhoneNotifications() {
  const [on, setOn] = useState(true);
  useEffect(() => void call<{ enabled: boolean }>("notifications", "state").then((state) => setOn(state.enabled), () => {}), []);
  return (
    <div className="option">
      <Bell size={18} />
      <strong>Show phone notifications as this computer's</strong>
      <p>They pop up like any other, and a click opens the app in a window.</p>
      <Toggle on={on} label="Show phone notifications" onChange={(next) => void call("notifications", "enable", { on: next }).then(() => setOn(next))} />
    </div>
  );
}

/** `asks` is false where the computer lets the phone in without asking, as Windows does. */
type RemoteState = { supported: boolean; active: boolean; asks?: boolean };

function RemoteControl() {
  const [state, setState] = useState<RemoteState>();
  useEffect(() => void call<RemoteState>("input", "state").then(setState, () => {}), []);
  useNotice<RemoteState>("input", "state", setState, { everyPhone: true });
  if (!state) return null;
  return (
    <div className="option">
      <MousePointer2 size={18} />
      <strong>Trackpad and keyboard from the phone</strong>
      <p>
        {!state.supported
          ? "This desktop doesn't let other programs control it yet."
          : state.active
            ? "A phone is controlling this computer now."
            : state.asks === false
              ? "Open Trackpad in Orither on the phone. Programs run as administrator ignore it."
              : "Open Trackpad in Orither on the phone. This computer asks once whether to allow it."}
      </p>
      <span className="settings-state" data-on={state.active}>
        {state.active ? "In use" : "Ready"}
      </span>
    </div>
  );
}

function PhoneRow({ phone, focused }: { phone: PeerStatus; focused: boolean }) {
  const [armed, setArmed] = useState(false);
  return (
    <motion.li className="settings-phone" layout="position" exit={{ opacity: 0, transition: timing.popOut }}>
      <PhoneMark phone={phone} focused={focused} size={30} />
      <div>
        <strong>{phone.device.name}</strong>
        <p className="dim">
          {routeWords(phone)} · linked {when(phone.paired_at * 1000)}
        </p>
      </div>
      <div className="settings-phone-actions">
        {!focused && (
          <Button kind="quiet" onClick={() => select(phone.id)}>
            Show
          </Button>
        )}
        {armed ? (
          <Button kind="danger" onClick={() => void unpair(phone.id)} onBlur={() => setArmed(false)} autoFocus>
            Unlink {phone.device.name}
          </Button>
        ) : (
          <Button onClick={() => setArmed(true)}>Unlink</Button>
        )}
      </div>
    </motion.li>
  );
}

function Page() {
  const phones = usePeers() ?? [];
  const focused = usePhone();
  const [linking, setLinking] = useState(false);
  const version = useUpdates()?.current;
  return (
    <div className="settings">
      <section className="pane settings-group">
        <div className="pane-head">
          <h2>Phones</h2>
          <span className="spacer" />
          <Button onClick={() => setLinking(true)}>
            <Plus size={15} />
            Link another phone
          </Button>
        </div>
        <ul className="settings-phones">
          <AnimatePresence initial={false}>
            {phones.map((phone) => (
              <PhoneRow key={phone.id} phone={phone} focused={phone.id === focused?.id} />
            ))}
          </AnimatePresence>
        </ul>
      </section>
      <section className="pane settings-group">
        <div className="pane-head">
          <h2>This computer</h2>
        </div>
        <div className="settings-options">
          <PhoneNotifications />
          <RemoteControl />
          <Autostart />
        </div>
      </section>
      <UpdatesPane />
      <p className="settings-about dim">{version ? `Orither ${version}, free` : "Free"} software under the GNU General Public License, version 3 or later.</p>
      <AnimatePresence>
        {linking && (
          <motion.div
            className="settings-pair"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1, transition: { duration: 0.2, ease: ease.out } }}
            exit={{ opacity: 0, transition: { duration: 0.15, ease: ease.out } }}
          >
            <Pair onClose={() => setLinking(false)} />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export const settings: Feature = { id: "settings", title: "Settings", icon: Gear, Page };
