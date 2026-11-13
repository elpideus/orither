---
name: Orither
description: The computer at the centre, each linked phone orbiting it at the distance of its route, in the desktop's own colours.
colors:
  window-bg: "#171717"
  view-surface: "#1c1c1c"
  button-surface: "#252525"
  text: "#ebebeb"
  text-dim: "#a0a0a0"
  accent: "#3ddc84"
  accent-contrast: "#05230f"
  danger: "#e5484d"
  positive: "#4cc38a"
typography:
  display:
    fontFamily: "system-ui, sans-serif"
    fontSize: "40px"
    fontWeight: 500
    lineHeight: 1.08
    letterSpacing: "-0.02em"
  headline:
    fontFamily: "system-ui, sans-serif"
    fontSize: "22px"
    fontWeight: 500
    letterSpacing: "-0.01em"
  title:
    fontFamily: "system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
  body:
    fontFamily: "system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.45
  body-secondary:
    fontFamily: "system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
  label:
    fontFamily: "system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 500
    fontFeature: "tnum"
  code:
    fontFamily: "system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    letterSpacing: "0.08em"
    fontFeature: "tnum"
rounded:
  nested: "8px"
  row: "10px"
  card: "14px"
  pane: "18px"
  dock: "24px"
spacing:
  hair: "2px"
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
components:
  button:
    backgroundColor: "{colors.button-surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
    height: "34px"
    padding: "0 14px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    rounded: "{rounded.row}"
    height: "34px"
    padding: "0 14px"
  button-primary-large:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    rounded: "{rounded.row}"
    height: "40px"
    padding: "0 18px"
  button-danger:
    backgroundColor: "{colors.danger}"
    textColor: "#ffffff"
    rounded: "{rounded.row}"
    height: "34px"
    padding: "0 14px"
  icon-button:
    textColor: "{colors.text-dim}"
    rounded: "{rounded.row}"
    size: "34px"
  field:
    backgroundColor: "{colors.button-surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
    height: "34px"
    padding: "0 12px"
  pane:
    backgroundColor: "{colors.view-surface}"
    rounded: "{rounded.pane}"
  row:
    rounded: "{rounded.row}"
    padding: "8px 10px"
    height: "56px"
  tabs:
    backgroundColor: "{colors.view-surface}"
    rounded: "{rounded.card}"
    padding: "4px"
  tab-active:
    backgroundColor: "{colors.button-surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
    height: "34px"
    padding: "0 12px"
  dock:
    backgroundColor: "{colors.view-surface}"
    rounded: "{rounded.dock}"
    padding: "12px 12px 10px"
  popover:
    backgroundColor: "{colors.button-surface}"
    rounded: "{rounded.card}"
  toast:
    backgroundColor: "{colors.button-surface}"
    rounded: "{rounded.card}"
    padding: "10px 8px 10px 10px"
  bubble-mine:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    padding: "8px 12px"
  phone-mark-focused:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    size: "20px"
---

# Design System: Orither

## Overview

**Creative North Star: "The Orrery"**

The computer is the centre and each linked phone orbits it at the distance of its route. Everything else is the desktop's own window: its surfaces, its text, its accent, light or dark, so Orither reads as part of the person's computer rather than a branded app laid over it. The orrery (132px) in the top band is the one drawn device in the system; it says at once which phone is in focus and how it is reached, and its satellite, a phone's initial in an accent-outlined circle, recurs as the mark on every item that came from a phone.

Density is that of a desktop utility opened many times a day for seconds: compact rows (56px), 34px controls, a dock of app icons as the largest targets on screen. Depth is quiet: surface steps and hairlines carry the structure, and shadow is kept for things that float above the window. Type is the system UI sans at three weights with tabular numerals wherever numbers change. Motion answers at once on the way in, gets out of the way on the way out, and never moves for its own sake.

The category's labelled sidebar with a phone header on top is refused; navigation is a segmented row of destination tabs at the top right, beside the orrery.

**Key Characteristics:**
- Seven colours from the desktop theme; every other colour derived from them with `color-mix` in OKLab.
- The orrery as signature: route = ring distance, focused phone filled and at the front.
- Phone-initial marks tag every item that came from a phone.
- One accent action per view.
- Depth from surface steps and 1px hairlines; shadow only on floating layers.
- System UI sans 400/500/600, tabular numerals.
- Radii 10/14/18 for row/card/pane, 8 nested inside them, 24 for the outermost floating containers.
- Strong ease-out for enter and exit, zero-bounce springs for interruptible motion, no idle loops.

## Colors

