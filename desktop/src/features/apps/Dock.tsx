import { LayoutGrid } from "lucide-react";
import { useMemo } from "react";

import { usePhone } from "../../core/link";
import { useNav } from "../../core/nav";
import { Icon, launch, useApps, useIntegrated, useRecent, type App } from "./shared";
import "./apps.css";

const DOCK_SIZE = 10;

/** The phone's apps opened here lately first, then the rest in the phone's order. */
function docked(apps: App[], recent: string[]): App[] {
  const byPackage = new Map(apps.map((app) => [app.package, app]));
  const first = recent.map((name) => byPackage.get(name)).filter((app): app is App => app !== undefined);
  const rest = apps.filter((app) => !recent.includes(app.package));
  return [...first, ...rest].slice(0, DOCK_SIZE);
}

/** The home page's first row: the apps most likely wanted, as the largest targets on screen. */
export function AppDock() {
  const { go } = useNav();
  const phone = usePhone();
  const apps = useApps();
  const recent = useRecent();
  const [integrated] = useIntegrated();
  const shown = useMemo(() => docked(apps.data ?? [], recent), [apps.data, recent]);

  return (
    <nav className="dock" aria-label="Apps">
      {shown.length === 0 ? (
        <p className="dock-note dim">
          {phone?.online ? (apps.error ?? "Loading the phone's apps…") : `Apps show here when ${phone?.device.name ?? "your phone"} connects.`}
        </p>
      ) : (
        shown.map((app) => (
          <button key={app.package} className="dock-app" onClick={() => void launch(app, integrated)} title={integrated ? `Open ${app.label} in a window` : `Open ${app.label} on the phone`}>
            <Icon app={app} size={52} />
            <span>{app.label}</span>
          </button>
        ))
      )}
      <span className="dock-divider" />
      <button className="dock-app" onClick={() => go("apps")}>
        <span className="dock-all">
          <LayoutGrid size={22} />
        </span>
        <span>All apps</span>
      </button>
    </nav>
  );
}
