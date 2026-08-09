//! Embeds the dashboard SPA static files into the binary at compile time.
//!
//! Phase 8 extracts the 44k-line inline SPA from amux-server.py into
//! `static/`. Until then this serves a placeholder shell so Phase 0's
//! server/health/Playwright items have a real page to load.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct DashboardAssets;

/// Version stamp for cache busting. Stamped from the crate version at build
/// time; the service worker CACHE name derives from this one value so the
/// two can never drift (Python required bumping APP_VER and sw.js CACHE
/// together by hand — L5 class fix).
pub const APP_VER: &str = env!("CARGO_PKG_VERSION");
