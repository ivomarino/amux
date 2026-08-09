//! Embedded dashboard serving (RR-0021). Files come from amux-dashboard's
//! `static/` at compile time — the binary is the whole deploy artifact.

use amux_dashboard::DashboardAssets;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", axum::routing::get(|| async { serve("index.html") }))
        .route("/{*path}", axum::routing::get(serve_path))
}

async fn serve_path(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve(path)
}

fn serve(path: &str) -> Response {
    match DashboardAssets::get(path) {
        Some(content) => {
            let mime = mime_for(path);
            (
                [(header::CONTENT_TYPE, mime)],
                content.data.into_owned(),
            )
                .into_response()
        }
        // SPA fallback: unknown paths get the shell so client routing works
        // offline-first; API 404s are handled by the API router before this.
        None => match DashboardAssets::get("index.html") {
            Some(index) => (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                index.data.into_owned(),
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
