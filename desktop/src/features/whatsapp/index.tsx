import { Download, MessageCircle, RotateCw, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState, type RefObject } from "react";

import type { Feature } from "../../core/feature";
import { announce } from "../../core/island";
import { call, useNotice } from "../../core/link";
import { useNav } from "../../core/nav";
import { Button, Empty } from "../../ui/kit";
import { useWeb } from "./shared";
import "./whatsapp.css";

const SAVED_LASTS = 6_000;

/** Where WhatsApp Web's view goes, in the window's pixels. */
interface Place {
  x: number;
  y: number;
  width: number;
  height: number;
  radius: number;
  behind: string;
}

const paint = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
const blended = new Map<string, string>();

/** The colour around an element: the backgrounds behind it, one over another. */
function behind(element: Element): string {
  const window = getComputedStyle(document.body).backgroundColor;
  const layers = [window];
  for (let node = element.parentElement; node; node = node.parentElement) layers.splice(1, 0, getComputedStyle(node).backgroundColor);
  const key = layers.join("|");
  const known = blended.get(key);
  if (known || !paint) return known ?? window;
  for (const layer of layers) {
    paint.fillStyle = layer;
    paint.fillRect(0, 0, 1, 1);
  }
  const [red, green, blue] = paint.getImageData(0, 0, 1, 1).data;
  const color = `rgb(${red}, ${green}, ${blue})`;
  blended.set(key, color);
  return color;
}

/** Whether nothing the element sits in moves or fades, as while a page opens. */
function still(element: Element): boolean {
  for (let node: Element | null = element; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.transform !== "none" || style.opacity !== "1") return false;
  }
  return true;
}

function placeOf(element: HTMLElement): Place {
  const box = element.getBoundingClientRect();
  return {
    x: Math.round(box.x),
    y: Math.round(box.y),
    width: Math.round(box.width),
    height: Math.round(box.height),
    radius: parseFloat(getComputedStyle(element).borderTopLeftRadius) || 0,
    behind: behind(element),
  };
}

/**
 * Keeps WhatsApp Web's view over `slot`, and out of the way while the page
 * moves, since the view is drawn over the window rather than inside the page.
 */
function usePlacement(slot: RefObject<HTMLElement | null>, on: boolean): string | undefined {
  const [trouble, setTrouble] = useState<string>();
  useEffect(() => {
    if (!on) return;
    let frame = 0;
    let sent = "";
    const place = (at: Place | null) => {
      const key = JSON.stringify(at);
      if (key === sent) return;
      sent = key;
      call("whatsapp", "place", { at }).then(() => setTrouble(undefined), setTrouble);
    };
    const follow = () => {
      frame = requestAnimationFrame(follow);
      const element = slot.current;
      place(element && still(element) && !document.querySelector(".popover") ? placeOf(element) : null);
    };
    follow();
    return () => {
      cancelAnimationFrame(frame);
      place(null);
    };
  }, [slot, on]);
  return trouble;
}

const NOT_AFFILIATED = "This is WhatsApp Web, from WhatsApp itself. Orither is not affiliated with WhatsApp.";

function Page() {
  const web = useWeb();
  const slot = useRef<HTMLDivElement>(null);
  const built = web !== undefined && web.state !== "unsupported";
  const trouble = usePlacement(slot, built);

  if (web?.state === "unsupported") {
    return (
      <section className="pane whatsapp-page">
        <Empty icon={MessageCircle} title="Not on this computer yet">
          <p>WhatsApp Web is built into Orither on Linux and Windows for now. New chats still show on the home page.</p>
        </Empty>
      </section>
    );
  }
  const error = trouble ?? (web?.state === "failed" ? (web.error ?? "WhatsApp Web stopped.") : undefined);
  const state = error !== undefined ? "failed" : web?.state === "open" ? "open" : "opening";
  return (
    <section className="pane whatsapp-page">
      <div className="whatsapp-web" data-state={state} ref={slot}>
        {state === "failed" ? (
          <Empty icon={TriangleAlert} title="WhatsApp Web didn't load">
            <p>{error}</p>
            <Button onClick={() => void call("whatsapp", "reload")}>
              <RotateCw size={15} />
              Try again
            </Button>
            <p className="whatsapp-note">{NOT_AFFILIATED}</p>
          </Empty>
        ) : (
          state === "opening" && (
            <div className="empty">
              <span className="waiting" />
              <h3>Opening WhatsApp Web</h3>
              <p className="whatsapp-note">{NOT_AFFILIATED}</p>
            </div>
          )
        )}
      </div>
    </section>
  );
}

/** Opens the page for a notification's click, and tells of saved downloads. */
function Ambient() {
  const { go } = useNav();
  useNotice("whatsapp", "show", () => go("whatsapp"), { everyPhone: true });
  useNotice<{ name: string; path: string }>("whatsapp", "saved", ({ name, path }) =>
    announce({
      id: `whatsapp-${path}`,
      icon: Download,
      title: name,
      detail: "Saved in Downloads",
      action: { label: "Show", run: () => void call("whatsapp", "reveal", { path }) },
      lasts: SAVED_LASTS,
    }),
  );
  return null;
}

export const whatsapp: Feature = { id: "whatsapp", title: "WhatsApp", icon: MessageCircle, Page, tab: true, Ambient };
