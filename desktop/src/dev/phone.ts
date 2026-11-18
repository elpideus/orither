// Pretend phones, so that the UI can be worked on in a plain browser. Only the
// dev server imports this, outside Tauri. Everything here is made-up sample data.

import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import { media, type PeerStatus } from "../core/link";

const minutes = (count: number) => Date.now() - count * 60_000;

function picture(seed: string, width = 300, height = 300) {
  const hash = [...seed].reduce((sum, letter) => (sum * 31 + letter.charCodeAt(0)) >>> 0, 7);
  const hue = hash % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="hsl(${hue} 55% 55%)"/><stop offset="1" stop-color="hsl(${(hue + 50) % 360} 55% 32%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="${hash % width}" cy="${hash % height}" r="${width / 5}" fill="#fff" opacity=".18"/></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

media.url = (feature, method, body, peer) => picture(`${peer}${feature}${method}${JSON.stringify(body)}`, 200, 200);

const params = new URLSearchParams(location.search);

let peers: PeerStatus[] = [
  { id: "poco", device: { name: "POCO F7", kind: "phone" }, paired_at: Date.now() / 1000 - 86_400 * 30, online: true, route: "local" },
  { id: "work", device: { name: "Galaxy A55", kind: "phone" }, paired_at: Date.now() / 1000 - 86_400 * 4, online: true, route: "internet" },
];
if (params.has("one")) peers = peers.slice(0, 1);
let selected: string | null = "poco";

const threads = [
  { id: "1", address: "+39 333 123 4567", name: "Marta Rinaldi", snippet: "Ci vediamo alle otto?", date: minutes(8), unread: true },
  { id: "2", address: "Intesa", name: null, snippet: "Il tuo codice di verifica è 482913.", date: minutes(41), unread: true },
  { id: "3", address: "Poste", name: "Poste Italiane", snippet: "Il tuo pacco è in consegna oggi.", date: minutes(240), unread: false },
  { id: "4", address: "+39 347 987 6543", name: "Luca Ferretti", snippet: "Grazie mille!", date: minutes(2000), unread: false },
];

const chats = [
  { id: "a", title: "Famiglia", unread: 3, last: "Nonna: domenica pranzo da me", time: minutes(3) },
  { id: "b", title: "Giulia Rossi", unread: 0, last: "Ti mando i documenti stasera", time: minutes(55) },
  { id: "c", title: "Studio", unread: 1, last: "Marco: *riunione* spostata alle 15", time: minutes(120) },
  { id: "d", title: "Calcetto giovedì", unread: 0, last: "Giorgio: siamo in 9, manca uno", time: minutes(300) },
  { id: "e", title: "Luca Ferretti", unread: 0, last: "Ci vediamo alle 8 in stazione?", time: minutes(900) },
];

const notifications: Record<string, object[]> = {
  poco: [
    { key: "n1", app: "it.poste.postepay", label: "Poste Italiane", title: "Accesso", text: "Il tuo codice di accesso è 482 913", time: minutes(2), alert: false, quiet: false, picture: false, conversation: null },
    { key: "n2", app: "com.whatsapp", label: "WhatsApp", title: "Luca Ferretti", text: "Ci vediamo alle 8 in stazione?", time: minutes(14), alert: false, quiet: false, picture: true, conversation: "e" },
    { key: "n3", app: "com.amazon.mShop", label: "Amazon", title: "Consegna oggi", text: "Il tuo pacco arriva entro le 21", time: minutes(70), alert: false, quiet: true, picture: false, conversation: null },
  ],
  work: [
    { key: "w1", app: "com.microsoft.teams", label: "Teams", title: "Standup", text: "Spostato alle 15:00", time: minutes(9), alert: false, quiet: false, picture: false, conversation: null },
    { key: "w2", app: "com.google.android.gm", label: "Gmail", title: "Elena Bassi", text: "Re: preventivo cucina", time: minutes(130), alert: false, quiet: false, picture: true, conversation: null },
  ],
};

let calls: Record<string, object[]> = { poco: [], work: [] };

const replies: Record<string, unknown> = {
  "system.battery": { level: 82, charging: false },
  "system.about": { model: "POCO F7", android: "16", storage: { free: 148e9, total: 512e9 } },
  "sms.threads": threads,
  "sms.messages": [
    { id: "1", body: "Accedi con il codice qui sotto.", date: minutes(50), incoming: true },
    { id: "2", body: "Il tuo codice di verifica è 482913. Non condividerlo con nessuno.", date: minutes(41), incoming: true },
    { id: "3", body: "Fatto, grazie", date: minutes(40), incoming: false },
  ],
  "whatsapp.chats": { allowed: true, chats },
  // The browser has no WhatsApp Web of its own to show.
  "whatsapp.web": { state: "unsupported", unread: 4, error: null },
  "gallery.albums": [
    { id: "camera", name: "Camera", count: 2410 },
    { id: "shots", name: "Screenshots", count: 312 },
    { id: "wa", name: "WhatsApp Images", count: 988 },
  ],
  "files.roots": [{ name: "Phone storage", path: "/storage/emulated/0", free: 148e9, total: 512e9 }],
  "files.list": [
    { name: "DCIM", path: "/storage/emulated/0/DCIM", folder: true, size: 0, modified: minutes(30), mime: null },
    { name: "Download", path: "/storage/emulated/0/Download", folder: true, size: 0, modified: minutes(300), mime: null },
    { name: "Pictures", path: "/storage/emulated/0/Pictures", folder: true, size: 0, modified: minutes(900), mime: null },
    { name: "Contratto affitto.pdf", path: "/storage/emulated/0/Contratto affitto.pdf", folder: false, size: 2_400_000, modified: minutes(4000), mime: "application/pdf" },
    { name: "Matera.mp4", path: "/storage/emulated/0/Matera.mp4", folder: false, size: 840_000_000, modified: minutes(9000), mime: "video/mp4" },
  ],
  "clipboard.state": {
    enabled: true,
    history: [
      { id: 2, text: "Via Roma 12, Ostuni", files: [], from: "phone", time: minutes(4) },
      { id: 1, text: "https://orither.dev", files: [], from: "computer", time: minutes(12) },
    ],
  },
  "notifications.state": { enabled: true },
  "input.state": { supported: true, active: false },
  "hardware.devices": [
    { id: "microphone", on: false, blocked: null, setup: false },
    { id: "speaker", on: true, blocked: null, setup: false },
    { id: "camera", on: false, blocked: "Needs a one-time setup that asks for your password. After that it works after every restart.", setup: true },
  ],
  "calls.audio": { state: "unpaired", where: "computer", detail: null },
  "calls.contacts": [
    { id: "c1", name: "Elena Bassi", picture: true, numbers: [{ number: "+39 345 220 1876", label: "Mobile" }], apps: [{ data: "11", app: "com.whatsapp", appLabel: "WhatsApp", video: false }, { data: "12", app: "com.whatsapp", appLabel: "WhatsApp", video: true }] },
    { id: "c2", name: "Giorgio Conte", picture: false, numbers: [{ number: "+39 328 554 0921", label: "Mobile" }], apps: [{ data: "21", app: "org.telegram.messenger", appLabel: "Telegram", video: false }] },
    { id: "c3", name: "Marta Rinaldi", picture: true, numbers: [{ number: "+39 333 123 4567", label: "Mobile" }], apps: [{ data: "31", app: "com.whatsapp", appLabel: "WhatsApp", video: false }] },
    { id: "c4", name: "Studio dentistico Ferri", picture: false, numbers: [{ number: "+39 080 431 2290", label: "Work" }], apps: [] },
  ],
  "calls.sims": { sims: [{ id: "1", label: "Iliad" }, { id: "2", label: "TIM" }], preferred: null },
  "calls.recents": [
    { number: "+39 333 123 4567", name: "Marta Rinaldi", type: "missed", date: minutes(25), duration: 0, contact: "c3" },
    { number: "+39 02 8846 1120", name: null, type: "incoming", date: minutes(180), duration: 342, contact: null },
    { number: "+39 345 220 1876", name: "Elena Bassi", type: "outgoing", date: minutes(1500), duration: 96, contact: "c1" },
  ],
  "apps.list": ["Trenitalia", "Revolut", "Strava", "Telegram", "Spotify", "Gmail", "Maps", "Duolingo", "Notion", "Camera", "Keep", "Instagram", "Satispay", "Calendar", "Authy", "Firefox", "Settings", "Clock", "Calculator", "YouTube"].map((label) => ({
    package: `com.example.${label.toLowerCase().replace(" ", "")}`,
    label,
  })),
};

function gallery(offset: number, limit: number) {
  const total = 300;
  return Array.from({ length: Math.max(0, Math.min(limit, total - offset)) }, (_, index) => {
    const id = offset + index;
    return { id: String(id), name: `IMG_${id}.jpg`, date: minutes(id * 400), video: id % 11 === 3, width: 4000, height: 3000 };
  });
}

const announcePeers = () => void emit("peers", peers);

mockWindows("main");
// Orither's own updates: ?update=available, downloading, ready, current, failed or manual.
const updateNotes = "## What's new\n\n- Phones on the same Wi-Fi find each other faster\n- The gallery keeps its place after a photo is opened\n- Calls ring on every linked phone at once\n\n**Full changelog**: https://github.com/elpideus/orither/compare/v0.1.0...v0.2.0";
let update = {
  current: "0.1.0",
  phase: "current",
  release: null as null | object,
  progress: null as null | { done: number; total: number | null },
  checked: Date.now() - 12 * 60_000,
  error: null as string | null,
  auto: true,
  manual: null as string | null,
  asks_password: false,
};
{
  const wanted = params.get("update");
  const release = { version: "0.2.0", notes: updateNotes, date: Date.now() - 86_400_000, page: "https://github.com/elpideus/orither/releases/tag/v0.2.0" };
  if (wanted === "available" || wanted === "manual") update = { ...update, phase: "available", release, auto: wanted === "manual", manual: wanted === "manual" ? "This copy was not installed from a release, so it does not update itself." : null };
  if (wanted === "downloading") update = { ...update, phase: "downloading", release, progress: { done: 41e6, total: 96e6 } };
  if (wanted === "ready") update = { ...update, phase: "ready", release };
  if (wanted === "failed") update = { ...update, phase: "failed", error: "Could not reach GitHub. Check the internet connection." };
}

function updates(method: string, body: Record<string, unknown>) {
  const changed = (next: typeof update) => {
    update = next;
    void emit("notice", { feature: "updates", method: "status", body: update, peer: null });
  };
  if (method === "auto") changed({ ...update, auto: Boolean(body.on) });
  if (method === "check") changed({ ...update, phase: "current", checked: Date.now(), error: null });
  if (method === "download" && update.release) {
    changed({ ...update, phase: "downloading", progress: { done: 0, total: 96e6 } });
    const tick = setInterval(() => {
      const done = Math.min((update.progress?.done ?? 0) + 12e6, 96e6);
      if (done >= 96e6) {
        clearInterval(tick);
        changed({ ...update, phase: "ready", progress: null });
      } else changed({ ...update, progress: { done, total: 96e6 } });
    }, 400);
  }
  if (method === "install") changed({ ...update, phase: "installing" });
  return update;
}

mockIPC(
  (command, payload) => {
    const args = (payload ?? {}) as { feature: string; method: string; body: Record<string, unknown>; peer?: string | null; id?: string };
    const phone = args.peer ?? selected ?? "poco";
    switch (command) {
      case "peers":
        return params.has("pair") ? [] : peers;
      case "selected":
        return selected;
      case "select":
        selected = args.id ?? null;
        return null;
      case "invite":
        return { code: "K7QM-2XPD", uri: "orither://pair/K7QM2XPDN4RT8VWYZ3F6HJ9CBG", expires_in_secs: 300 };
      case "theme":
        return params.has("light")
          ? { window: "#eff0f1", view: "#fcfcfc", button: "#fcfcfc", text: "#232627", dim: "#707d8a", accent: "#3daee9", on_accent: "#ffffff" }
          : params.has("pink")
            ? { window: "#171717", view: "#1c1c1c", button: "#252525", text: "#ebebeb", dim: "#a0a0a0", accent: "#e6a5d9", on_accent: "#341e30" }
            : null;
      case "call": {
        const { feature, method, body } = args;
        if (feature === "updates") return updates(method, body);
        if (feature === "setup" && method === "devices") {
          const wanted = params.get("usb");
          if (wanted === null) return { adb: true, devices: [] };
          const state = wanted === "unauthorized" ? "unauthorized" : "ready";
          return { adb: true, devices: [{ serial: "R5CT1234", model: "Galaxy S24", state, app: wanted === "installed" ? "0.1.1" : null }] };
        }
        if (feature === "gallery" && method === "items") return gallery(body.offset as number, body.limit as number);
        if (feature === "notifications" && method === "list") return notifications[phone] ?? [];
        if (feature === "calls" && method === "list") return { calls: calls[phone] ?? [] };
        if (feature === "calls" && ["answer", "decline", "hangup"].includes(method)) {
          const list = (calls[phone] ?? []) as { id: string; state: string; since: number }[];
          const next = list.map((entry) =>
            entry.id === body.id ? { ...entry, state: method === "answer" ? "active" : "ended", since: method === "answer" ? Date.now() : entry.since, can: { answer: false, decline: false, hangup: method === "answer" } } : entry,
          );
          calls = { ...calls, [phone]: next };
          void emit("notice", { feature: "calls", method: "changed", body: { calls: next }, peer: phone });
          if (method !== "answer") setTimeout(() => {
            calls = { ...calls, [phone]: [] };
            void emit("notice", { feature: "calls", method: "changed", body: { calls: [] }, peer: phone });
          }, 2000);
          return true;
        }
        if (phone === "work" && feature === "system" && method === "battery") return { level: 41, charging: true };
        if (phone === "work" && feature === "system" && method === "about") return { model: "Galaxy A55", android: "15", storage: { free: 61e9, total: 128e9 } };
        return replies[`${feature}.${method}`] ?? null;
      }
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

// Lets things happen while the page is watched, from the console or the address bar.
const pretend = {
  smsArrives: () =>
    emit("notice", { feature: "sms", method: "received", body: { name: null, address: "Intesa", body: "Il tuo codice di verifica è 482913." }, peer: "poco" }),
  callRings: (phone = "work") => {
    calls = {
      ...calls,
      [phone]: [
        { id: "app:1", source: "app", app: "com.whatsapp", appLabel: "WhatsApp", name: "Marta Rinaldi", number: null, picture: true, video: false, state: "ringing", since: Date.now(), can: { answer: true, decline: true, hangup: false } },
      ],
    };
    void emit("notice", { feature: "calls", method: "changed", body: { calls: calls[phone] }, peer: phone });
  },
  goesAway: (phone = "work") => {
    peers = peers.map((peer) => (peer.id === phone ? { ...peer, online: !peer.online } : peer));
    announcePeers();
  },
  moves: (phone = "poco", route: PeerStatus["route"] = "nearby") => {
    peers = peers.map((peer) => (peer.id === phone ? { ...peer, route } : peer));
    announcePeers();
  },
  copies: () =>
    emit("notice", { feature: "transfer", method: "progress", body: { id: 1, name: "Matera.mp4", done: 320e6, total: 840e6, finished: false }, peer: null }),
  refusesClicks: (phone = "POCO F7") => emit("notice", { feature: "screen", method: "touch", body: { phone }, peer: null }),
};
Object.assign(window, pretend);
if (params.has("ring")) setTimeout(() => pretend.callRings(), 600);
if (params.has("clicks")) setTimeout(() => void pretend.refusesClicks(), 600);
