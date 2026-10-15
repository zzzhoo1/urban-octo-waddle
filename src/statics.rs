use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

/// Next.js static export embedded into the binary (built by CI into web/out).
#[derive(Embed)]
#[folder = "web/out"]
struct AdminAssets;

/// Serve /admin/ (the panel index).
pub async fn admin_index() -> Response {
    match AdminAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => (StatusCode::NOT_FOUND, "admin panel not built").into_response(),
    }
}

/// Serve /admin/{*path} static assets, with SPA fallback to index.html
/// for client-side routes (/admin/accounts, /admin/models, ...).
pub async fn admin_static(Path(path): Path<String>) -> Response {
    let path = path.trim_start_matches('/');
    if let Some(content) = AdminAssets::get(path) {
        return serve(path, &content.data);
    }
    // Try path/index.html (Next.js static export layout).
    let nested = format!("{}/index.html", path.trim_end_matches('/'));
    if let Some(content) = AdminAssets::get(&nested) {
        return Html(content.data.into_owned()).into_response();
    }
    // Try path.html (Next export trailingSlash=false layout).
    let html = format!("{}.html", path.trim_end_matches('/'));
    if let Some(content) = AdminAssets::get(&html) {
        return Html(content.data.into_owned()).into_response();
    }
    // SPA fallback.
    match AdminAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn serve(path: &str, data: &[u8]) -> Response {
    let mime = mime_from_path(path);
    (
        [(header::CONTENT_TYPE, mime)],
        data.to_vec(),
    )
        .into_response()
}

fn mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}
