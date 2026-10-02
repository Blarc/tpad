# tpad

`tpad` is a minimal, self-hosted plain-text editor with a filesystem sidebar. Notes are ordinary UTF-8 files in the configured data directory; one Rust binary serves the app. Filesystem contents are the source of truth.

All extensions are treated as plain text. There is no Markdown rendering, rich text, database, full-text search, or external service dependency.

## Quick start with Docker Compose

```sh
mkdir -p tpad
docker compose up --build -d
```

Open <http://localhost:8080>. The host directory `./tpad` is mounted at `/data`, so files created in the app are directly available on the host. Compose runs the container as UID/GID `1000:1000` by default. Match the current host user when needed:

```sh
TPAD_UID="$(id -u)" TPAD_GID="$(id -g)" docker compose up --build -d
```

## Run with Docker directly

```sh
mkdir -p tpad
docker build -t tpad .
docker run --rm \
  -p 8080:8080 \
  -v "$PWD/tpad:/data" \
  --user "$(id -u):$(id -g)" \
  tpad
```

The image includes a health check at `GET /healthz`.

## Configuration

| Variable | Default | Purpose |
|---|---:|---|
| `TPAD_DATA_DIR` | `/data` | Notes directory; it must exist and be accessible to the process. |
| `TPAD_LISTEN_ADDR` | `0.0.0.0:8080` | Address and port to listen on. |
| `TPAD_MAX_FILE_BYTES` | `2097152` | Maximum size of a file read or saved by the app (2 MiB). |
| `TPAD_AUTH_USERNAME` | unset | Optional HTTP Basic username. |
| `TPAD_AUTH_PASSWORD` | unset | Optional HTTP Basic password. |
| `TPAD_PORT` | `8080` | Compose-only host port mapped to container port 8080. |
| `TPAD_UID` / `TPAD_GID` | `1000` / `1000` | Compose-only container user and group IDs. |

Built-in authentication is disabled if both credentials are unset or empty. Set both to non-empty values to enable it; partial configuration is rejected. `GET /healthz` remains available without authentication.

Use HTTPS when exposing HTTP Basic authentication remotely. Alternatively, protect the app behind a trusted reverse proxy or an authentication gateway such as Authelia, Authentik, or Cloudflare Access. The frontend uses relative asset and API URLs, so it can be mounted under a URL prefix if the proxy strips that prefix before forwarding. No WebSocket or special proxy timeout is required.

## Using the app

- Single-click a sidebar item to select it; double-click a file to open it or a folder to navigate into it. Right-click an item for file or folder actions.
- Use **Find** to fuzzy-match file and folder paths across the whole tree. Search is limited to paths; note contents are never searched. Results are matched in memory using the locally bundled Fuse.js 7.5.0 library.
- The editor is a plain text area. It supports arbitrary valid UTF-8 text, preserves the file’s newline style when saving, and autosaves shortly after edits. `Ctrl/Cmd+S` saves immediately.
- Binary files, symlinks, and special filesystem entries cannot be edited. Symlinks and special entries are excluded from the Find tree.

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd+S` | Save the open file. |
| `Ctrl/Cmd+N` | Create a file in the current folder. |
| `Ctrl/Cmd+O` | Focus the file browser, or open the sidebar on small screens. |
| `Escape` | Close a dialog or action menu; on small screens, also close the sidebar. |
| Find: `↑` / `↓`, `Enter`, `Escape` | Move through results, open/navigate to the selected result, or close Find. |
| File browser: `↑` / `↓` | Move through visible items. |
| File browser: `Enter` / `Space` | Open a file or expand/collapse a folder. |
| File browser: `←` / `→` | Collapse or expand folders; move to the parent or into visible children. |
| `Shift+F10` or `Context Menu` | Open the selected item’s action menu. Within the menu, `↑` / `↓` moves between actions. |
| Focused sidebar divider: `←` / `→` | Resize the sidebar by 10 pixels. |

Normal browser text-editing shortcuts work in the editor as usual.

## Files and safety

- Any filename extension is allowed; file contents must be valid UTF-8 text without NUL bytes. Markdown files are still treated as plain text.
- Saves use a temporary file and atomic replacement in the same directory. New files and renames do not overwrite existing entries.
- Deleting a folder permanently removes its contents. Symlinks inside it are removed without following their targets.
- Multiple app instances or editors may access the same files; concurrent edits use last-completed-save-wins behavior.
- Back up the data directory with ordinary filesystem tools. There is no application-specific export format.

## API

The browser uses a same-origin API. All endpoints except `/healthz` use the configured built-in authentication when enabled.

| Method | Endpoint | Purpose |
|---|---|---|
| `GET` | `/api/entries?path=...` | List one directory. |
| `GET` | `/api/tree` | Recursively list relative file and folder paths for Find. |
| `GET` / `PUT` | `/api/file?path=...` | Read or atomically save a file. |
| `POST` | `/api/file` | Create an empty file. |
| `PATCH` / `DELETE` | `/api/file` or `/api/file?path=...` | Rename or delete a file. |
| `POST` / `PATCH` / `DELETE` | `/api/directory` or `/api/directory?path=...` | Create, rename, or recursively delete a folder. |

Mutation errors are returned as JSON. Attempts to create or rename over an existing entry return `409 Conflict`.

The Fuse.js Apache-2.0 license is included at `web/assets/FUSE-LICENSE.txt`.

## Local development

Rust 1.98 is pinned in `rust-toolchain.toml`.

```sh
mkdir -p tpad
TPAD_DATA_DIR="$PWD/tpad" TPAD_LISTEN_ADDR=127.0.0.1:8080 cargo run
```

Run the project checks with:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release --locked
```
