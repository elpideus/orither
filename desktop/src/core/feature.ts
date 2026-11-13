import type { LucideIcon } from "lucide-react";
import type { ComponentType } from "react";

/**
 * One thing Orither can do. A feature is a folder in `features/` that exports
 * one of these, and listing it in `features/index.ts` is all it takes to appear.
 */
export interface Feature {
  id: string;
  title: string;
  icon: LucideIcon;
  /** A page of its own, reached from the tabs at the top. */
  Page?: ComponentType;
  /** Whether the tab bar shows it. Pages without a tab are reached from elsewhere. */
  tab?: boolean;
  /** Stays mounted while the window is open, for listeners and floating UI. */
  Ambient?: ComponentType;
}
