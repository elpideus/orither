import { BatteryCharging, BatteryFull, BatteryLow, BatteryMedium, ChevronDown, House, Settings, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";

import { dismiss, useActivities, type Activity } from "../core/island";
import { useCall, useNotice, usePeers, usePhone, select, type PeerStatus } from "../core/link";
import { ease, spring, timing } from "../core/motion";
import { NavContext } from "../core/nav";
import { features } from "../features";
import { useRingingPhones } from "../features/calls/state";
import { Home } from "./Home";
import { Button, IconButton, Popover } from "./kit";
import { Orrery, routeWords } from "./Orrery";
import { PhoneSheet } from "./PhoneSheet";
import "./Shell.css";

interface Battery {
  level: number;
  charging: boolean;
}

function batteryIcon({ level, charging }: Battery) {
  if (charging) return BatteryCharging;
  if (level > 70) return BatteryFull;
  return level > 25 ? BatteryMedium : BatteryLow;
}

function BatteryLevel() {
  const battery = useCall<Battery>("system", "battery");
  useNotice("system", "battery", battery.reload);
  if (!battery.data) return null;
  const Icon = batteryIcon(battery.data);
  return (
    <span className="identity-fact num" data-low={battery.data.level <= 15 && !battery.data.charging}>
      <Icon size={15} />
      {battery.data.level}%
    </span>
  );
}

/** The phone in focus by name, how it is reached, and the other phones one click away. */
function Identity({ phone, phones }: { phone: PeerStatus; phones: PeerStatus[] }) {
  const [open, setOpen] = useState(false);
  const name = useRef<HTMLButtonElement>(null);
  const others = phones.filter((other) => other.id !== phone.id);
  return (
    <div className="identity">
      <button ref={name} className="identity-name" aria-expanded={open} onClick={() => setOpen(!open)}>
        <AnimatePresence initial={false} mode="popLayout">
          <motion.span
            key={phone.id}
            initial={{ opacity: 0, y: 8, filter: "blur(2px)" }}
            animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
            exit={{ opacity: 0, y: -8, filter: "blur(2px)" }}
            transition={{ duration: 0.22, ease: ease.out }}
          >
            {phone.device.name}
          </motion.span>
        </AnimatePresence>
        <ChevronDown size={16} className="identity-chevron" />
      </button>
      <div className="identity-facts">
        <span className="identity-fact">
          <span className="identity-dot" data-online={phone.online} />
          {routeWords(phone)}
        </span>
        {phone.online && <BatteryLevel key={phone.id} />}
      </div>
      {others.length > 0 && (
        <div className="identity-others">
          {others.map((other) => (
            <button key={other.id} onClick={() => select(other.id)}>
              {other.device.name}
              <span className="dim"> · {other.online ? routeWords(other).toLowerCase() : "not connected"}</span>
            </button>
          ))}
        </div>
      )}
      <Popover open={open} onClose={() => setOpen(false)} anchor={name} className="phone-popover">
        <PhoneSheet phone={phone} onClose={() => setOpen(false)} />
      </Popover>
    </div>
  );
}

const tabs = [{ id: "home", title: "Home", icon: House }, ...features.filter((feature) => feature.Page && feature.tab)];

function Tabs({ page, go }: { page: string; go: (page: string) => void }) {
  return (
    <nav className="tabs" aria-label="Pages">
      {tabs.map(({ id, title, icon: Icon }) => (
        <button key={id} className="tab" aria-current={page === id ? "page" : undefined} onClick={() => go(id)} title={title}>
          {page === id && <motion.span className="tab-mark" layoutId="tab-mark" transition={spring.glide} />}
          <Icon size={17} />
          <span className="tab-label">{title}</span>
        </button>
      ))}
    </nav>
  );
}

function Toast({ activity }: { activity: Activity }) {
  const { icon: Icon, title, detail, progress, action, tone } = activity;
  return (
    <motion.li
      className="toast"
      data-tone={tone}
      layout
      initial={{ opacity: 0, y: 16, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.26, ease: ease.out } }}
      exit={{ opacity: 0, y: 8, scale: 0.98, transition: { duration: 0.16, ease: ease.out } }}
      transition={spring.glide}
    >
      <span className="toast-glyph">
        <Icon size={18} />
      </span>
      <div className="toast-words">
        <strong>{title}</strong>
        {detail && <p>{detail}</p>}
        {progress !== undefined && (
          <div className="toast-progress">
            <motion.span initial={false} animate={{ scaleX: progress }} transition={spring.snappy} />
          </div>
        )}
      </div>
      {action && (
        <Button
          kind="primary"
          onClick={() => {
            action.run();
            dismiss(activity.id);
          }}
        >
          {action.label}
        </Button>
      )}
      <IconButton icon={X} label="Dismiss" size={16} onClick={() => dismiss(activity.id)} />
    </motion.li>
  );
}

function Toasts() {
  const shown = useActivities().slice(-3);
  return (
    <ol className="toasts" aria-live="polite">
      <AnimatePresence initial={false}>
        {shown.map((activity) => (
          <Toast key={activity.id} activity={activity} />
        ))}
      </AnimatePresence>
    </ol>
  );
}

/** The window once a phone is linked: the orrery and tabs on top, one page below. */
export function Shell() {
  const [page, go] = useState("home");
  const phones = usePeers() ?? [];
  const phone = usePhone();
  const ringing = useRingingPhones();

  useEffect(() => {
    const back = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) go("home");
    };
    window.addEventListener("keydown", back);
    return () => window.removeEventListener("keydown", back);
  }, []);

  const Page = page === "home" ? Home : (features.find(({ id }) => id === page)?.Page ?? Home);

  return (
    <NavContext.Provider value={{ page, go }}>
      <div className="shell">
        <header className="top">
          <Orrery phones={phones} focused={phone?.id} ringing={ringing} />
          {phone && <Identity phone={phone} phones={phones} />}
          <span className="spacer" />
          <Tabs page={page} go={go} />
          <IconButton icon={Settings} label="Settings" active={page === "settings"} onClick={() => go(page === "settings" ? "home" : "settings")} />
        </header>
        <main className="stage">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.div
              key={page}
              className="page"
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0, transition: timing.page }}
              exit={{ opacity: 0, transition: { duration: 0.12, ease: ease.out } }}
            >
              <Page />
            </motion.div>
          </AnimatePresence>
        </main>
        <Toasts />
        {features.map(({ id, Ambient }) => Ambient && <Ambient key={id} />)}
      </div>
    </NavContext.Provider>
  );
}
