import { Check, Copy, Search as SearchIcon, type LucideIcon } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { type ButtonHTMLAttributes, type ReactNode, type RefObject, useEffect, useRef, useState } from "react";

import { call, initialOf, usePeers, type PeerStatus, type Remote } from "../core/link";
import { timing } from "../core/motion";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement>;

export function Button({ kind, size, className, ...props }: ButtonProps & { kind?: "primary" | "danger" | "quiet"; size?: "large" }) {
  return <button className={className ? `button ${className}` : "button"} data-kind={kind} data-size={size} {...props} />;
}

export function IconButton({ icon: Icon, label, active, size = 18, ...props }: ButtonProps & { icon: LucideIcon; label: string; active?: boolean; size?: number }) {
  return (
    <button className="icon-button" data-active={active} aria-label={label} title={label} {...props}>
      <Icon size={size} />
    </button>
  );
}

/** The check replaces the copy icon in place, so the button never changes width. */
export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const copy = () => {
    void call("clipboard", "copy", { text });
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1400);
  };
  return (
    <Button onClick={copy} aria-label={copied ? "Copied" : label}>
      <span className="swap">
        <AnimatePresence initial={false} mode="popLayout">
          <motion.span
            key={copied ? "done" : "copy"}
            initial={{ opacity: 0, scale: 0.4, filter: "blur(3px)" }}
            animate={{ opacity: 1, scale: 1, filter: "blur(0px)" }}
            exit={{ opacity: 0, scale: 0.4, filter: "blur(3px)" }}
            transition={{ type: "spring", duration: 0.3, bounce: 0 }}
          >
            {copied ? <Check size={15} /> : <Copy size={15} />}
          </motion.span>
        </AnimatePresence>
      </span>
      {label}
    </Button>
  );
}

export function Empty({ icon: Icon, title, children }: { icon?: LucideIcon; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      {Icon && <Icon size={28} strokeWidth={1.5} />}
      {title && <h3>{title}</h3>}
      {children}
    </div>
  );
}

export function Avatar({ name, src, size = 40 }: { name: string; src?: string; size?: number }) {
  const [broken, setBroken] = useState(false);
  return (
    <span className="avatar" style={{ width: size, height: size, fontSize: size * 0.4 }}>
      {src && !broken ? <img src={src} alt="" onError={() => setBroken(true)} /> : name.trim().charAt(0).toUpperCase() || "?"}
    </span>
  );
}

/** A phone's mark: its initial on a small satellite, filled for the phone in focus. */
export function PhoneMark({ phone, focused, size = 20 }: { phone: PeerStatus; focused?: boolean; size?: number }) {
  const all = usePeers() ?? [];
  return (
    <span
      className="phone-mark"
      data-focused={focused}
      data-online={phone.online}
      style={{ width: size, height: size, fontSize: size * 0.48 }}
      title={phone.device.name}
      aria-label={phone.device.name}
    >
      {initialOf(phone, all)}
    </span>
  );
}

const shortTime = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const shortDate = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

/** "14:32" today, "3 Nov" any other day. */
export function when(milliseconds: number): string {
  const date = new Date(milliseconds);
  const today = date.toDateString() === new Date().toDateString();
  return (today ? shortTime : shortDate).format(date);
}

export function bytes(size: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  const unit = Math.min(Math.floor(Math.log(Math.max(size, 1)) / Math.log(1024)), units.length - 1);
  const value = size / 1024 ** unit;
  return `${value.toFixed(value < 10 && unit > 0 ? 1 : 0)} ${units[unit]}`;
}

/** "4:07", or "1:02:33" past the hour. */
export function clock(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const [hours, minutes, rest] = [Math.floor(whole / 3600), Math.floor((whole % 3600) / 60), whole % 60];
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours ? `${hours}:${pad(minutes)}:${pad(rest)}` : `${minutes}:${pad(rest)}`;
}

export function Toggle({ on, label, onChange, disabled }: { on: boolean; label: string; onChange: (on: boolean) => void; disabled?: boolean }) {
  return (
    <button
      className="toggle"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        onChange(!on);
      }}
    />
  );
}

export function Search({ value, onChange, placeholder = "Search", autoFocus }: { value: string; onChange: (value: string) => void; placeholder?: string; autoFocus?: boolean }) {
  return (
    <label className="search">
      <SearchIcon size={16} />
      <input className="field" type="search" placeholder={placeholder} value={value} autoFocus={autoFocus} spellCheck={false} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

/** Shows what came back from the phone, or why nothing did, without flashing a spinner for quick answers. */
export function Loaded<T>({ remote, icon, children }: { remote: Remote<T>; icon: LucideIcon; children: (data: T) => ReactNode }) {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setSlow(true), 300);
    return () => clearTimeout(timer);
  }, []);
  if (remote.data !== undefined) return children(remote.data);
  if (remote.error) {
    return (
      <Empty icon={icon} title="That didn't work">
        <p>{remote.error}</p>
        <Button onClick={remote.reload}>Try again</Button>
      </Empty>
    );
  }
  return <div className="empty">{slow && <span className="waiting" aria-label="Loading" />}</div>;
}

/**
 * A panel that opens from `anchor` and closes on Escape or a click elsewhere.
 * It grows out of its trigger rather than appearing from nowhere.
 */
export function Popover({
  open,
  onClose,
  anchor,
  align = "start",
  children,
  className,
}: {
  open: boolean;
  onClose: () => void;
  anchor: RefObject<HTMLElement | null>;
  align?: "start" | "end";
  children: ReactNode;
  className?: string;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<{ top: number; left?: number; right?: number }>({ top: 0 });

  useEffect(() => {
    if (!open || !anchor.current) return;
    const box = anchor.current.getBoundingClientRect();
    setAt(align === "start" ? { top: box.bottom + 8, left: box.left } : { top: box.bottom + 8, right: window.innerWidth - box.right });
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent) {
        if (event.key !== "Escape") return;
        event.stopPropagation();
      } else if (panel.current?.contains(event.target as Node) || anchor.current?.contains(event.target as Node)) {
        return;
      }
      onClose();
    };
    window.addEventListener("keydown", close, true);
    window.addEventListener("pointerdown", close, true);
    return () => {
      window.removeEventListener("keydown", close, true);
      window.removeEventListener("pointerdown", close, true);
    };
  }, [open, anchor, align, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          ref={panel}
          className={`popover ${className ?? ""}`}
          role="dialog"
          style={{ ...at, transformOrigin: align === "start" ? "top left" : "top right" }}
          initial={{ opacity: 0, scale: 0.96, y: -4 }}
          animate={{ opacity: 1, scale: 1, y: 0, transition: timing.pop }}
          exit={{ opacity: 0, scale: 0.98, y: -2, transition: timing.popOut }}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
