import { X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";

import { cancelInvite, invite, usePeers, type Invite } from "../core/link";
import { UsbSetup } from "../features/setup";
import { ease } from "../core/motion";
import { IconButton } from "./kit";
import { Orrery } from "./Orrery";
import { qrModules } from "./qr";
import "./Pair.css";

const RENEW_EARLY = 5;

function useInvite(): Invite | null {
  const [current, setCurrent] = useState<Invite | null>(null);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    let live = true;
    const renew = async () => {
      const next = await invite();
      if (!live) return;
      setCurrent(next);
      timer = setTimeout(renew, (next.expires_in_secs - RENEW_EARLY) * 1000);
    };
    void renew();
    return () => {
      live = false;
      clearTimeout(timer);
      void cancelInvite();
    };
  }, []);
  return current;
}

/** The dots of a QR code, with its three corner squares drawn as one shape each. */
function QrCode({ text }: { text: string }) {
  const { size, data } = useMemo(() => qrModules(text), [text]);
  const inCorner = (x: number, y: number) => ((x < 7 || x >= size - 7) && y < 7) || (x < 7 && y >= size - 7);
  const corners = [
    [0, 0],
    [size - 7, 0],
    [0, size - 7],
  ];
  return (
    <svg className="qr" viewBox={`-3 -3 ${size + 6} ${size + 6}`} role="img" aria-label="Pairing code to scan">
      <rect x="-3" y="-3" width={size + 6} height={size + 6} rx="3" fill="#fff" />
      {data.flatMap((row, y) =>
        row.map((dark, x) => dark && !inCorner(x, y) && <rect key={`${x},${y}`} x={x + 0.08} y={y + 0.08} width="0.84" height="0.84" rx="0.3" />),
      )}
      {corners.map(([x, y]) => (
        <g key={`${x},${y}`}>
          <rect x={x + 0.5} y={y + 0.5} width="6" height="6" rx="1.6" fill="none" stroke="#111" />
          <rect x={x + 2} y={y + 2} width="3" height="3" rx="0.8" />
        </g>
      ))}
    </svg>
  );
}

const STEPS = ["Install Orither on the phone and open it.", "Tap Link a computer.", "Scan the square, or type the code."];

/** Linking a phone: the computer waits at the centre for something to orbit it. */
export function Pair({ onClose }: { onClose?: () => void }) {
  const current = useInvite();
  const phones = usePeers() ?? [];
  const [first, second] = current?.code.split("-") ?? [];
  const known = useMemo(() => new Set(phones.map(({ id }) => id)), []); // eslint-disable-line react-hooks/exhaustive-deps
  const joined = phones.find(({ id }) => !known.has(id));

  useEffect(() => {
    if (!joined || !onClose) return;
    const timer = setTimeout(onClose, 900);
    return () => clearTimeout(timer);
  }, [joined, onClose]);

  return (
    <main className="pair">
      {onClose && (
        <span className="pair-close">
          <IconButton icon={X} label="Close" onClick={onClose} />
        </span>
      )}
      <section className="pair-words">
        <Orrery phones={joined ? [joined] : []} focused={joined?.id} size={168} waiting={!joined} />
        <h1>{joined ? `${joined.device.name} is linked.` : phones.length ? "Link another phone." : "Your phone, on this computer."}</h1>
        <UsbSetup />
        <p className="pair-or dim">Or, with Orither already on the phone:</p>
        <ol>
          {STEPS.map((step, index) => (
            <li key={step}>
              <span className="num">{index + 1}</span>
              {step}
            </li>
          ))}
        </ol>
        <p className="dim pair-usb">Some features also need the phone connected by USB once, with USB debugging on. Orither asks when it does.</p>
      </section>

      <section className="pair-card">
        <AnimatePresence mode="popLayout" initial={false}>
          {current && (
            <motion.div
              key={current.uri}
              className="pair-invite"
              initial={{ opacity: 0, scale: 0.97 }}
              animate={{ opacity: 1, scale: 1, transition: { duration: 0.3, ease: ease.out } }}
              exit={{ opacity: 0, transition: { duration: 0.2 } }}
            >
              <QrCode text={current.uri} />
              <p className="pair-code selectable num" aria-label={`Code ${current.code}`}>
                <span>{first}</span>
                <span>{second}</span>
              </p>
            </motion.div>
          )}
        </AnimatePresence>
        <p className="dim">The code changes every few minutes.</p>
      </section>
    </main>
  );
}