The palette is borrowed, not chosen: seven roles come from the desktop (KDE `kdeglobals`, else the XDG portal) and are applied as `--bg`, `--surface`, `--surface-2`, `--text`, `--text-dim`, `--accent`, `--accent-contrast`. The values in the frontmatter are the fallback used when the desktop reports nothing: Breeze-dark neutrals with a green accent. Screens must hold up under any accent the desktop supplies (the shipped review shows the same window in a pink accent on dark and a blue accent on light).

### Primary
- **Desktop Accent** (fallback Android Green, `accent`): the one action per view (Answer, Call, View screen), the focused phone's filled satellite and mark, outgoing message bubbles, unread counts, the switch's on state, the field focus border (70% mix), text selection (38% mix).
- **On Accent** (`accent-contrast`): text and icons sitting on accent fill only.

### Neutral
- **Window** (`window-bg`): the window background and the satellite's interior, so a satellite cuts its ring cleanly.
- **View Surface** (`view-surface`): panes, the dock, the tab rail. The first step up.
- **Button Surface** (`button-surface`): the inset fill (`--inset`) for avatars, the active tab mark, incoming bubbles; also the raised surface of popovers, toasts and tips in dark.
- **Text** (`text`) and **Dim Text** (`text-dim`): primary copy; secondary copy, meta, times, idle icons.

### Derived roles (from the seven, not themed separately)
- **Hairline** (`--line`, text at 9% dark / 11% light) and **Strong hairline** (`--line-strong`, 16%): pane outlines, section dividers, unused and used orrery rings.
- **Hover** (text 6% / 5%) and **Press** (text 10% / 9%): row hover; the neutral button and field fill.
- **Accent Soft** (accent 15%) and **Accent Ink** (accent 82% into text dark, 70% light): selected rows and active icon buttons; accent-coloured text on a neutral ground.
- **Raised** (`--raised`): `button-surface` lifted 3% toward text in dark; equals `view-surface` in light, where the float shadow does the separating.

### Status
- **Danger** (`danger`, light `#d13438`): hang up, low battery, problems. **Danger Soft** at 16% for tinted wells.
- **Positive** (`positive`, light `#16a34a`): the online dot only. It is not a second accent.

### Named Rules
**The Borrowed Palette Rule.** Only the seven desktop roles, plus danger and positive, are literal colours. Every other colour is a `color-mix` of them. Never add a hex that does not follow the desktop.

**The One Accent Action Rule.** Each view has at most one accent-filled button. Everything else is neutral (`press` fill), quiet (transparent) or danger. The shipped primary buttons are Answer, Call, View screen and a toast's single action.

## Typography

**Display Font:** system-ui (with sans-serif)
**Body Font:** system-ui (with sans-serif)

**Character:** The desktop's own UI face, so Orither speaks in the same voice as every other window. Hierarchy comes from size and three weights, never from a display face or uppercase.

### Hierarchy
- **Display** (500, 40px, 1.08, -0.02em): the pairing screen's heading only; balanced wrap, 14ch measure.
- **Headline** (500, 22px, -0.01em): the focused phone's name beside the orrery.
- **Title** (600, 15px): pane heads, empty-state titles; 16px/600 for a popover's head.
- **Body** (400, 14px, 1.45): the base; row names at 600, option labels at 500.
- **Body secondary** (400, 13px): row previews, explanations, identity facts, pane counts.
- **Label** (500 or 400, 12px, tabular): times, meta, section headings in sheets, dock app names, tips.
- **Code** (600, 18px, 0.08em, tabular): one-time codes lifted out of a message; the pairing code is the same treatment at 30px/500.

### Named Rules
**The Three Weights Rule.** 400, 500, 600. Nothing lighter, nothing heavier in running type.

**The Tabular Rule.** Any number that changes or lines up (times, counts, battery, durations, codes) uses `font-variant-numeric: tabular-nums`.

## Layout

A two-row shell: a top band (14px 20px 10px 18px padding) holding the orrery, the identity block beside it, a flexible spacer, then the tab rail and the settings button at the right; below it a stage (4px 20px 20px) that holds one page at a time. Pages cross-fade in place. Home is a centred dock above two panes (1.35fr WhatsApp, 1fr calls and notifications; equal below 1060px) with 16px between dock and panes and 12px between panes. List-and-detail pages use a split of 260-340px list and flexible detail, 12px apart. Inside lists, rows sit 2px apart in a 6px-padded column.

Spacing steps are 2, 4, 8, 12, 16 and 20px, with 6, 10, 14 and 18 used as half-steps inside components. Below 1180px, inactive tabs collapse to their icon; the current tab keeps its label.

## Elevation & Depth

