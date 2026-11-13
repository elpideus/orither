import { animate, motion, useMotionValue, useReducedMotion, useTransform } from "motion/react";
import { useEffect, useState } from "react";

import { initialOf, select, type PeerStatus, type Route } from "../core/link";
import { spring } from "../core/motion";

/** Where the phone in focus sits: up and to the right of the computer, nearest the name beside it. */
const FRONT = -40;

/** How far out each way of reaching a phone puts it, as a share of the orrery's radius. */
const ORBIT: Record<Route | "away", number> = { local: 0.42, nearby: 0.64, internet: 0.86, relay: 0.86, away: 0.86 };
const RINGS = [0.42, 0.64, 0.86];

export const ROUTE_WORDS: Record<Route, string> = {
  local: "Same Wi-Fi",
  nearby: "Bluetooth",
  internet: "Over the internet",
  relay: "Over the internet, relayed",
};

export function routeWords(phone: PeerStatus): string {
  if (!phone.online) return "Not connected";
  return phone.route ? ROUTE_WORDS[phone.route] : "Connecting";
}

/** The shortest way round from one angle to another, so a satellite never laps the ring. */
function nearest(from: number, to: number): number {
  let target = to;
  while (target - from > 180) target -= 360;
  while (target - from < -180) target += 360;
  return target;
}

function Satellite({
  phone,
  label,
  angle,
  orbit,
  focused,
  ringing,
  size,
  satellite,
  onPick,
}: {
  phone: PeerStatus;
  label: string;
  angle: number;
  orbit: number;
  focused: boolean;
  ringing: boolean;
  size: number;
  satellite: number;
  onPick?: () => void;
}) {
  const reduce = useReducedMotion();
  const turn = useMotionValue(angle);
  const reach = useMotionValue(orbit);
  const [hover, setHover] = useState(false);

  useEffect(() => {
    const target = nearest(turn.get(), angle);
    const controls = animate(turn, target, reduce ? { duration: 0 } : spring.glide);
    return () => controls.stop();
  }, [angle, turn, reduce]);

  useEffect(() => {
    const controls = animate(reach, orbit, reduce ? { duration: 0 } : spring.settle);
    return () => controls.stop();
  }, [orbit, reach, reduce]);

  const middle = size / 2;
  const transform = useTransform(() => {
    const radians = (turn.get() * Math.PI) / 180;
    const distance = reach.get() * middle;
    const x = middle + distance * Math.cos(radians) - satellite / 2;
    const y = middle + distance * Math.sin(radians) - satellite / 2;
    return `translate(${x}px, ${y}px)`;
  });

  return (
    <motion.button
      className="satellite"
      data-focused={focused}
      data-online={phone.online}
      data-ringing={ringing}
      style={{ transform, width: satellite, height: satellite, fontSize: satellite * 0.46 }}
      aria-label={`${phone.device.name}, ${routeWords(phone)}${focused ? ", in focus" : ""}`}
      aria-pressed={focused}
      onClick={onPick}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onFocus={() => setHover(true)}
      onBlur={() => setHover(false)}
    >
      {label}
      {hover && !focused && (
        <span className="tip satellite-tip">
          {phone.device.name}
          <span className="dim"> · {routeWords(phone)}</span>
        </span>
      )}
    </motion.button>
  );
}

/**
 * The computer at the centre, and each phone orbiting it at the distance of its
 * route. The phone in focus swings round to the front; picking another one is
 * how the window changes phone.
 */
export function Orrery({
  phones,
  focused,
  ringing = [],
  size = 132,
  waiting = false,
}: {
  phones: PeerStatus[];
  focused: string | undefined;
  ringing?: string[];
  size?: number;
  /** No phone yet: a faint satellite circles where the first one will be. */
  waiting?: boolean;
}) {
  const satellite = Math.round(size * 0.21);
  const start = Math.max(0, phones.findIndex((phone) => phone.id === focused));
  const placed = phones.map((phone, index) => {
    const slot = (index - start + phones.length) % phones.length;
    return { phone, angle: FRONT + (360 / phones.length) * slot };
  });
  const occupied = new Set(phones.filter((phone) => phone.online).map((phone) => ORBIT[phone.route ?? "away"]));

  return (
    <div className="orrery" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        {RINGS.map((ring) => (
          <circle key={ring} className="orrery-ring" data-used={occupied.has(ring)} cx={size / 2} cy={size / 2} r={(ring * size) / 2} />
        ))}
        <rect className="orrery-computer" x={size / 2 - size * 0.065} y={size / 2 - size * 0.065} width={size * 0.13} height={size * 0.13} rx={size * 0.035} />
      </svg>
      {waiting && <span className="orrery-waiting" style={{ width: satellite, height: satellite, ["--reach" as string]: `${(ORBIT.local * size) / 2}px` }} />}
      {placed.map(({ phone, angle }) => (
        <Satellite
          key={phone.id}
          phone={phone}
          label={initialOf(phone, phones)}
          angle={angle}
          orbit={ORBIT[phone.online ? (phone.route ?? "away") : "away"]}
          focused={phone.id === focused}
          ringing={ringing.includes(phone.id)}
          size={size}
          satellite={satellite}
          onPick={phones.length > 1 ? () => select(phone.id) : undefined}
        />
      ))}
    </div>
  );
}
