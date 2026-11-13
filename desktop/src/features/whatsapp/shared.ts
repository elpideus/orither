import { useEffect, useState } from "react";

import { call, useCall, useNotice, type Remote } from "../../core/link";

/** A chat, as the phone's notifications showed it. */
export interface Chat {
  id: string;
  title: string;
  unread: number;
  last: string;
  time: number;
}

export interface Inbox {
  allowed: boolean;
  chats: Chat[];
}

/** WhatsApp Web, which runs on the computer whether or not the phone is there. */
export interface Web {
  state: "off" | "opening" | "open" | "failed" | "unsupported";
  /** How many chats wait, as WhatsApp Web counts them. */
  unread: number;
  error: string | null;
}

export function useInbox(): Remote<Inbox> {
  const inbox = useCall<Inbox>("whatsapp", "chats");
  useNotice("whatsapp", "changed", inbox.reload);
  return inbox;
}

export function useWeb(): Web | undefined {
  const [web, setWeb] = useState<Web>();
  useEffect(() => void call<Web>("whatsapp", "web").then((fetched) => setWeb((heard) => heard ?? fetched), () => {}), []);
  useNotice("whatsapp", "web", setWeb);
  return web;
}

/** Opens WhatsApp Web on a chat from the phone's list, as its notification would. */
export function openChat(chat: Chat) {
  void call("whatsapp", "chat", { title: chat.title, id: chat.id });
}
