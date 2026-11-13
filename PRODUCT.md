# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

The desktop app is a Tauri window (React UI) that should feel at home on the Linux desktop it runs on; the phone app is native Android (Jetpack Compose).

## Users

One person and their own phones. They link one or more of their phones (for example a personal and a work phone) to their computer, keep one in focus, and switch quickly. Orither is meant for many such people, not only its author.

## Product Purpose

Orither puts the phone on the computer: messages, WhatsApp, notifications, calls, apps, screen, photos, files, clipboard, and the phone's microphone, speaker and camera, at home or across the world. It is an open source alternative to Phone Link and KDE Connect. Success is that the person stops reaching for the phone while at the computer.

## Positioning

Phone apps open as windows of their own on the computer (Integrated Mode), with their own icon and name, and a running app can be handed from the phone to the computer and back. The link survives restarts and network changes and picks the best route by itself (local network, Bluetooth, internet, relay), end to end encrypted.

## Operating Context

The desktop app lives in the tray and is opened many times a day for a few seconds at a time (open an app, answer something, copy a code), and sometimes for long sessions (WhatsApp Web, browsing photos). The phone app is opened rarely, to link a computer, allow access, or use the phone as a trackpad and keyboard for the computer.

Most used, in order: apps in Integrated Mode, WhatsApp, calls and notifications, clipboard, photos and videos.

## Capabilities and Constraints

- Linking by QR code or 8-character code; several phones per computer.
- Features: SMS (read, one-time codes), WhatsApp Web (WhatsApp's own site inside Orither, styled only with a CSS user style sheet; never scripted, never an unofficial client), notifications (click opens the app in a window), calls (answer and place cellular and app calls, audio through the computer as a Bluetooth hands-free device), apps and Integrated Mode, Send to computer, screen mirroring and control, gallery, files, clipboard sync, phone microphone/speaker/camera as computer devices, phone as trackpad and keyboard for the computer.
- A helper (engine) runs on the phone as the shell user; it needs a USB connection with USB debugging once, and is restarted over Wi-Fi after phone restarts.
- Linux (KDE Plasma first) is the tested desktop; Windows is configured but untested.
- Not affiliated with WhatsApp; WhatsApp's logo is never used as Orither's.

## Brand Commitments

- Name: Orither. Mark: a ring with a smaller device in orbit (`desktop/src/ui/Logo.tsx`).
- Colours follow the desktop fully: surfaces, text and accent come from the desktop theme (KDE `kdeglobals`, else the XDG portal), light or dark. When nothing is known, fall back to a neutral dark palette close to Breeze Dark (window #171717, view #1c1c1c, button #252525, text #ebebeb, dim #a0a0a0) with a green accent.
- The phone app mirrors the desktop's language.
- Voice: plain, short, second person, no hype, no exclamation marks.

## Evidence on Hand

- Real feature set and copy in README.md and docs/architecture.md.
- No testimonials, user counts or benchmarks exist; do not invent them.

## Product Principles

1. The computer is where the phone's things open, not a remote control for the phone.
2. Fast in, fast out: the frequent jobs (open an app, answer, copy a code) take one click from wherever the window opens.
3. Belongs to the desktop: native colours, native behaviour, nothing that looks like a website inside a window.
4. Private by construction: say plainly what each permission enables, and never surprise the person.
5. One phone in focus at a time, every phone reachable in one move.

## Accessibility & Inclusion

Keyboard reachable throughout, reduced motion honoured, contrast at least 4.5:1 for text in both light and dark desktop themes.
