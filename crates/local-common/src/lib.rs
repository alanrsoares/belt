//! `local-common` — shared helpers for the `belt` workspace.
//!
//! Deliberately dependency-free (standard library only) so every tool that
//! re-uses it builds and tests with no network access.
//!
//! Six modules live here:
//! * [`base64`]  — standard and URL-safe base64 encoding/decoding.
//! * [`cli`]     — lightweight argument cursor, flag splitting, and typo suggestions.
//! * [`json`]    — minimal zero-dependency JSON parser and serializer.
//! * [`paths`]   — consistent per-tool configuration and data locations.
//! * [`process`] — process-group spawn/cleanup helpers for subprocesses that
//!   fork children of their own.
//! * [`term`]    — terminal-aware output helpers (colour that silently
//!   disables when stdout is not a TTY or when the user has opted out via
//!   `NO_COLOR`).
//!
//! Add new cross-cutting concern as its own module and re-export the small,
//! stable public surface from here so tool crates keep a single import path.

pub mod base64;
pub mod cli;
pub mod json;
pub mod paths;
pub mod process;
pub mod term;

pub use cli::{split_flag, suggest, ArgCursor, CommonFlags};
pub use paths::{ensure_dir, tool_config_dir, tool_data_dir};
pub use term::{
    color_enabled_for, draw_meter, file_uri, format_duration, format_size, is_terminal,
    nerdfont_enabled_for, strip_ansi, terminal_columns, terminal_rows, visible_width, Colour,
    CursorGuard, Glyphs, PALETTE, SPINNER,
};

/// The shared, human-readable project name. Handy for `--help` strings and
/// log banners so every tool speaks the same language.
pub const PROJECT: &str = "belt";
