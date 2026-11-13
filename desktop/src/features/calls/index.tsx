import { Bluetooth, Delete, Laptop, Phone, PhoneCall, PhoneIncoming, PhoneMissed, PhoneOutgoing, Smartphone, Video, type LucideIcon } from "lucide-react";
import { AnimatePresence } from "motion/react";
import { useMemo, useState } from "react";

import type { Feature } from "../../core/feature";
import { call, media, useCall, useNotice, usePhone } from "../../core/link";
import { useNav } from "../../core/nav";
import { Avatar, Button, Empty, Loaded, Search, when } from "../../ui/kit";
import { CallCard } from "./CallCards";
import { dialNumber, useCalls, useFollowCalls, useSims, type Audio } from "./state";
import "./calls.css";

interface Contact {
  id: string;
  name: string;
  picture: boolean;
  numbers: { number: string; label: string }[];
  apps: { data: string; app: string; appLabel: string; video: boolean }[];
}

interface Recent {
  number: string;
  name: string | null;
  type: "incoming" | "outgoing" | "missed" | "rejected" | "blocked" | "voicemail";
  date: number;
  duration: number;
}

const KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"];
const LETTERS: Record<string, string> = { "2": "ABC", "3": "DEF", "4": "GHI", "5": "JKL", "6": "MNO", "7": "PQRS", "8": "TUV", "9": "WXYZ", "0": "+" };

