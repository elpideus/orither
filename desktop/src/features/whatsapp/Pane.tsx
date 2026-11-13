import { ArrowUpRight, MessageCircle } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import { call } from "../../core/link";
import { ease } from "../../core/motion";
import { useNav } from "../../core/nav";
import { Avatar, Button, Empty, when } from "../../ui/kit";
import { formatted } from "./format";
import { openChat, useInbox, useWeb } from "./shared";
import "./whatsapp.css";

/** The latest chats from the phone, each opening in WhatsApp Web here. */
export function WhatsAppPane() {
  const { go } = useNav();
  const inbox = useInbox();
  const web = useWeb();
  const chats = inbox.data?.chats ?? [];
  const waiting = Math.max(web?.unread ?? 0, chats.filter(({ unread }) => unread > 0).length);

  return (
    <section className="pane" aria-label="WhatsApp">
      <div className="pane-head">
        <h2>WhatsApp</h2>
        {waiting > 0 && <span className="count">{waiting} unread</span>}
        <span className="spacer" />
        <Button kind="quiet" onClick={() => go("whatsapp")}>
          Open
          <ArrowUpRight size={15} />
        </Button>
      </div>
      {inbox.data?.allowed === false ? (
        <Empty icon={MessageCircle} title="New messages show here">
          <p>Allow notification access on the phone, and the latest chats appear as they arrive.</p>
          <Button onClick={() => void call("whatsapp", "allow")}>Open the setting on the phone</Button>
        </Empty>
      ) : chats.length === 0 ? (
        <Empty icon={MessageCircle} title={inbox.data ? "No new chats" : ""}>
          {inbox.data && <p>Chats appear here as messages arrive.</p>}
        </Empty>
      ) : (
        <ul className="whatsapp-chats">
          <AnimatePresence initial={false}>
            {chats.slice(0, 12).map((chat) => (
              <motion.li
                key={chat.id}
                layout="position"
                initial={{ opacity: 0, y: -8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.22, ease: ease.out }}
              >
                <button
                  className="row"
                  onClick={() => {
                    openChat(chat);
                    go("whatsapp");
                  }}
                >
                  <Avatar name={chat.title} />
                  <strong>{chat.title}</strong>
                  <time>{when(chat.time)}</time>
                  <p>
                    {chat.unread > 0 && <span className="whatsapp-unread num">{chat.unread}</span>}
                    {formatted(chat.last)}
                  </p>
                </button>
              </motion.li>
            ))}
          </AnimatePresence>
        </ul>
      )}
    </section>
  );
}
