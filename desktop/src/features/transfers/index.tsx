import { ArrowDownUp, TriangleAlert } from "lucide-react";

import type { Feature } from "../../core/feature";
import { announce, dismiss } from "../../core/island";
import { useNotice } from "../../core/link";
import { bytes } from "../../ui/kit";

interface Progress {
  id: number;
  name: string;
  done: number;
  total: number | null;
  finished: boolean;
}

const WARNING_LASTS = 8_000;

/** Puts what is moving between the devices, and what went wrong, on the island. */
function Ambient() {
  useNotice<Progress>("transfer", "progress", ({ id, name, done, total, finished }) => {
    const key = `transfer-${id}`;
    if (finished) dismiss(key);
    else announce({ id: key, icon: ArrowDownUp, title: name, detail: bytes(done), progress: total ? done / total : undefined });
  });
  const warn = ({ error }: { error: string }) =>
    announce({ id: "problem", icon: TriangleAlert, title: "That didn't work", detail: error, lasts: WARNING_LASTS, tone: "problem" });
  useNotice("transfer", "failed", warn);
  useNotice("screen", "failed", warn);
  useNotice("hardware", "failed", warn);
  useNotice("whatsapp", "failed", warn);
  return null;
}

export const transfers: Feature = { id: "transfers", title: "Transfers", icon: ArrowDownUp, Ambient };