function Dialer() {
  const [number, setNumber] = useState("");
  const [problem, setProblem] = useState<string>();
  const { sims, sim, pick } = useSims();
  const dial = () => {
    if (!number.trim()) return;
    setProblem(undefined);
    dialNumber(number.trim(), sim).then(() => setNumber(""), setProblem);
  };
  return (
    <div className="dialer">
      {sims.length > 1 && (
        <div className="segmented dialer-sims" role="radiogroup" aria-label="Call from">
          {sims.map(({ id, label }) => (
            <button key={id} role="radio" aria-checked={id === sim} aria-selected={id === sim} onClick={() => pick(id)}>
              {label}
            </button>
          ))}
        </div>
      )}
      <div className="dialer-number">
        <input
          className="selectable num"
          value={number}
          placeholder="Number"
          inputMode="tel"
          aria-label="Number to call"
          onChange={(event) => setNumber(event.target.value.replace(/[^\d+*#\s()-]/g, ""))}
          onKeyDown={(event) => event.key === "Enter" && dial()}
        />
        {number && <button className="icon-button" aria-label="Delete a digit" onClick={() => setNumber(number.slice(0, -1))}><Delete size={18} /></button>}
      </div>
      <div className="keypad">
        {KEYS.map((key) => (
          <button key={key} className="key" onClick={() => setNumber(number + key)}>
            <span className="num">{key}</span>
            <small>{LETTERS[key] ?? " "}</small>
          </button>
        ))}
      </div>
      <Button kind="primary" size="large" onClick={dial} disabled={!number.trim()} className="dialer-call">
        <Phone size={17} />
        Call
      </Button>
      {problem && <p className="calls-problem">{problem}</p>}
    </div>
  );
}

/** Where calls sound, and the Bluetooth fallback for when the link can't carry them. */
function CallAudio() {
  const phone = usePhone();
  const audio = useCall<Audio>("calls", "audio", { peer: phone?.id });
  useNotice("calls", "audio", audio.reload);
  if (!audio.data) return null;
  const here = audio.data.where === "computer";
  const route = (to: "computer" | "phone") => void call("calls", "route", { to, peer: phone?.id }).finally(audio.reload);
  const bluetooth: Record<Audio["state"], string | null> = {
    unsupported: null,
    "no-bluetooth": null,
    "unknown-phone": null,
    unpaired: "Pair over Bluetooth as a fallback, for when the link is slow or down.",
    pairing: "Pairing: check that both devices show the same code, and confirm on both.",
    disconnected: "Paired over Bluetooth as a fallback.",
    connecting: "Connecting over Bluetooth…",
    connected: "Bluetooth hands-free is carrying calls.",
  };
  const fallback = audio.data.detail ?? bluetooth[audio.data.state];
  return (
    <div className="call-audio">
      <span className="call-audio-glyph">{here ? <Laptop size={18} /> : <Smartphone size={18} />}</span>
      <div>
        <strong>{here ? "Calls use this computer" : "Calls stay on the phone"}</strong>
        <p>{here ? "Its microphone and speakers or headphones carry phone calls and app calls, over Orither's link." : "Answering here brings the sound here too."}</p>
      </div>
      {here ? <Button onClick={() => route("phone")}>Back to phone</Button> : <Button onClick={() => route("computer")}>Use this computer</Button>}
      {fallback && (
        <p className="call-audio-fallback">
          <Bluetooth size={14} />
          <span>{fallback}</span>
          {audio.data.state === "unpaired" && (
            <Button kind="quiet" onClick={() => void call("calls", "pair", { peer: phone?.id }).finally(audio.reload)}>
              Pair
            </Button>
          )}
        </p>
      )}
    </div>
  );
}

const RECENT_ICON: Record<Recent["type"], LucideIcon> = {
  incoming: PhoneIncoming,
  outgoing: PhoneOutgoing,
  missed: PhoneMissed,
  rejected: PhoneMissed,
  blocked: PhoneMissed,
  voicemail: PhoneIncoming,
};

function Recents() {
  const recents = useCall<Recent[]>("calls", "recents", { limit: 80 });
  const { sim } = useSims();
  useNotice("calls", "changed", recents.reload);
  return (
    <Loaded remote={recents} icon={PhoneCall}>
      {(list) =>
        list.length === 0 ? (
          <Empty icon={PhoneCall} title="No calls yet" />
        ) : (
          <ul className="calls-list">
            {list.map((recent) => {
              const Icon = RECENT_ICON[recent.type];
              const missed = recent.type === "missed" || recent.type === "rejected";
              return (
                <li key={`${recent.date}${recent.number}`}>
                  <button className="row" onClick={() => void dialNumber(recent.number, sim)} title={`Call ${recent.name ?? recent.number}`}>
                    <span className="recent-glyph" data-missed={missed}>
                      <Icon size={17} />
                    </span>
                    <strong data-missed={missed}>{recent.name ?? recent.number}</strong>
                    <time>{when(recent.date)}</time>
                    <p>{recent.name ? recent.number : recent.type === "missed" ? "Missed" : " "}</p>
                  </button>
                </li>
              );
            })}
          </ul>
        )
      }
    </Loaded>
  );
}

function ContactRow({ contact, sim }: { contact: Contact; sim: string | undefined }) {
  const [problem, setProblem] = useState<string>();
  const place = (data: string) => call("calls", "place", { data }).catch(setProblem);
  const picture = contact.picture ? media.url("calls", "contact-picture", { id: contact.id }) : undefined;
  return (
    <li className="contact">
      <Avatar name={contact.name} src={picture} />
      <div className="contact-words">
        <strong>{contact.name}</strong>
        <p className="num">{problem ?? contact.numbers[0]?.number ?? ""}</p>
      </div>
      <div className="contact-actions">
        {contact.numbers.slice(0, 1).map(({ number }) => (
          <button key={number} className="contact-action" onClick={() => void dialNumber(number, sim).catch(setProblem)} title={`Call ${number}`}>
            <Phone size={15} />
            Call
          </button>
        ))}
        {contact.apps.map((app) => (
          <button key={app.data} className="contact-action" onClick={() => void place(app.data)} title={`${app.appLabel} ${app.video ? "video call" : "call"}`}>
            {app.video ? <Video size={15} /> : <PhoneCall size={15} />}
            {app.appLabel}
          </button>
        ))}
      </div>
    </li>
  );
}

function Contacts() {
  const [search, setSearch] = useState("");
  const { sim } = useSims();
  const contacts = useCall<Contact[]>("calls", "contacts", { limit: 400 });
  const shown = useMemo(() => {
    const words = search.trim().toLowerCase();
    const digits = words.replace(/\D/g, "");
    return contacts.data?.filter(
      ({ name, numbers }) => name.toLowerCase().includes(words) || (digits.length > 2 && numbers.some(({ number }) => number.replace(/\D/g, "").includes(digits))),
    );
  }, [contacts.data, search]);
  return (
    <>
      <div className="calls-search">
        <Search value={search} onChange={setSearch} placeholder="Search contacts" />
      </div>
      <Loaded remote={contacts} icon={PhoneCall}>
        {() =>
          shown?.length ? (
            <ul className="calls-list">
              {shown.slice(0, 200).map((contact) => (
                <ContactRow key={contact.id} contact={contact} sim={sim} />
              ))}
            </ul>
          ) : (
            <Empty icon={PhoneCall} title={search ? `Nobody called “${search.trim()}”` : "No contacts"} />
          )
        }
      </Loaded>
    </>
  );
}

function Page() {
  const [list, setList] = useState<"recents" | "contacts">("recents");
  const calls = useCalls();
  return (
    <div className="calls-page">
      <section className="pane calls-side">
        <AnimatePresence initial={false} mode="popLayout">
          {calls.map((entry) => (
            <CallCard key={`${entry.peer}/${entry.call.id}`} entry={entry} />
          ))}
        </AnimatePresence>
        <Dialer />
        <CallAudio />
      </section>
      <section className="pane calls-main">
        <div className="pane-head">
          <div className="segmented" role="tablist">
            {(["recents", "contacts"] as const).map((name) => (
              <button key={name} role="tab" aria-selected={list === name} onClick={() => setList(name)}>
                {name === "recents" ? "Recent" : "Contacts"}
              </button>
            ))}
          </div>
        </div>
        {list === "recents" ? <Recents /> : <Contacts />}
      </section>
    </div>
  );
}

/** Follows every phone's calls, and puts a ringing one in front wherever the window is. */
function Ambient() {
  useFollowCalls();
  const { page } = useNav();
  const calls = useCalls();
  const floating = page === "home" || page === "calls" ? [] : calls.filter(({ call: current }) => current.state !== "ended");
  return (
    <div className="call-layer">
      <AnimatePresence initial={false}>
        {floating.slice(0, 2).map((entry) => (
          <CallCard key={`${entry.peer}/${entry.call.id}`} entry={entry} floating />
        ))}
      </AnimatePresence>
    </div>
  );
}

export const calls: Feature = { id: "calls", title: "Calls", icon: Phone, Page, tab: true, Ambient };
