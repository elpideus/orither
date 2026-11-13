import type { LucideIcon } from "lucide-react";
import { useSyncExternalStore } from "react";

/** Something worth a moment of attention, shown as a toast at the bottom of the window. */
export interface Activity {
  id: string;
  icon: LucideIcon;
  title: string;
  detail?: string;
  /** From 0 to 1, for something under way. */
  progress?: number;
  action?: { label: string; run: () => void };
  /** How long it stays, in milliseconds. Until dismissed if left out. */
  lasts?: number;
  tone?: "problem";
}

let activities: Activity[] = [];
const listeners = new Set<() => void>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();

function publish(next: Activity[]) {
  activities = next;
  listeners.forEach((notify) => notify());
}

/** Shows an activity, or updates the one with the same id in place. */
export function announce(activity: Activity) {
  clearTimeout(timers.get(activity.id));
  const known = activities.some(({ id }) => id === activity.id);
  publish(known ? activities.map((shown) => (shown.id === activity.id ? activity : shown)) : [...activities, activity]);
  if (activity.lasts) timers.set(activity.id, setTimeout(() => dismiss(activity.id), activity.lasts));
}

export function dismiss(id: string) {
  clearTimeout(timers.get(id));
  publish(activities.filter((activity) => activity.id !== id));
}

function subscribe(notify: () => void) {
  listeners.add(notify);
  return () => void listeners.delete(notify);
}

/** Everything announced and not yet gone, oldest first. */
export function useActivities(): Activity[] {
  return useSyncExternalStore(subscribe, () => activities);
}
