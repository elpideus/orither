import { RotateCcw, Smartphone, TriangleAlert } from "lucide-react";

import type { Feature } from "../../core/feature";
import { announce, dismiss } from "../../core/island";
import { useNotice } from "../../core/link";

const ID = "helper";
const SETTLED_LASTS = 8_000;

/** Tells what happens while the computer starts Orither's helper on the phone without a cable. */
function Ambient() {
  useNotice(ID, "waking", () =>
    announce({ id: ID, icon: RotateCcw, title: "Starting Orither on your phone", detail: "Over Wi-Fi, no cable needed" }),
  );
  useNotice(ID, "rehearsing", () =>
    announce({ id: ID, icon: RotateCcw, title: "Setting up restarts", detail: "So your phone needs no cable after one" }),
  );
  useNotice(ID, "asking", () =>
    announce({
      id: ID,
      icon: Smartphone,
      title: "Check your phone",
      detail: "Tap Allow, with Always allow on this network ticked",
    }),
  );
  useNotice(ID, "woke", () => dismiss(ID));
  useNotice(ID, "rehearsed", () =>
    announce({
      id: ID,
      icon: RotateCcw,
      title: "Restarts are covered",
      detail: "Your phone starts Orither again by itself",
      lasts: SETTLED_LASTS,
    }),
  );
  useNotice<{ error: string }>(ID, "failed", ({ error }) =>
    announce({ id: ID, icon: TriangleAlert, title: "Orither could not start on your phone", detail: error, lasts: SETTLED_LASTS, tone: "problem" }),
  );
  return null;
}

export const helper: Feature = { id: ID, title: "Helper", icon: RotateCcw, Ambient };
