import { useEffect, useMemo, useSyncExternalStore } from "react";

import { call, focusedId, useCall, usePeers, useNotice } from "../../core/link";
import { setting, useSetting } from "../../core/settings";

/** A call on a phone, cellular or through an app such as WhatsApp. */
export interface Call {
  id: string;
  source: "cellular" | "app";
  app: string;
  appLabel: string;
  name: string;
  number: string | null;
  picture: boolean;
  video: boolean;
  state: "ringing" | "dialing" | "active" | "held" | "ended";
  since: number;
  can: { answer: boolean; decline: boolean; hangup: boolean };
}

/** A call, with the phone it is on. */
export interface PhoneCall {
  peer: string;
  call: Call;
}

/** Where a phone's call audio is heard, from the computer's side. */
export interface Audio {
  state: "unsupported" | "no-bluetooth" | "unknown-phone" | "unpaired" | "pairing" | "disconnected" | "connecting" | "connected";
  where: "computer" | "phone";
  detail: string | null;
}

let byPhone: Record<string, Call[]> = {};
let flat: PhoneCall[] = [];
const listeners = new Set<() => void>();

function put(peer: string, calls: Call[]) {
  byPhone = { ...byPhone, [peer]: calls };
  flat = Object.entries(byPhone).flatMap(([phone, list]) => list.map((entry) => ({ peer: phone, call: entry })));
  listeners.forEach((notify) => notify());
}

function subscribe(notify: () => void) {
  listeners.add(notify);
  return () => void listeners.delete(notify);
}

/** Keeps every connected phone's calls; mounted once, by the calls feature. */
export function useFollowCalls() {
  const peers = usePeers() ?? [];
  const online = peers.filter((peer) => peer.online).map((peer) => peer.id);
  const key = online.join(",");
  useEffect(() => {
    for (const peer of Object.keys(byPhone)) if (!online.includes(peer)) put(peer, []);
    for (const peer of online) {
      call<{ calls: Call[] }>("calls", "list", {}, peer).then(
        ({ calls }) => put(peer, calls ?? []),
        () => put(peer, []),
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useNotice<{ calls: Call[] }>("calls", "changed", ({ calls }, peer) => peer && put(peer, calls ?? []), { everyPhone: true });
}

/** Every call on every phone, ringing ones first. */
export function useCalls(): PhoneCall[] {
  const all = useSyncExternalStore(subscribe, () => flat);
  return useMemo(() => [...all].sort((a, b) => Number(b.call.state === "ringing") - Number(a.call.state === "ringing") || b.call.since - a.call.since), [all]);
}

/** The phones with a call ringing, for the orrery's ripple. */
export function useRingingPhones(): string[] {
  const all = useSyncExternalStore(subscribe, () => flat);
  return useMemo(() => [...new Set(all.filter(({ call }) => call.state === "ringing").map(({ peer }) => peer))], [all]);
}

export function act(entry: PhoneCall, action: "answer" | "decline" | "hangup") {
  return call("calls", action, { id: entry.call.id }, entry.peer);
}

export function describe({ call: entry }: PhoneCall): string {
  const kind = entry.video ? "video call" : "call";
  const via = entry.source === "cellular" ? (entry.video ? "Video call" : "Phone call") : `${entry.appLabel} ${kind}`;
  switch (entry.state) {
    case "ringing":
      return `${via}, ringing`;
    case "dialing":
      return `${via}, calling…`;
    case "held":
      return `${via}, on hold`;
    case "ended":
      return "Call ended";
    default:
      return via;
  }
}

/** A SIM a phone call can go out on. */
export interface Sim {
  id: string;
  label: string;
}

interface Sims {
  sims: Sim[];
  /** The SIM Android uses by itself, when one is set. */
  preferred: string | null;
}

/** The phone's SIMs, and the one calls from here go out on: the one picked here, else Android's, else the first. */
export function useSims(): { sims: Sim[]; sim: string | undefined; pick: (id: string) => void } {
  const phone = focusedId();
  const listed = useCall<Sims>("calls", "sims");
  const [picked, pick] = useSetting<string | null>(`calls.sim.${phone}`, null);
  const sims = listed.data?.sims ?? [];
  const sim = sims.find(({ id }) => id === picked)?.id ?? listed.data?.preferred ?? sims[0]?.id;
  return { sims, sim, pick };
}

/** Calls a number from the phone in focus, with its sound on this computer. */
export async function dialNumber(number: string, sim?: string) {
  const chosen = sim ?? setting<string | null>(`calls.sim.${focusedId()}`, null) ?? undefined;
  await call("calls", "route", { to: "computer" }).catch(() => {});
  await call("calls", "dial", chosen ? { number, sim: chosen } : { number });
}
