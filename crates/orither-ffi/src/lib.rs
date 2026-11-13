//! The Android app's window into [`orither_link`].
//!
//! Kotlin talks to this crate through `dev.orither.link.Native`. Values cross as
//! JSON text, so a new feature never touches this file. Anything that takes a
//! while answers through a `Reply` object, and every call into Kotlin happens on
//! one thread that belongs to this crate, so no runtime worker ever blocks on
//! the JVM.

mod bridge;
mod native;
