import { KeyRound, MessageSquareText } from "lucide-react";
import { useMemo, useState } from "react";

import type { Feature } from "../../core/feature";
import { announce } from "../../core/island";
import { call, useCall, useNotice, type Remote } from "../../core/link";
import { Avatar, CopyButton, Empty, Loaded, Search, when } from "../../ui/kit";
import { findCode } from "./code";

interface Thread {
  id: string;
  address: string;
  name: string | null;
  snippet: string;
  date: number;
  unread: boolean;
}

interface Message {
  id: string;
  body: string;
  date: number;
  incoming: boolean;
}

const nameOf = (thread: Pick<Thread, "name" | "address">) => thread.name ?? thread.address;

function useThreads(): Remote<Thread[]> {
  const threads = useCall<Thread[]>("sms", "threads");
  useNotice("sms", "received", threads.reload);
  return threads;
}

function Bubble({ message }: { message: Message }) {
  const code = message.incoming ? findCode(message.body) : null;
  return (
    <div className="bubble" data-mine={!message.incoming}>
      {message.body}
      {code && (
        <div className="bubble-code">
          <KeyRound size={15} />
          <strong>{code}</strong>
          <CopyButton text={code} />
        </div>
      )}
    </div>
  );
}

function Conversation({ thread }: { thread: Thread }) {
  const messages = useCall<Message[]>("sms", "messages", { thread: thread.id });
  useNotice("sms", "received", messages.reload);
  return (
    <>
      <div className="pane-head">
        <Avatar name={nameOf(thread)} size={32} />
        <h2 className="selectable">{nameOf(thread)}</h2>
        {thread.name && <span className="count selectable">{thread.address}</span>}
      </div>
      <Loaded remote={messages} icon={MessageSquareText}>
        {(data) => (
          <div className="conversation selectable">
            {data.map((message, index) => {
              const earlier = data[index - 1];
              const gap = !earlier || message.date - earlier.date > 3_600_000;
              return (
                <div key={message.id} className="bubble-row" data-mine={!message.incoming}>
                  {gap && <p className="bubble-time">{when(message.date)}</p>}
                  <Bubble message={message} />
                </div>
              );
            })}
          </div>
        )}
      </Loaded>
    </>
  );
}

function Page() {
  const threads = useThreads();
  const [open, setOpen] = useState<string>();
  const [search, setSearch] = useState("");
  const shown = useMemo(() => {
    const words = search.trim().toLowerCase();
    return threads.data?.filter((thread) => nameOf(thread).toLowerCase().includes(words) || thread.snippet.toLowerCase().includes(words));
  }, [threads.data, search]);
  const selected = threads.data?.find(({ id }) => id === open) ?? shown?.[0];
  return (
    <Loaded remote={threads} icon={MessageSquareText}>
      {(data) =>
        data.length === 0 ? (
          <section className="pane">
            <Empty icon={MessageSquareText} title="No text messages">
              <p>RCS chats live outside Android's message store, so only SMS show here.</p>
            </Empty>
          </section>
        ) : (
          <div className="split">
            <section className="pane">
              <div className="pane-head messages-search">
                <Search value={search} onChange={setSearch} placeholder="Search messages" />
              </div>
              <nav className="split-list">
                {shown?.map((thread) => (
                  <button key={thread.id} className="row" data-selected={thread.id === selected?.id} onClick={() => setOpen(thread.id)}>
                    <Avatar name={nameOf(thread)} />
                    <strong>{nameOf(thread)}</strong>
                    <time>{when(thread.date)}</time>
                    <p data-unread={thread.unread}>{thread.snippet}</p>
                  </button>
                ))}
              </nav>
            </section>
            <section className="pane">{selected ? <Conversation key={selected.id} thread={selected} /> : <Empty icon={MessageSquareText} title="Pick a conversation" />}</section>
          </div>
        )
      }
    </Loaded>
  );
}

/** Puts an arriving code within one click, whatever else is on screen. */
function Ambient() {
  useNotice<{ name: string | null; address: string; body: string }>(
    "sms",
    "received",
    (message) => {
      const code = findCode(message.body);
      if (!code) return;
      announce({
        id: "sms-code",
        icon: KeyRound,
        title: code,
        detail: `Code from ${nameOf(message)}`,
        action: { label: "Copy", run: () => void call("clipboard", "copy", { text: code }) },
        lasts: 60_000,
      });
    },
    { everyPhone: true },
  );
  return null;
}

export const messages: Feature = { id: "sms", title: "Messages", icon: MessageSquareText, Page, tab: true, Ambient };
