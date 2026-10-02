# tpad

`tpad` is a small, self-hosted plain-text editor with a filesystem sidebar. It stores tpad as ordinary UTF-8 files and serves a single vanilla HTML/CSS/JavaScript interface from one Rust binary.

It deliberately has no Markdown rendering, rich text, database, tags, full-text search index, collaboration, telemetry, or cloud dependency.

## Run with Docker Compose

Create the host directory before starting the container so it has the ownership you expect:

```sh
mkdir -p tpad
docker compose up --build -d
```

Open <http://localhost:8080>. Files created in the browser appear directly in `./tpad`.

The container runs as UID/GID `1000:1000` by default. Override these when the host directory belongs to another account:

```sh
TPAD_UID="$(id -u)" TPAD_GID="$(id -g)" docker compose up --build -d
```

## Run with Docker

```sh
docker build -t tpad .
docker run --rm \
  -p 8080:8080 \
  -v "$PWD/tpad:/data" \
  --user "$(id -u):$(id -g)" \
  tpad
```

The image exposes `GET /healthz` and includes a container health check.

## Configuration

| Variable | Default | Purpose |
|---|---:|---|
| `TPAD_DATA_DIR` | `/data` | tpad directory |
| `TPAD_LISTEN_ADDR` | `0.0.0.0:8080` | Listen address and port |
| `TPAD_MAX_FILE_BYTES` | `2097152` | Maximum readable or writable file size |
| `TPAD_AUTH_USERNAME` | unset | Optional HTTP Basic username |
| `TPAD_AUTH_PASSWORD` | unset | Optional HTTP Basic password |

Authentication is disabled when both authentication variables are absent. The server refuses to start when only one is set or either is empty. The Compose file intentionally expands unset credentials to empty strings; the application treats that pair as authentication disabled.

HTTP Basic credentials are only protected in transit when TLS is used. Put `tpad` behind an HTTPS reverse proxy for remote access. Instead of built-in authentication, the whole application can be protected by Authelia, Authentik, Cloudflare Access, or reverse-proxy basic auth.

The frontend uses relative asset and API URLs, so a reverse proxy may mount it below a path when that prefix is stripped before proxying. No WebSocket support or special proxy timeout is required.

## Filesystem behavior

- The configured directory is the source of truth; stopping `tpad` leaves normal files behind.
- Any extension is accepted, but content must be valid UTF-8 text without NUL bytes.
- The Find picker fuzzy-matches relative file and folder paths only; it never searches text contents. Its path list is fetched when opened and matched in memory.
- Symlinks and special files are listed as unsupported and cannot be edited.
- Saves use a flushed temporary file and an atomic replacement in the same directory.
- New files and renames never overwrite an existing name.
- Deleting a folder permanently removes all of its contents. Symbolic links inside it are removed without following their targets.
- Multiple editors use last-completed-save-wins behavior.

Back up `/data` with ordinary filesystem tools. There is no application-specific export format.

## Local development

Rust 1.98 is pinned in `rust-toolchain.toml`.

```sh
mkdir -p tpad
TPAD_DATA_DIR="$PWD/tpad" TPAD_LISTEN_ADDR=127.0.0.1:8080 cargo run
```

Checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release --locked
```

## API

The browser uses a small same-origin API:

- `GET /api/entries?path=` lists one directory.
- `GET /api/tree` lists searchable relative file and folder paths recursively.
- `GET` and `PUT /api/file?path=` read and atomically save a file.
- `POST /api/file` creates an empty file.
- `PATCH /api/file` renames a file within its current directory.
- `DELETE /api/file?path=` deletes a file.
- `POST /api/directory` creates a folder.
- `DELETE /api/directory?path=` recursively deletes a folder and its contents.

Mutation requests return JSON errors and use `409 Conflict` rather than replacing existing entries.

The browser bundles Fuse.js 7.5.0 locally for path matching. Its Apache-2.0 license is included at `web/assets/FUSE-LICENSE.txt`.
