import type { Transition } from "motion/react";

/**
 * Every curve and duration in the app, so everything moves as one object.
 * CSS has the same values as custom properties in `ui/app.css`.
 */
export const ease = {
  /** Things arriving or leaving: fast start, so the response feels immediate. */
  out: [0.23, 1, 0.32, 1],
  /** Things already on screen moving or resizing. */
  inOut: [0.77, 0, 0.175, 1],
  /** Sheets and drawers. */
  drawer: [0.32, 0.72, 0, 1],
} as const;

export const spring = {
  /** Anything that can be interrupted and turned around: the orrery, tab indicators, layout. */
  glide: { type: "spring", duration: 0.5, bounce: 0 },
  /** Small things answering a press. */
  snappy: { type: "spring", duration: 0.3, bounce: 0 },
  /** A physical object landing, such as a satellite reaching its orbit. */
  settle: { type: "spring", duration: 0.55, bounce: 0.15 },
} satisfies Record<string, Transition>;

export const timing = {
  /** Popovers and menus coming in. */
  pop: { duration: 0.2, ease: ease.out },
  /** Their exit, a little quicker and quieter. */
  popOut: { duration: 0.15, ease: ease.out },
  /** One page giving way to another. */
  page: { duration: 0.24, ease: ease.out },
} satisfies Record<string, Transition>;
