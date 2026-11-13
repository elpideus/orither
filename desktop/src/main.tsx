import { MotionConfig } from "motion/react";
import { createRoot } from "react-dom/client";

import { followDesktopTheme } from "./core/theme";
import { spring } from "./core/motion";
import { App } from "./ui/App";
import "./ui/app.css";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("./dev/phone");
}

followDesktopTheme();

createRoot(document.getElementById("root")!).render(
  <MotionConfig transition={spring.glide} reducedMotion="user">
    <App />
  </MotionConfig>,
);
