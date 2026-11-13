import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Theme {
  window: string;
  view: string;
  button: string;
  text: string;
  dim: string;
  accent: string;
  on_accent: string;
}

const NAMES = ["--bg", "--surface", "--surface-2", "--text", "--text-dim", "--accent", "--accent-contrast"];

/** Relative luminance, enough to tell a light desktop from a dark one. */
function luminance(color: string): number | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(color.trim());
  if (!match) return null;
  const value = parseInt(match[1], 16);
  const channel = (shift: number) => {
    const c = ((value >> shift) & 255) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
}

function apply(theme: Theme | null) {
  const root = document.documentElement;
  // Colours change all at once, never each at its own transition's pace.
  root.dataset.switching = "";
  NAMES.forEach((name) => root.style.removeProperty(name));
  if (theme) {
    const colors = [theme.window, theme.view, theme.button, theme.text, theme.dim, theme.accent, theme.on_accent];
    NAMES.forEach((name, index) => root.style.setProperty(name, colors[index]));
  }
  const light = (luminance(theme?.window ?? "") ?? 0) > 0.4;
  root.dataset.scheme = light ? "light" : "dark";
  root.style.colorScheme = light ? "light" : "dark";
  requestAnimationFrame(() => requestAnimationFrame(() => delete root.dataset.switching));
}

/** Takes on the desktop's colours now and whenever they change. */
export function followDesktopTheme() {
  void invoke<Theme | null>("theme").then(apply);
  void listen<Theme | null>("theme", (event) => apply(event.payload));
}
