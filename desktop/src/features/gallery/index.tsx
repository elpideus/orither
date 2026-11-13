import { Download, Images, Play, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { Feature } from "../../core/feature";
import { call, media, useCall } from "../../core/link";
import { Empty, IconButton, Loaded } from "../../ui/kit";
import "./gallery.css";

interface Album {
  id: string;
  name: string;
  count: number;
}

interface Item {
  id: string;
  name: string;
  date: number;
  video: boolean;
  width: number;
  height: number;
}

const PAGE = 120;
const THUMBNAIL = 360;
const month = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" });

const thumbnail = (item: Item) => media.url("gallery", "thumbnail", { id: item.id, size: THUMBNAIL });

/** Loads a page of pictures at a time, as the end of the list comes into view. */
function useItems(album: string | undefined) {
  const [items, setItems] = useState<Item[]>([]);
  const finished = useRef(false);
  const busy = useRef(false);

  const more = useCallback(async () => {
    if (busy.current || finished.current) return;
    busy.current = true;
    const next = await call<Item[]>("gallery", "items", { album, offset: items.length, limit: PAGE });
    finished.current = next.length < PAGE;
    busy.current = false;
    setItems((items) => [...items, ...next]);
  }, [album, items.length]);

  useEffect(() => {
    setItems([]);
    finished.current = false;
  }, [album]);

  return { items, more };
}

/** The file itself. Videos are too large to play through the webview. */
const original = ({ id, name }: Item) => ({ feature: "gallery", method: "original", body: { id }, name });

function Viewer({ item, onClose }: { item: Item; onClose: () => void }) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", close, true);
    return () => window.removeEventListener("keydown", close, true);
  }, [onClose]);
  return (
    <motion.div className="gallery-viewer" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose}>
      {item.video ? (
        <button
          className="gallery-play"
          aria-label={`Play ${item.name}`}
          onClick={(event) => {
            event.stopPropagation();
            void call("transfer", "open", original(item));
          }}
        >
          <motion.img layoutId={item.id} src={thumbnail(item)} alt={item.name} />
          <Play size={34} fill="currentColor" />
        </button>
      ) : (
        <motion.img layoutId={item.id} src={media.url("gallery", "original", { id: item.id })} alt={item.name} style={{ backgroundImage: `url("${thumbnail(item)}")` }} />
      )}
      <div className="gallery-viewer-bar" onClick={(event) => event.stopPropagation()}>
        <span>{item.name}</span>
        <IconButton icon={Download} label="Save to this computer" onClick={() => void call("transfer", "download", original(item))} />
        <IconButton icon={X} label="Close" onClick={onClose} />
      </div>
    </motion.div>
  );
}

function Grid({ album }: { album: string | undefined }) {
  const { items, more } = useItems(album);
  const [open, setOpen] = useState<Item>();
  const end = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const observer = new IntersectionObserver(([entry]) => entry.isIntersecting && void more(), { rootMargin: "600px" });
    observer.observe(end.current!);
    return () => observer.disconnect();
  }, [more]);

  const months = useMemo(() => Map.groupBy(items, (item) => month.format(item.date)), [items]);

  return (
    <>
      <div className="gallery-scroll">
        {[...months].map(([name, items]) => (
          <section key={name}>
            <h3>{name}</h3>
            <div className="gallery-grid">
              {items.map((item) => (
                <motion.button key={item.id} className="gallery-item" layoutId={open === item ? undefined : item.id} onClick={() => setOpen(item)} whileTap={{ scale: 0.96 }}>
                  <img src={thumbnail(item)} alt={item.name} loading="lazy" />
                  {item.video && <Play size={16} fill="currentColor" />}
                </motion.button>
              ))}
            </div>
          </section>
        ))}
        <div ref={end} />
      </div>
      <AnimatePresence>{open && <Viewer item={open} onClose={() => setOpen(undefined)} />}</AnimatePresence>
    </>
  );
}

function Page() {
  const albums = useCall<Album[]>("gallery", "albums");
  const [album, setAlbum] = useState<string>();
  return (
    <section className="pane gallery-page">
      <Loaded remote={albums} icon={Images}>
        {(data) =>
          data.length === 0 ? (
            <Empty icon={Images} title="No photos or videos" />
          ) : (
            <>
              <div className="gallery-albums" role="tablist" aria-label="Albums">
                <button role="tab" aria-selected={album === undefined} onClick={() => setAlbum(undefined)}>
                  All
                </button>
                {data.map(({ id, name, count }) => (
                  <button key={id} role="tab" aria-selected={album === id} onClick={() => setAlbum(id)}>
                    {name}
                    <span className="num">{count}</span>
                  </button>
                ))}
              </div>
              <Grid key={album} album={album} />
            </>
          )
        }
      </Loaded>
    </section>
  );
}

export const gallery: Feature = { id: "gallery", title: "Photos", icon: Images, Page, tab: true };
