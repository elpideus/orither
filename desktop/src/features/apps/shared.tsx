import { useSyncExternalStore } from "react";

import { call, focusedId, media, useCall } from "../../core/link";
import { remember, setting, useSetting } from "../../core/settings";

export interface App {
  package: string;
  label: string;
}

export function useApps() {
  return useCall<App[]>("apps", "list");
}

/** Integrated Mode is what Orither is used for most, so apps open in windows unless turned off. */
export function useIntegrated() {
  return useSetting("apps.integrated", true);
}

const RECENT_KEPT = 24;
const recentListeners = new Set<() => void>();
const recentKey = (phone: string | undefined) => `apps.recent.${phone ?? "none"}`;
let recentCache: { phone: string | undefined; list: string[] } = { phone: undefined, list: [] };

function recentOf(phone: string | undefined): string[] {
  if (recentCache.phone !== phone) recentCache = { phone, list: setting<string[]>(recentKey(phone), []) };
  return recentCache.list;
}

/** The phone's apps opened from here, most recent first. */
export function useRecent(): string[] {
  return useSyncExternalStore(
    (notify) => {
      recentListeners.add(notify);
      return () => void recentListeners.delete(notify);
    },
    () => recentOf(focusedId()),
  );
}

function used(app: App) {
  const phone = focusedId();
  const list = [app.package, ...recentOf(phone).filter((name) => name !== app.package)].slice(0, RECENT_KEPT);
  recentCache = { phone, list };
  remember(recentKey(phone), list);
  recentListeners.forEach((notify) => notify());
}

/** Opens an app on the phone, or in a window of its own here. */
export function launch(app: App, integrated: boolean) {
  used(app);
  return integrated ? call("screen", "app", { ...app }) : call("apps", "open", { package: app.package });
}

export function Icon({ app, size }: { app: App; size: number }) {
  return (
    <img
      className="app-icon"
      src={media.url("apps", "icon", { package: app.package })}
      alt=""
      width={size}
      height={size}
      draggable={false}
      loading="lazy"
    />
  );
}
