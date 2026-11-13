import { Laptop, Phone, PhoneOff, Smartphone, Video } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";

import { call, media, useCall, useNotice, usePeers } from "../../core/link";
import { ease, spring } from "../../core/motion";
import { Avatar, Button, clock, PhoneMark } from "../../ui/kit";
import { act, describe, useCalls, type Audio, type PhoneCall } from "./state";
import "./calls.css";

function useSeconds(since: number, running: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);
  return (now - since) / 1000;
}

/** Where the call is heard: this computer, over the link, or the phone itself. */
function AudioSwitch({ peer }: { peer: string }) {
  const audio = useCall<Audio>("calls", "audio", { peer });
  useNotice("calls", "audio", audio.reload, { everyPhone: true });
  if (!audio.data) return null;
  const here = audio.data.where === "computer";
  const route = (to: "computer" | "phone") => void call("calls", "route", { to, peer }).finally(audio.reload);
  return (
    <div className="audio-switch" role="radiogroup" aria-label="Call audio">
      {(["computer", "phone"] as const).map((where) => {
        const on = (where === "computer") === here;
        const Icon = where === "computer" ? Laptop : Smartphone;
        return (
          <button key={where} role="radio" aria-checked={on} onClick={() => route(where)} title={where === "computer" ? "Use this computer's microphone and speakers" : "Use the phone"}>
            {on && <motion.span className="audio-switch-mark" layoutId={`audio-${peer}`} transition={spring.glide} />}
            <Icon size={15} />
            <span>{where === "computer" ? "Here" : "Phone"}</span>
          </button>
        );
      })}
    </div>
  );
}

export function CallCard({ entry, floating = false }: { entry: PhoneCall; floating?: boolean }) {
  const phones = usePeers() ?? [];
  const phone = phones.find(({ id }) => id === entry.peer);
  const { call: current } = entry;
  const live = current.state === "active" || current.state === "held";
  const seconds = useSeconds(current.since, live);
  const picture = current.picture ? media.url("calls", "picture", { id: current.id }, entry.peer) : undefined;
  const [busy, setBusy] = useState(false);

  const run = (action: "answer" | "decline" | "hangup") => {
    setBusy(true);
    // Answering here means talking here, so the call's sound comes to this computer.
    const before = action === "answer" ? call("calls", "route", { to: "computer", peer: entry.peer }).catch(() => {}) : Promise.resolve();
    before.then(() => act(entry, action)).finally(() => setBusy(false));
  };

  return (
    <motion.article
      className={floating ? "call-card" : "call-band"}
      data-state={current.state}
      data-floating={floating}
      layout
      initial={{ opacity: 0, y: floating ? 16 : 8, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.28, ease: ease.out } }}
      exit={{ opacity: 0, scale: 0.98, transition: { duration: 0.18, ease: ease.out } }}
      transition={spring.glide}
    >
      <div className="call-who">
        <span className="call-avatar" data-ringing={current.state === "ringing"}>
          <Avatar name={current.name || current.number || "?"} src={picture} size={48} />
        </span>
        <div className="call-words">
          <strong>{current.name || current.number || "Unknown caller"}</strong>
          <p>
            {current.video ? <Video size={14} /> : <Phone size={14} />}
            {live ? <span className="num">{clock(seconds)}</span> : describe(entry)}
            {live && current.state === "held" && " · on hold"}
            {live && current.source === "app" && ` · ${current.appLabel}`}
          </p>
        </div>
        {phone && phones.length > 1 && <PhoneMark phone={phone} size={22} />}
      </div>
      <div className="call-actions">
        {current.state === "ringing" ? (
          <>
            {current.can.decline && (
              <Button onClick={() => run("decline")} disabled={busy}>
                <PhoneOff size={16} />
                Decline
              </Button>
            )}
            {current.can.answer && (
              <Button kind="primary" onClick={() => run("answer")} disabled={busy}>
                <Phone size={16} />
                Answer
              </Button>
            )}
          </>
        ) : current.state === "ended" ? (
          <span className="dim">Call ended</span>
        ) : (
          <>
            <AudioSwitch peer={entry.peer} />
            <span className="spacer" />
            {current.can.hangup && (
              <Button kind="danger" onClick={() => run("hangup")} disabled={busy} aria-label="Hang up">
                <PhoneOff size={16} />
                Hang up
              </Button>
            )}
          </>
        )}
      </div>
      {live && current.source === "app" && current.video && <p className="call-note">The video stays on the phone. Its sound comes here with “Here”.</p>}
    </motion.article>
  );
}

/** Calls on every phone, pinned at the top of the home page's activity pane. */
export function CallCards() {
  const calls = useCalls();
  return (
    <AnimatePresence initial={false}>
      {calls.length > 0 && (
        <motion.div
          className="call-cards"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0, transition: { duration: 0.15 } }}
        >
          <AnimatePresence initial={false} mode="popLayout">
            {calls.map((entry) => (
              <CallCard key={`${entry.peer}/${entry.call.id}`} entry={entry} />
            ))}
          </AnimatePresence>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
