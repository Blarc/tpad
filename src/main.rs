mod auth;
mod config;
mod error;
mod fs_store;

use std::{io, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Query, State,
        rejection::{BytesRejection, JsonRejection},
    },
    http::{HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::task;
use tracing_subscriber::EnvFilter;

use crate::{
    auth::{AuthConfig, require_basic_auth},
    config::Config,
    error::AppError,
    fs_store::{Entry, FileStore, TreeEntry},
};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_CSS: &str = include_str!("../web/app.css");
const APP_JS: &str = include_str!("../web/app.js");
const FUSE_JS: &str = include_str!("../web/assets/fuse.basic.min.mjs");
const APP_ICON_PNG: &[u8] = include_bytes!("../web/assets/icon64.png");
const FOLDER_ICON: &str = include_str!("../web/assets/folder.svg");
const FOLDER_ICON_DARK: &str = include_str!("../web/assets/folder-dark.svg");
const TEXT_ICON: &str = include_str!("../web/assets/text.svg");
const TEXT_ICON_DARK: &str = include_str!("../web/assets/text-dark.svg");
const CHEVRON_RIGHT: &str = include_str!("../web/assets/chevron-right.svg");
const CHEVRON_RIGHT_DARK: &str = include_str!("../web/assets/chevron-right-dark.svg");
const CHEVRON_DOWN: &str = include_str!("../web/assets/chevron-down.svg");
const CHEVRON_DOWN_DARK: &str = include_str!("../web/assets/chevron-down-dark.svg");

#[derive(Clone)]
struct AppState {
    store: Arc<FileStore>,
}

#[derive(Deserialize)]
struct PathQuery {
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct CreateRequest {
    #[serde(default)]
    directory: String,
    name: String,
}

#[derive(Deserialize)]
struct RenameRequest {
    path: String,
    name: String,
}

#[derive(Serialize)]
struct EntriesResponse {
    path: String,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
struct TreeResponse {
    entries: Vec<TreeEntry>,
}

#[derive(Serialize)]
struct PathResponse {
    path: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "tpad=info".into()))
        .init();

    let config = Config::from_env()?;
    let store = FileStore::open(&config.data_dir, config.max_file_bytes)?;
    let app = build_router(store, config.auth);
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    tracing::info!(address = %config.listen_addr, data_dir = %config.data_dir.display(), "tpad is listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_router(store: FileStore, auth: Option<AuthConfig>) -> Router {
    let max_file_bytes = store.max_file_bytes();
    let state = AppState {
        store: Arc::new(store),
    };
    let protected = Router::new()
        .route("/", get(index))
        .route("/app.css", get(stylesheet))
        .route("/app.js", get(javascript))
        .route("/assets/fuse.basic.min.mjs", get(fuse_javascript))
        .route("/assets/icon64.png", get(app_icon))
        .route("/assets/folder.svg", get(folder_icon))
        .route("/assets/folder-dark.svg", get(folder_icon_dark))
        .route("/assets/text.svg", get(text_icon))
        .route("/assets/text-dark.svg", get(text_icon_dark))
        .route("/assets/chevron-right.svg", get(chevron_right))
        .route("/assets/chevron-right-dark.svg", get(chevron_right_dark))
        .route("/assets/chevron-down.svg", get(chevron_down))
        .route("/assets/chevron-down-dark.svg", get(chevron_down_dark))
        .route("/api/entries", get(list_entries))
        .route("/api/tree", get(list_tree))
        .route(
            "/api/file",
            get(read_file)
                .put(write_file)
                .post(create_file)
                .patch(rename_file)
                .delete(delete_file),
        )
        .route(
            "/api/directory",
            post(create_directory)
                .patch(rename_directory)
                .delete(delete_directory),
        )
        .layer(DefaultBodyLimit::max(max_file_bytes))
        .with_state(state);

    let protected = match auth {
        Some(auth) => protected.layer(middleware::from_fn_with_state(auth, require_basic_auth)),
        None => protected,
    };

    Router::new()
        .route("/healthz", get(health))
        .merge(protected)
        .layer(middleware::from_fn(security_headers))
}

async fn health() -> &'static str {
    "ok\n"
}

async fn index() -> Response {
    static_text(INDEX_HTML, "text/html; charset=utf-8")
}

async fn stylesheet() -> Response {
    static_text(APP_CSS, "text/css; charset=utf-8")
}

async fn javascript() -> Response {
    static_text(APP_JS, "text/javascript; charset=utf-8")
}

async fn fuse_javascript() -> Response {
    static_text(FUSE_JS, "text/javascript; charset=utf-8")
}

async fn app_icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        Bytes::from_static(APP_ICON_PNG),
    )
        .into_response()
}

