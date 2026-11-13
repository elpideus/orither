import { AppWindow, LayoutGrid } from "lucide-react";
import { useMemo, useState } from "react";

import type { Feature } from "../../core/feature";
import { Empty, Loaded, Search, Toggle } from "../../ui/kit";
import { Icon, launch, useApps, useIntegrated } from "./shared";
import "./apps.css";

function Page() {
  const apps = useApps();
  const [integrated, setIntegrated] = useIntegrated();
  const [search, setSearch] = useState("");
  const shown = useMemo(() => {
    const words = search.trim().toLowerCase();
    return apps.data?.filter(({ label }) => label.toLowerCase().includes(words));
  }, [apps.data, search]);

  return (
    <section className="pane apps-page">
      <div className="pane-head apps-head">
        <div className="apps-search">
          <Search value={search} onChange={setSearch} placeholder={apps.data ? `Search ${apps.data.length} apps` : "Search apps"} autoFocus />
        </div>
        <span className="spacer" />
        <label className="apps-mode">
          <AppWindow size={17} />
          <span>
            <strong>Integrated Mode</strong>
            <span className="dim">{integrated ? "Apps open in windows here" : "Apps open on the phone"}</span>
          </span>
          <Toggle on={integrated} label="Integrated Mode" onChange={setIntegrated} />
        </label>
      </div>
      <Loaded remote={apps} icon={LayoutGrid}>
        {() =>
          shown?.length ? (
            <div className="apps-grid">
              {shown.map((app) => (
                <button key={app.package} className="apps-item" onClick={() => void launch(app, integrated)}>
                  <Icon app={app} size={56} />
                  <span>{app.label}</span>
                </button>
              ))}
            </div>
          ) : (
            <Empty icon={LayoutGrid} title={`No app called “${search.trim()}”`} />
          )
        }
      </Loaded>
    </section>
  );
}

export const apps: Feature = { id: "apps", title: "Apps", icon: LayoutGrid, Page };
