import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

export type Route = "local" | "nearby" | "internet" | "relay";

export interface PeerStatus {
  id: string;
  device: { name: string; kind: "phone" | "desktop" };
  paired_at: number;
  online: boolean;
  route: Route | null;
}

export interface Invite {
  code: string;
  uri: string;
  expires_in_secs: number;
}

/** A tiny store that React components can subscribe to. */
function store<T>(initial: T) {
  let value = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => value,
    set(next: T) {
      value = next;
      listeners.forEach((notify) => notify());
    },
    subscribe(notify: () => void) {
      listeners.add(notify);
      return () => void listeners.delete(notify);
    },
  };
}

const peers = store<PeerStatus[] | null>(null);
/** The phone the UI shows, as chosen; null until the computer says which one it remembers. */
const chosen = store<string | null | undefined>(undefined);

let watching = false;
function watch() {
  if (watching) return;
  watching = true;
  void invoke<PeerStatus[]>("peers").then(peers.set);
  void listen<PeerStatus[]>("peers", (event) => peers.set(event.payload));
  void invoke<string | null>("selected").then((id) => chosen.get() === undefined && chosen.set(id ?? null));
}

/**
 * Calls a feature, on this computer if it lives here and on a phone if not:
 * the phone `peer` names, or else the one in focus.
 */
export function call<T = unknown>(feature: string, method: string, body: object = {}, peer?: string): Promise<T> {
  return invoke<T>("call", { feature, method, body, peer: peer ?? null });
}

export function invite(): Promise<Invite> {
  return invoke("invite");
}

export function cancelInvite(): Promise<void> {
  return invoke("cancel_invite");
}

export function unpair(id: string): Promise<void> {
  return invoke("unpair", { id });
}

/** Puts a phone in focus. Everything that shows one phone's things follows it. */
export function select(id: string) {
  if (chosen.get() === id) return;
  chosen.set(id);
  void invoke("select", { id });
}

/** Every paired device, or null until they have loaded. */
export function usePeers(): PeerStatus[] | null {
  watch();
  return useSyncExternalStore(peers.subscribe, peers.get);
}

/** The phone a choice of `id` resolves to: that one, or the connected one, or the newest. */
export function resolvePhone(all: PeerStatus[], id: string | null | undefined): PeerStatus | null {
  return all.find((peer) => peer.id === id) ?? all.find((peer) => peer.online) ?? all.at(-1) ?? null;
}

/** The phone in focus: the chosen one, or else the connected one, or else the one paired last. */
export function usePhone(): PeerStatus | null {
  const all = usePeers() ?? [];
  const id = useSyncExternalStore(chosen.subscribe, chosen.get);
  return resolvePhone(all, id);
}

/** The id of the phone in focus, for code outside React. */
export function focusedId(): string | undefined {
  return resolvePhone(peers.get() ?? [], chosen.get())?.id;
}

interface Notice {
  feature: string;
  method: string;
  body: unknown;
  /** The phone it came from, or is about. */
  peer: string | null;
}

const noticeListeners = new Set<(notice: Notice) => void>();
let hearing = false;

function watchNotices(handler: (notice: Notice) => void) {
  if (!hearing) {
    hearing = true;
    void listen<Notice>("notice", (event) => noticeListeners.forEach((notify) => notify(event.payload)));
  }
  noticeListeners.add(handler);
  return () => void noticeListeners.delete(handler);
}

/**
 * Runs `handler` for each such message. By default only messages about the phone
 * in focus, or about no phone in particular, arrive; `everyPhone` takes all.
 */
export function useNotice<T>(
  feature: string,
  method: string,
  handler: (body: T, peer: string | null) => void,
  { everyPhone = false } = {},
) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(
    () =>
      watchNotices((notice) => {
        if (notice.feature !== feature || notice.method !== method) return;
        if (!everyPhone && notice.peer !== null && notice.peer !== focusedId()) return;
        latest.current(notice.body as T, notice.peer);
      }),
    [feature, method, everyPhone],
  );
}

export interface Remote<T> {
  data: T | undefined;
  error: string | undefined;
  loading: boolean;
  reload: () => void;
}

/**
 * Keeps a value from the phone in focus (or from `peer`), fetched again whenever
 * that phone comes back or another one takes focus.
 */
export function useCall<T>(feature: string, method: string, body: object = {}, peer?: string): Remote<T> {
  const focused = usePhone();
  const all = usePeers() ?? [];
  const target = peer ?? focused?.id;
  const online = all.find((candidate) => candidate.id === target)?.online ?? false;
  const key = JSON.stringify(body);
  const [state, setState] = useState<Omit<Remote<T>, "reload"> & { from?: string }>({ data: undefined, error: undefined, loading: true });
  const request = useRef(0);

  const reload = useCallback(() => {
    const current = ++request.current;
    setState((state) => ({ ...state, loading: true }));
    call<T>(feature, method, JSON.parse(key), target).then(
      (data) => current === request.current && setState({ data, error: undefined, loading: false, from: target }),
      (error: string) => current === request.current && setState((state) => ({ ...state, error, loading: false })),
    );
  }, [feature, method, key, target]);

  useEffect(() => {
    if (online) reload();
  }, [online, reload]);

  // Another phone's answer is not this one's, so it is not shown while the new one loads.
  const stale = state.from !== undefined && state.from !== target;
  return { data: stale ? undefined : state.data, error: state.error, loading: state.loading, reload };
}

export const media = {
  /** Where the app serves bytes from a phone, for `<img>` and the like. */
  url(feature: string, method: string, body: object, peer = focusedId()): string {
    const origin = navigator.userAgent.includes("Windows") ? "http://orither.localhost" : "orither://localhost";
    const phone = peer ? `&p=${encodeURIComponent(peer)}` : "";
    return `${origin}/${feature}/${method}?b=${encodeURIComponent(JSON.stringify(body))}${phone}`;
  },
};

/** The letter a phone goes by on its satellite: its first, or its first two when another phone shares it. */
export function initialOf(peer: PeerStatus, all: PeerStatus[]): string {
  const letters = (name: string) => name.replace(/[^\p{L}\p{N}]/gu, "").toUpperCase();
  const own = letters(peer.device.name) || "?";
  const clash = all.some((other) => other.id !== peer.id && letters(other.device.name)[0] === own[0]);
  return clash ? own.slice(0, 2) : own[0];
}
