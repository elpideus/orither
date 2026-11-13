import { ClipboardList, File, Laptop, Smartphone } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import type { Feature } from "../../core/feature";
import { call, useCall, useNotice } from "../../core/link";
import { CopyButton, Empty, Loaded, Toggle, when } from "../../ui/kit";
import "./clipboard.css";

interface Clip {
  id: number;
  text: string | null;
  files: string[];
  from: "phone" | "computer";
  time: number;
}

interface State {
  enabled: boolean;
  history: Clip[];
}

function useClipboard() {
  const state = useCall<State>("clipboard", "state");
  useNotice("clipboard", "changed", state.reload);
  return state;
}


function Item({ clip }: { clip: Clip }) {
  const From = clip.from === "phone" ? Smartphone : Laptop;
  return (
    <motion.li className="clipboard-item" layout initial={{ opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }}>
      <From size={16} className="dim" />
      {clip.text !== null ? (
        <p className="selectable">{clip.text}</p>
      ) : (
        <p>
          {clip.files.map((file) => (
            <span className="clipboard-file" key={file}>
              <File size={14} />
              {file}
            </span>
          ))}
        </p>
      )}
      <time className="dim">{when(clip.time)}</time>
      {clip.text !== null && <CopyButton text={clip.text} />}
    </motion.li>
  );
}

function Page() {
  const state = useClipboard();
  return (
    <section className="pane clipboard-page">
    <Loaded remote={state} icon={ClipboardList}>
      {({ enabled, history }) => (
        <>
          <div className="pane-head">
            <h2>Clipboard</h2>
            <span className="spacer" />
            <span className="dim">{enabled ? "Copied on one, ready to paste on the other" : "Kept apart"}</span>
            <Toggle on={enabled} label="Share the clipboard" onChange={(on) => void call("clipboard", "enable", { on }).then(state.reload)} />
          </div>
          {history.length === 0 ? (
            <Empty icon={ClipboardList} title="Nothing copied yet">
              <p>Copy text or files on either device, and they are ready to paste on the other.</p>
            </Empty>
          ) : (
            <ul className="clipboard-list">
              <AnimatePresence initial={false}>
                {history.map((clip) => (
                  <Item key={clip.id} clip={clip} />
                ))}
              </AnimatePresence>
            </ul>
          )}
        </>
      )}
    </Loaded>
    </section>
  );
}

export const clipboard: Feature = { id: "clipboard", title: "Clipboard", icon: ClipboardList, Page, tab: true };
