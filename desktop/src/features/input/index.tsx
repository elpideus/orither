import { MousePointer2 } from "lucide-react";

import type { Feature } from "../../core/feature";
import { announce, dismiss } from "../../core/island";
import { useNotice } from "../../core/link";

const ID = "input";

/** Says so while a phone is the computer's trackpad and keyboard, since that is not visible otherwise. */
function Ambient() {
  useNotice<{ active: boolean }>(
    ID,
    "state",
    ({ active }) =>
      active
        ? announce({ id: ID, icon: MousePointer2, title: "A phone is controlling this computer", detail: "As its trackpad and keyboard", lasts: 6_000 })
        : dismiss(ID),
    { everyPhone: true },
  );
  return null;
}

export const input: Feature = { id: ID, title: "Trackpad", icon: MousePointer2, Ambient };
