import { ArrowUpCircle, CircleCheck, Download, ExternalLink, RefreshCw, RotateCcw, TriangleAlert } from "lucide-react";
import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";

import type { Feature } from "../../core/feature";
import { announce, dismiss } from "../../core/island";
import { call, useNotice } from "../../core/link";
import { spring } from "../../core/motion";
import { Button, Toggle, bytes, when } from "../../ui/kit";
import "./updates.css";

const ID = "updates";

type Phase = "idle" | "checking" | "current" | "available" | "downloading" | "ready" | "installing" | "failed";

export interface UpdateStatus {
  current: string;
  phase: Phase;
  release: { version: string; notes: string | null; date: number | null; page: string } | null;
  progress: { done: number; total: number | null } | null;
  checked: number | null;
  error: string | null;
  auto: boolean;
  manual: string | null;
  asks_password: boolean;
}

let latest: UpdateStatus | null = null;
const listeners = new Set<(status: UpdateStatus) => void>();

function publish(status: UpdateStatus) {
  latest = status;
  listeners.forEach((notify) => notify(status));
}

function ask(method: string, body: object = {}) {
  return call<UpdateStatus>(ID, method, body).then(publish, () => {});
}

/** Where Orither's updates stand, as the computer reports it. */
export function useUpdates(): UpdateStatus | null {
  const [status, setStatus] = useState(latest);
  useEffect(() => {
    listeners.add(setStatus);
    if (!latest) void ask("status");
    return () => void listeners.delete(setStatus);
  }, []);
  useNotice<UpdateStatus>(ID, "status", publish);
  return status;
}

export const updates = {
  check: () => ask("check"),
  download: () => ask("download"),
  install: () => ask("install"),
  open: () => ask("open"),
  auto: (on: boolean) => ask("auto", { on }),
};

function percent({ done, total }: { done: number; total: number | null }) {
  return total ? Math.min(100, Math.round((done / total) * 100)) : null;
}

/** One line on where things stand. */
function headline(status: UpdateStatus): string {
  const version = status.release?.version;
  switch (status.phase) {
    case "idle":
      return "Checks for new versions shortly after Orither starts.";
    case "checking":
      return "Checking for a new version…";
    case "current":
      return status.checked ? `Up to date, checked at ${when(status.checked)}.` : "Up to date.";
    case "available":
      return status.manual ? `Orither ${version} is out.` : `Orither ${version} is available.`;
    case "downloading": {
      const done = status.progress ? percent(status.progress) : null;
      return done === null ? `Downloading Orither ${version}…` : `Downloading Orither ${version}, ${done}%`;
    }
    case "ready":
      return `Orither ${version} is ready. Restart to finish updating.`;
    case "installing":
      return `Installing Orither ${version}…`;
    case "failed":
      return status.error ?? "Something went wrong.";
  }
}

function Notes({ notes }: { notes: string }) {
  const lines = notes
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#") && !line.startsWith("**Full"));
  return (
    <ul className="update-notes">
      {lines.map((line, index) => (
        <li key={index}>{line.replace(/^[-*]\s+/, "").replace(/\*\*/g, "")}</li>
      ))}
    </ul>
  );
}

function Action({ status }: { status: UpdateStatus }) {
  const { phase, manual } = status;
  if (phase === "ready")
    return (
      <Button kind="primary" onClick={() => void updates.install()}>
        <RotateCcw size={15} />
        Restart to update
      </Button>
    );
  if (phase === "available" && manual)
    return (
      <Button onClick={() => void updates.open()}>
        <ExternalLink size={15} />
        Get it on GitHub
      </Button>
    );
  if (phase === "available")
    return (
      <Button kind="primary" onClick={() => void updates.download()}>
        <Download size={15} />
        Download
      </Button>
    );
  const busy = phase === "checking" || phase === "downloading" || phase === "installing";
  return (
    <Button onClick={() => void updates.check()} disabled={busy}>
      <RefreshCw size={15} className={phase === "checking" ? "update-spin" : undefined} />
      Check now
    </Button>
  );
}

/** The Updates section of Settings. */
export function UpdatesPane() {
  const status = useUpdates();
  if (!status) return null;
  const { phase, release, progress } = status;
  const Icon = phase === "failed" ? TriangleAlert : phase === "current" ? CircleCheck : ArrowUpCircle;
  const showRelease = release && (phase === "available" || phase === "downloading" || phase === "ready" || phase === "installing");
  return (
    <section className="pane settings-group">
      <div className="pane-head">
        <h2>Updates</h2>
      </div>
      <div className="settings-options">
        <div className="option update-status" data-phase={phase}>
          <Icon size={18} />
          <strong>Orither {status.current}</strong>
          <p>{headline(status)}</p>
          <Action status={status} />
        </div>
        {phase === "downloading" && progress && (
          <div className="update-progress" aria-hidden>
            <motion.span initial={false} animate={{ scaleX: (percent(progress) ?? 0) / 100 }} transition={spring.snappy} />
            <small className="dim num">
              {bytes(progress.done)}
              {progress.total ? ` of ${bytes(progress.total)}` : ""}
            </small>
          </div>
        )}
        {status.error && phase === "ready" && <p className="update-problem">{status.error}</p>}
        {showRelease && (
          <div className="update-release">
            <div className="update-release-head">
              <strong>What's new in {release.version}</strong>
              {release.date && <span className="dim">{when(release.date)}</span>}
              <span className="spacer" />
              <Button kind="quiet" onClick={() => void updates.open()}>
                Release page
                <ExternalLink size={14} />
              </Button>
            </div>
            {release.notes ? <Notes notes={release.notes} /> : <p className="dim">No notes for this release.</p>}
          </div>
        )}
        {status.manual ? (
          <p className="update-manual dim">{status.manual}</p>
        ) : (
          <div className="option">
            <Download size={18} />
            <strong>Download updates automatically</strong>
            <p>
              New versions download in the background and install when you restart Orither, or by themselves the next time it starts with
              this computer{status.asks_password ? ". Installing asks for your password" : ""}.
            </p>
            <Toggle on={status.auto} label="Download updates automatically" onChange={(on) => void updates.auto(on)} />
          </div>
        )}
      </div>
    </section>
  );
}

/** Says so once an update is ready, with a button that installs it. */
function Ambient() {
  const status = useUpdates();
  const told = useRef<string | null>(null);
  const version = status?.release?.version ?? null;
  const phase = status?.phase;
  useEffect(() => {
    if (phase === "ready" && version && told.current !== version) {
      told.current = version;
      announce({
        id: ID,
        icon: ArrowUpCircle,
        title: `Orither ${version} is ready`,
        detail: "Restart to finish updating. It takes a few seconds.",
        action: { label: "Restart", run: () => void updates.install() },
      });
    } else if (phase === "installing") {
      announce({ id: ID, icon: RotateCcw, title: `Installing Orither ${version}`, detail: "Orither opens again in a moment." });
    } else if (phase !== "ready") {
      dismiss(ID);
    }
  }, [phase, version]);
  return null;
}

export const updatesFeature: Feature = { id: ID, title: "Updates", icon: ArrowUpCircle, Ambient };