Flat by default. The window background, view surface and button surface are three tonal steps, and every pane, the dock and the tab rail carries a 1px inner hairline (`inset 0 0 0 1px var(--line)`) instead of a shadow. Shadow exists only where something floats above the window: popovers, toasts, tips. Small sliding marks (the switch thumb, the audio-route segment) carry a 1-2px contact shadow so they read as objects on a track.

### Shadow Vocabulary
- **Float** (dark: `0 1px 0 color-mix(in oklab, var(--text) 6%, transparent) inset, 0 16px 40px rgb(0 0 0 / 0.36), 0 2px 8px rgb(0 0 0 / 0.24)`; light: `0 0 0 1px color-mix(in oklab, var(--text) 8%, transparent), 0 16px 40px rgb(0 0 0 / 0.14), 0 2px 6px rgb(0 0 0 / 0.08)`): popovers, toasts, tips.
- **Contact** (`0 1px 2px rgb(0 0 0 / 0.2)` to `0.3`): a thumb or segment sliding inside a track.
- **Hairline** (`inset 0 0 0 1px var(--line)`): the outline of every resting surface.

### Named Rules
**The Floating-Only Shadow Rule.** A surface that sits in the layout never casts a shadow. If it does not float over other content, it gets a surface step and a hairline.

## Shapes

Rounded, soft-cornered rectangles nested by size: rows, controls and tabs at 10px; cards, options, popovers, toasts and keypad keys at 14px; panes at 18px; the dock and the pairing card at 24px. Anything sitting inside a 10px element (tips, tab and segment marks, small chips) takes 8px. Circles are reserved for people and phones: avatars, satellites, phone marks, the status dot. App icons keep a 24% squircle. On the orrery, the device at the centre is a small rounded rectangle and every device orbiting it is a circle.

## Components

### Buttons
Compact and solid, sinking slightly under the press.
- **Shape:** 10px corners, 34px tall (40px large), 14px side padding (18px large), weight 500.
- **Neutral (default):** `press` fill (text at 10%), hover to text at 14%.
- **Primary:** accent fill with on-accent text; hover mixes 12% text in. One per view.
- **Danger:** danger fill, white text; hover mixes 12% white in.
- **Quiet:** transparent, hover fill.
- **Press:** scale 0.97 (icon buttons 0.94, keys 0.96) over 150ms on the ease-out curve. Disabled at 45% opacity.
- **Icon button:** 34px square, dim icon; active state is accent soft with accent ink. Lucide icons at 1.75 stroke.

### Inputs / Fields
- **Style:** 34px, 10px corners, `press` fill, transparent 1px border.
- **Focus:** border to accent at 70%, fill to view surface. Global focus ring elsewhere: 2px text at 70%, 2px offset.
- **Search:** a 16px dim magnifier inset 10px, text at 34px.

### Switches
40 by 24px track in `line-strong`; 18px thumb in text with a contact shadow. On: track takes the accent, thumb takes on-accent and slides 16px, 200ms ease-out.

### Panes / Containers
- **Corner Style:** 18px.
- **Background:** view surface.
- **Shadow Strategy:** none; inner hairline.
- **Internal Padding:** head 10px 12px 6px 18px at 52px min height with a 15px/600 title and a dim tabular count; lists inside pad 6px.

### Rows
A 40px leading avatar or icon, name at 600 with a 12px tabular time at the end, a 13px dim preview below, 56px min height, 10px corners. Hover takes the hover fill; selected takes accent soft. Unread previews switch to text colour at 500.

### Navigation
A segmented tab rail: view surface, 14px corners, 4px padding, hairline. Tabs are 34px, dim with icon and label; the current tab is text-coloured with an accent-ink icon over a sliding inset mark (10px, button surface, 1px top highlight) that moves between tabs on the glide spring. Settings sits apart as an icon button.

### Popovers, Toasts, Tips
Raised surface with the float shadow. Popovers (14px) grow from their trigger's corner: scale 0.96 and 4px up to rest in 200ms, out in 150ms. Toasts (14px) stack at the bottom centre, at most three, 440px wide, with a 36px tinted glyph well (accent soft, or danger soft for problems), one optional primary action, and a 4px accent progress bar. Tips are 8px, 12px text.

### Message bubbles
16px corners, 8px 12px padding, max 72% or 560px. Incoming on the inset fill; outgoing on accent with on-accent text. A detected code sits beneath in an accent-soft strip with a 30px accent Copy button.

