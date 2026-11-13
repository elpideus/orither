import { open as pickFiles } from "@tauri-apps/plugin-dialog";
import { ChevronRight, Download, File, FileArchive, FileAudio, FileImage, FileText, FileVideo, Folder, FolderOpen, Trash2, TriangleAlert, Upload, type LucideIcon } from "lucide-react";
import { motion } from "motion/react";
import { useState } from "react";

import type { Feature } from "../../core/feature";
import { announce } from "../../core/island";
import { call, useCall } from "../../core/link";
import { spring } from "../../core/motion";
import { bytes, Button, Empty, IconButton, Loaded, when } from "../../ui/kit";
import "./files.css";

interface Root {
  name: string;
  path: string;
  free: number;
  total: number;
}

interface Entry {
  name: string;
  path: string;
  folder: boolean;
  size: number;
  modified: number;
  mime: string | null;
}

function iconFor({ folder, mime }: Entry): LucideIcon {
  if (folder) return Folder;
  if (mime?.startsWith("image/")) return FileImage;
  if (mime?.startsWith("video/")) return FileVideo;
  if (mime?.startsWith("audio/")) return FileAudio;
  if (mime?.startsWith("text/") || mime === "application/pdf") return FileText;
  if (mime?.includes("zip") || mime?.includes("compressed")) return FileArchive;
  return File;
}

const source = ({ path, name, size }: Entry) => ({ feature: "files", method: "read", body: { path }, name, size });

/** Asks once more, right where the pointer already is. */
function Delete({ onDelete }: { onDelete: () => void }) {
  const [armed, setArmed] = useState(false);
  if (!armed) return <IconButton icon={Trash2} label="Delete" onClick={() => setArmed(true)} />;
  return (
    <Button kind="danger" onClick={onDelete} onMouseLeave={() => setArmed(false)} onBlur={() => setArmed(false)} autoFocus>
      Delete?
    </Button>
  );
}

function Row({ entry, onOpen, onDelete }: { entry: Entry; onOpen: () => void; onDelete: () => void }) {
  const Icon = iconFor(entry);
  return (
    <div className="files-row" role="button" tabIndex={0} onDoubleClick={onOpen} onKeyDown={(event) => event.key === "Enter" && onOpen()}>
      <Icon size={19} data-folder={entry.folder} />
      <strong onClick={entry.folder ? onOpen : undefined}>{entry.name}</strong>
      <span className="dim">{entry.folder ? "" : bytes(entry.size)}</span>
      <span className="dim">{when(entry.modified)}</span>
      <div className="files-actions">
        {!entry.folder && <IconButton icon={Download} label="Save to this computer" onClick={() => void call("transfer", "download", source(entry))} />}
        <Delete onDelete={onDelete} />
      </div>
    </div>
  );
}

function Browser({ root }: { root: Root }) {
  const [path, setPath] = useState(root.path);
  const listing = useCall<Entry[]>("files", "list", { path });
  const relative = path.slice(root.path.length).split("/").filter(Boolean);
  const crumbs = relative.map((name, index) => ({ name, path: `${root.path}/${relative.slice(0, index + 1).join("/")}` }));

  const open = (entry: Entry) => {
    if (entry.folder) setPath(entry.path);
    else void call("transfer", "open", source(entry));
  };

  const upload = async () => {
    const picked = await pickFiles({ multiple: true });
    await Promise.all((picked ?? []).map((local) => call("transfer", "upload", { local, feature: "files", method: "write", folder: path })));
    listing.reload();
  };

  const remove = async (entry: Entry) => {
    await call("files", "delete", { path: entry.path }).catch((error: string) =>
      announce({ id: "files-delete", icon: TriangleAlert, title: `Couldn't delete ${entry.name}`, detail: error, lasts: 8_000 }),
    );
    listing.reload();
  };

  return (
    <>
      <div className="pane-head files-head">
        <nav className="files-crumbs">
          <button onClick={() => setPath(root.path)}>{root.name}</button>
          {crumbs.map((crumb) => (
            <motion.span key={crumb.path} layout="position" initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={spring.snappy}>
              <ChevronRight size={14} className="dim" />
              <button onClick={() => setPath(crumb.path)}>{crumb.name}</button>
            </motion.span>
          ))}
        </nav>
        <span className="spacer" />
        <span className="count num">{bytes(root.free)} free</span>
        <Button onClick={upload}>
          <Upload size={15} />
          Send here
        </Button>
      </div>
      <Loaded remote={listing} icon={FolderOpen}>
        {(entries) =>
          entries.length === 0 ? (
            <Empty icon={Folder} title="This folder is empty" />
          ) : (
            <div className="files-list">
              {entries.map((entry) => (
                <Row key={entry.path} entry={entry} onOpen={() => open(entry)} onDelete={() => void remove(entry)} />
              ))}
            </div>
          )
        }
      </Loaded>
    </>
  );
}

function Page() {
  const roots = useCall<Root[]>("files", "roots");
  return (
    <section className="pane files-page">
      <Loaded remote={roots} icon={FolderOpen}>
        {(data) => (data[0] ? <Browser root={data[0]} /> : <Empty icon={FolderOpen} title="No storage found" />)}
      </Loaded>
    </section>
  );
}

export const files: Feature = { id: "files", title: "Files", icon: FolderOpen, Page, tab: true };
