//! # Koda
//!
//! A modern, lightweight, terminal-native IDE.
//!
//! Koda is built around a few core ideas:
//!
//! * **Zero configuration** — supported languages work out of the box.
//! * **Koda owns language intelligence** — detection and providers are first-class.
//! * **Terminal-native** — the UI cooperates with the terminal instead of fighting it.
//!
//! The crate is split into clear subsystems so that language-specific behaviour never
//! leaks into the editor core:
//!
//! * [`app`] — application state and the event loop.
//! * [`editor`] — the text editing foundation (buffers, cursors, undo/redo).
//! * [`language`] — language detection and language providers.
//! * [`project`] — workspaces, projects and the file tree.
//! * [`ui`] — rendering for every surface.
//! * [`commands`] — the extensible command system.
//! * [`git`] — lightweight git integration.

pub mod app;
pub mod background;
pub mod commands;
pub mod editor;
pub mod filesystem;
pub mod git;
pub mod language;
pub mod project;
pub mod session;
pub mod terminal;
pub mod ui;
