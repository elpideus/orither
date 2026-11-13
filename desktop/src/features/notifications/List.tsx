import { BellOff, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";

import { call, focusedId, media, select, useNotice, usePeers } from "../../core/link";
import { ease } from "../../core/motion";
import { Empty, IconButton, PhoneMark, when } from "../../ui/kit";
import "./notifications.css";

/** A notification as the phone describes it. */
interface Shown {
  key: string;
  app: string;
  label: string;
  title: string;
  text: string;
  time: number;
  picture: boolean;
  conversation: string | null;
}

interface Kept extends Shown {
  peer: string;
}

/** Every connected phone's notifications, newest first, kept current as they come and go. */
function useNotifications(): Kept[] {
  const peers = usePeers() ?? [];
  const online = peers.filter((peer) => peer.online).map((peer) => peer.id);
  const key = online.join(",");
  const [kept, setKept] = useState<Kept[]>([]);

  useEffect(() => {
    setKept((all) => all.filter((entry) => online.includes(entry.peer)));
    for (const peer of online) {
      call<Shown[]>("notifications", "list", {}, peer).then(
        (list) => setKept((all) => [...all.filter((entry) => entry.peer !== peer), ...(list ?? []).map((shown) => ({ ...shown, peer }))]),
        () => {},
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  useNotice<Shown>(
    "notifications",
    "posted",
    (shown, peer) => peer && setKept((all) => [{ ...shown, peer }, ...all.filter((entry) => !(entry.key === shown.key && entry.peer === peer))]),
    { everyPhone: true },
  );
  useNotice<{ key: string }>(
    "notifications",
    "removed",
    ({ key: gone }, peer) => setKept((all) => all.filter((entry) => !(entry.key === gone && entry.peer === peer))),
    { everyPhone: true },
  );

  return useMemo(() => [...kept].sort((a, b) => b.time - a.time), [kept]);
}

function Row({ entry, mark }: { entry: Kept; mark: boolean }) {
  const phones = usePeers() ?? [];
  const phone = phones.find(({ id }) => id === entry.peer);
  const picture = entry.picture
    ? media.url("notifications", "picture", { key: entry.key }, entry.peer)
    : media.url("apps", "icon", { package: entry.app }, entry.peer);
  const open = () => {
    if (entry.peer !== focusedId()) select(entry.peer);
    void call("notifications", "open", { key: entry.key, app: entry.app, label: entry.label, title: entry.title, conversation: entry.conversation });
  };
  return (
    <motion.li
      layout="position"
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0, transition: { duration: 0.24, ease: ease.out } }}
      exit={{ opacity: 0, x: 24, transition: { duration: 0.16, ease: ease.out } }}
      className="note"
    >
      <button className="row" onClick={open} title={`Open in ${entry.label}`}>
        <img className="note-picture" data-person={entry.picture} src={picture} alt="" draggable={false} />
        <strong>{entry.title || entry.label}</strong>
        <time>{when(entry.time)}</time>
        <p>
          {mark && phone && <PhoneMark phone={phone} size={16} />}
          <span className="note-app">{entry.label}</span>
          {entry.text && <span className="note-text">· {entry.text}</span>}
        </p>
      </button>
      <span className="note-dismiss">
        <IconButton icon={X} label="Dismiss" size={15} onClick={() => void call("notifications", "dismiss", { key: entry.key }, entry.peer)} />
      </span>
    </motion.li>
  );
}

/** The phones' notifications on the home page; a click opens each where it leads. */
export function NotificationList() {
  const notes = useNotifications();
  const phones = usePeers() ?? [];
  const several = phones.length > 1;
  const [enabled, setEnabled] = useState(true);
  useEffect(() => void call<{ enabled: boolean }>("notifications", "state").then((state) => setEnabled(state.enabled), () => {}), []);

  return (
    <>
      <div className="pane-head">
        <h2>Notifications</h2>
        {notes.length > 0 && <span className="count">{notes.length}</span>}
        <span className="spacer" />
      </div>
      {notes.length === 0 ? (
        <Empty icon={BellOff} title="Nothing new">
          <p>{enabled ? "Notifications from your phones land here, and a click opens the app in a window." : "Showing phone notifications is off in Settings."}</p>
        </Empty>
      ) : (
        <ul className="notes">
          <AnimatePresence initial={false}>
            {notes.map((entry) => (
              <Row key={`${entry.peer}/${entry.key}`} entry={entry} mark={several} />
            ))}
          </AnimatePresence>
        </ul>
      )}
    </>
  );
}