### The Orrery (signature)
Three hairline concentric rings at 0.42, 0.64 and 0.86 of the radius: inner for same Wi-Fi, middle for Bluetooth, outer for internet and relay (and for a phone that is not connected). A ring a phone is on strengthens to `line-strong`. The computer is a small rounded square at the centre in dim text at 70%. Each phone is a satellite, 21% of the orrery's size: a 2px accent outline on the window colour carrying the phone's initial in accent ink; the focused phone is filled with accent and sits at the front position (-40 degrees, up and right, nearest its name). An offline phone's outline drops to `line-strong` and its initial to dim. Choosing a phone swings every satellite along its ring by the shortest way on the glide spring while the page cross-fades; a route change moves the satellite to its new ring on the settle spring. A ringing phone sends a 2px accent ripple out (scale 1 to 2.4, 1.4s) only while it rings. Hovering an unfocused satellite shows a tip with its name and route; the hit area extends 8px past the circle.

### Phone mark
The satellite on its own, 20px by default with a 1.5px accent outline and the initial at 48% of its size: filled for the focused phone, `line-strong` and dim when offline. It tags every item that came from a phone (notifications, calls, settings) so a person with several phones always knows the source.

### Dock
The home row of the most-used apps: view surface, 24px corners, hairline, 84px app cells with app icons and 12px dim names, an "All apps" cell after a divider. The largest targets in the window.

### Motion
- **Enter/exit:** `cubic-bezier(0.23, 1, 0.32, 1)`; popovers 200ms in and 150ms out, pages 240ms in (6px rise) and 120ms out, toasts 260ms in and 160ms out.
- **Things already on screen moving:** `cubic-bezier(0.77, 0, 0.175, 1)`; sheets and drawers `cubic-bezier(0.32, 0.72, 0, 1)`.
- **Interruptible motion:** springs with zero bounce, glide (0.5s; orrery turn, tab mark, layout, and the app-wide default) and snappy (0.3s; presses, progress, small swaps).
- **Landing:** settle (0.55s, bounce 0.15) only for a satellite arriving on a new ring.
- **Reduced motion:** springs collapse to instant; the ripple holds still at 1.5x and 60% opacity.
- **Theme switch:** colours change at once, with transitions suspended for two frames.

### Android companion
The phone app mirrors the language in Compose, reviewed on a POCO F7. Neutrals are the desktop's (dark `#171717` / `#1C1C1C` / `#252525`; light `#F2F2F3` / `#FFFFFF` / `#E9E9EB`), following the phone's light or dark setting. The accent comes from the phone's own theme through Material You on Android 12 and later, the way the desktop's comes from KDE, with the green as the fallback below that. Controls are Material 3: the one `Action` per screen is a `Button`, everything else a `FilledTonalButton` (`Quiet`), keys are `Surface(onClick)` with roles (sticky modifiers as switches), close is an `IconButton`; every target is at least 48dp, list rows 56dp, and status is words ("On", "Ready") rather than toggles that do nothing. Type uses `MaterialTheme.typography` roles in sp. Shapes: Tile 18dp, Card 14dp, Small 10dp, with a 1dp border on tiles in place of shadow. The orrery is the same picture from the other end: the phone at the centre as an upright rounded rectangle, its computers orbiting on the same three rings; the computer in front is filled, and picking another turns the ring until it is in front. The Trackpad screen keeps the keyboard up from entry, the trackpad fills the space above it, and two rows of keys sit between. Springs: snappy (damping 1, stiffness 700), page (damping 1, stiffness 260), pop (damping 0.8, stiffness 300) for orbit landing.

## Do's and Don'ts

### Do:
- **Do** take every colour from the seven desktop roles or a `color-mix` of them; test any new screen under a light desktop and a non-green accent.
- **Do** give each view at most one accent-filled action.
- **Do** tag anything that came from a phone with its phone mark, filled when that phone is in focus.
- **Do** separate resting surfaces with a surface step and a 1px hairline; reserve the float shadow for popovers, toasts and tips.
- **Do** keep radii on the 8/10/14/18/24 ladder, nested smaller inside larger.
- **Do** set changing numbers in tabular numerals.
- **Do** enter and exit on the strong ease-out curve and move interruptible things on zero-bounce springs.
- **Do** honour reduced motion with instant springs and still ripples.

### Don't:
- **Don't** add a labelled sidebar with a phone header; the orrery and the tab rail are the navigation.
- **Don't** hard-code a brand colour or a second accent; positive green is for the online dot only.
- **Don't** put shadows on panes, rows, the dock or the tab rail.
- **Don't** loop any animation idly; the ripple runs only while a phone rings, and the waiting satellite and spinner only while something is pending.
- **Don't** draw an orbiting device as anything but a circle, or the centre device as anything but a small rounded rectangle.
- **Don't** use weights outside 400/500/600 or uppercase labels in running type.