async fn folder_icon() -> Response {
    static_text(FOLDER_ICON, "image/svg+xml; charset=utf-8")
}

async fn folder_icon_dark() -> Response {
    static_text(FOLDER_ICON_DARK, "image/svg+xml; charset=utf-8")
}

async fn text_icon() -> Response {
    static_text(TEXT_ICON, "image/svg+xml; charset=utf-8")
}

async fn text_icon_dark() -> Response {
    static_text(TEXT_ICON_DARK, "image/svg+xml; charset=utf-8")
}

async fn chevron_right() -> Response {
    static_text(CHEVRON_RIGHT, "image/svg+xml; charset=utf-8")
}

async fn chevron_right_dark() -> Response {
    static_text(CHEVRON_RIGHT_DARK, "image/svg+xml; charset=utf-8")
}

async fn chevron_down() -> Response {
    static_text(CHEVRON_DOWN, "image/svg+xml; charset=utf-8")
}

async fn chevron_down_dark() -> Response {
    static_text(CHEVRON_DOWN_DARK, "image/svg+xml; charset=utf-8")
}

fn static_text(content: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        content,
    )
        .into_response()
}

async fn list_entries(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<Json<EntriesResponse>, AppError> {
    let path = query.path;
    let store = Arc::clone(&state.store);
    let request_path = path.clone();
    let entries = blocking(move || store.list(&request_path)).await?;
    Ok(Json(EntriesResponse { path, entries }))
}

async fn list_tree(State(state): State<AppState>) -> Result<Json<TreeResponse>, AppError> {
    let store = Arc::clone(&state.store);
    let entries = blocking(move || store.list_tree()).await?;
    Ok(Json(TreeResponse { entries }))
}

async fn read_file(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<Response, AppError> {
    let store = Arc::clone(&state.store);
    let contents = blocking(move || store.read_file(&query.path)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        contents,
    )
        .into_response())
}

async fn write_file(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, AppError> {
    let contents = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::TooLarge
        } else {
            AppError::BadRequest("The request body could not be read")
        }
    })?;
    let store = Arc::clone(&state.store);
    blocking(move || store.write_file(&query.path, &contents)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_file(
    State(state): State<AppState>,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PathResponse>), AppError> {
    let request = json_body(request)?;
    let store = Arc::clone(&state.store);
    let path = blocking(move || store.create_file(&request.directory, &request.name)).await?;
    Ok((StatusCode::CREATED, Json(PathResponse { path })))
}

async fn create_directory(
    State(state): State<AppState>,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PathResponse>), AppError> {
    let request = json_body(request)?;
    let store = Arc::clone(&state.store);
    let path = blocking(move || store.create_directory(&request.directory, &request.name)).await?;
    Ok((StatusCode::CREATED, Json(PathResponse { path })))
}

async fn rename_directory(
    State(state): State<AppState>,
    request: Result<Json<RenameRequest>, JsonRejection>,
) -> Result<Json<PathResponse>, AppError> {
    let request = json_body(request)?;
    let store = Arc::clone(&state.store);
    let path = blocking(move || store.rename_directory(&request.path, &request.name)).await?;
    Ok(Json(PathResponse { path }))
}

async fn rename_file(
    State(state): State<AppState>,
    request: Result<Json<RenameRequest>, JsonRejection>,
) -> Result<Json<PathResponse>, AppError> {
    let request = json_body(request)?;
    let store = Arc::clone(&state.store);
    let path = blocking(move || store.rename_file(&request.path, &request.name)).await?;
    Ok(Json(PathResponse { path }))
}

async fn delete_file(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, AppError> {
    let store = Arc::clone(&state.store);
    blocking(move || store.delete_file(&query.path)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_directory(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, AppError> {
    let store = Arc::clone(&state.store);
    blocking(move || store.delete_directory(&query.path)).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn json_body<T>(request: Result<Json<T>, JsonRejection>) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    request.map(|Json(value)| value).map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::TooLarge
        } else {
            AppError::BadRequest("The JSON request body is invalid")
        }
    })
}

async fn blocking<T, F>(operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    task::spawn_blocking(operation)
        .await
        .map_err(|_| AppError::Io(io::Error::other("filesystem worker stopped")))?
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;

    fn test_app(temp: &TempDir, auth: Option<AuthConfig>) -> Router {
        build_router(FileStore::open(temp.path(), 64).unwrap(), auth)
    }

    #[tokio::test]
    async fn serves_light_and_dark_entry_icons() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, None);

        for (path, expected) in [
            ("/assets/folder.svg", FOLDER_ICON),
            ("/assets/folder-dark.svg", FOLDER_ICON_DARK),
            ("/assets/text.svg", TEXT_ICON),
            ("/assets/text-dark.svg", TEXT_ICON_DARK),
            ("/assets/chevron-right.svg", CHEVRON_RIGHT),
            ("/assets/chevron-right-dark.svg", CHEVRON_RIGHT_DARK),
            ("/assets/chevron-down.svg", CHEVRON_DOWN),
            ("/assets/chevron-down-dark.svg", CHEVRON_DOWN_DARK),
        ] {
            let request = Request::get(path).body(Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "image/svg+xml; charset=utf-8"
            );
            assert_eq!(
                to_bytes(response.into_body(), 2048).await.unwrap(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn serves_the_pinned_fuse_browser_build() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, None);
        let request = Request::get("/assets/fuse.basic.min.mjs")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            to_bytes(response.into_body(), 30_000).await.unwrap(),
            FUSE_JS
        );
    }

    #[tokio::test]
    async fn serves_the_app_icon() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, None);
        let request = Request::get("/assets/icon64.png")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            to_bytes(response.into_body(), APP_ICON_PNG.len() + 1)
                .await
                .unwrap()
                .as_ref(),
            APP_ICON_PNG
        );
    }

    #[tokio::test]
    async fn tree_endpoint_returns_recursive_relative_entries() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("projects/app")).unwrap();
        std::fs::write(temp.path().join("projects/app/ideas.txt"), "ideas").unwrap();
        let app = test_app(&temp, None);

        let request = Request::get("/api/tree").body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 2048).await.unwrap(),
            r#"{"entries":[{"path":"projects","kind":"directory"},{"path":"projects/app","kind":"directory"},{"path":"projects/app/ideas.txt","kind":"file"}]}"#
        );
    }

    #[tokio::test]
    async fn lifecycle_and_traversal_statuses() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, None);

        let create = Request::post("/api/file")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"directory":"","name":"ideas.txt"}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(create).await.unwrap().status(),
            StatusCode::CREATED
        );

        let save = Request::put("/api/file?path=ideas.txt")
            .body(Body::from("hello ž"))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(save).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );

        let read = Request::get("/api/file?path=ideas.txt")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(read).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .unwrap(),
            "nosniff"
        );
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "hello ž"
        );

        let traversal = Request::get("/api/file?path=%2e%2e%2Foutside")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(traversal).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn renames_directories_through_the_directory_endpoint() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, None);
        let create = Request::post("/api/directory")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"directory":"","name":"projects"}"#))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(create).await.unwrap().status(),
            StatusCode::CREATED
        );

        let rename = Request::patch("/api/directory")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"path":"projects","name":"work"}"#))
            .unwrap();
        let response = app.clone().oneshot(rename).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            r#"{"path":"work"}"#
        );

        let listing = Request::get("/api/entries?path=work")
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(listing).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn deletes_nonempty_directories_through_the_api() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("projects/nested")).unwrap();
        std::fs::write(temp.path().join("projects/nested/notes.txt"), "plain text").unwrap();
        let app = test_app(&temp, None);

        let delete = Request::delete("/api/directory?path=projects")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(delete).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert!(!temp.path().join("projects").exists());
    }

    #[tokio::test]
    async fn authentication_exempts_health() {
        let temp = TempDir::new().unwrap();
        let app = test_app(&temp, Some(AuthConfig::new("u", "p")));
        let health = Request::get("/healthz").body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(health).await.unwrap().status(),
            StatusCode::OK
        );

        let denied = Request::get("/").body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(denied).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );

        let denied_tree = Request::get("/api/tree").body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(denied_tree).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );

        let accepted = Request::get("/")
            .header(
                header::AUTHORIZATION,
                format!("Basic {}", STANDARD.encode("u:p")),
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(accepted).await.unwrap().status(),
            StatusCode::OK
        );
    }
}
