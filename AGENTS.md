# AGENTS.md — DB Studio

Read this before you change code. User docs are in [README.md](README.md).

## Goal and priorities

A fast, small SQL Server client for macOS. In order: **safety on production**, **speed**, then features.
Keep the code compact and dependency-light. Match the style of the file you edit.

## Hard rules

1. **Test only on localhost.** Never connect to QA, prod or any Azure server for a test, not even read-only.
   Create throwaway databases on the local SQL Server and drop them after the test.
2. Keep the safety checks in the Rust core (`lib.rs`, `sqlguard.rs`), not only in the UI.
   The UI may ask first; the backend must refuse.
3. Do not run timing tests next to builds.
4. Never commit secrets. Passwords live in the Keychain; connections live in `~/.dbstudio/`, outside the repo.

## Layout

```
app/
  scripts/build-app.sh     release build + signing (pnpm app)
  src-tauri/               Rust core (Tauri v2)
    src/main.rs            entry, calls lib::run
    src/lib.rs             Tauri commands, prod confirmation, run_todo, get_definition, metadata SQL, macOS menu
    src/db.rs              TDS sessions (tiberius), connect (SQL login / az token), run, cancel, row limit, live tests
    src/sqlguard.rs        T-SQL lexer: read-only guard, GO split, @@ROWCOUNT rule (unit tests)
    src/store.rs           connections.json, Keychain, to-do files (unit tests)
    src/devbridge.rs       debug-only HTTP bridge for browser tests
    capabilities/default.json   frontend permissions
    tauri.conf.json        window, bundle id com.vaibhavdandekar.dbstudio (do not change: Keychain + saved UI state are tied to it)
  src/                     SvelteKit SPA (adapter-static, ssr = false), Svelte 5 runes
    app.html               sets the theme before first paint
    routes/+page.svelte    layout: sidebar, tabs, status bar, shortcuts, theme, zoom
    lib/api.ts             typed wrapper over every Tauri command: the frontend/backend contract
    lib/store.svelte.ts    app state, tabs, guarded() prod-confirm retry, caches, IntelliSense schema cache
    lib/QueryTab.svelte    editor (CodeMirror 6, MSSQL dialect), toolbar, results
    lib/Grid.svelte        virtualized results grid (selection, copy, sort, column resize)
    lib/Servers.svelte     object explorer tree, context menus, drop database
    lib/Todos.svelte       to-do panel
    lib/ProdConfirm.svelte typed-phrase dialog (prod writes, drop database)
    lib/ConnDialog.svelte, ContextMenu.svelte, Icon.svelte, app.css (theme variables)
```

## How it works

- **Sessions:** one TDS connection per session key: a tab id, `explorer:<conn>:<db>`, `todo:<id>`, `drop:<conn>`.
  The client is taken out of the map while a query runs. **Cancel** aborts the task and drops the socket.
- **Spare connections:** after a session opens a new connection, `refill` opens one spare per (connection, database)
  in the background. The next new session on that key takes it and skips TCP + TLS + login. TCP keepalive (30 s)
  keeps idle sockets alive through VPN/NAT. `close("explorer:<conn>:<db>")` also drops that database's spares.
- **Timing:** `Trace` records (step, ms) for token, TCP, login, server, rows. It sends live `dbs-phase` events
  (tab toolbar, tree "Loading" rows) and returns `steps` in `QueryResult` (tab status bar). The az token is warmed on start.
- **Azure:** `az account get-access-token --resource https://database.windows.net/`, cached until 5 min before
  expiry. `az_bin()` also searches `/opt/homebrew/bin` and `/usr/local/bin`, because apps opened from Finder
  get a minimal PATH. Azure routing redirects are followed.
- **Read-only:** `sqlguard::write_reason` blocks write keywords outside comments, strings and `[ ]` names.
  The batch also runs inside `BEGIN TRAN` ... `ROLLBACK`.
- **Prod:** `run_query` returns `CONFIRM_WRITE: ...` for a prod write unless `confirm` starts with `execute prod`.
  `run_todo` needs exactly `execute prod <card>` when the to-do has a card. The UI's `guarded()` catches it,
  shows ProdConfirm, and retries.
- **Row limit:** `SET ROWCOUNT max+1` on the server for read batches only. Default 100, hard cap 200k, 0 = no limit.
- **To-dos:** files in `~/.dbstudio/todos/`. `run_todo` re-reads the file, resolves the target by exact
  connection name, runs, and writes `status` + `note` back.
- **Passwords:** Keychain service `dbstudio`, account = connection id. `password_for` falls back to a
  connection with the same server, port and user.
- **UI state** (WebView localStorage): `dbs.tabs`, `dbs.theme`, `dbs.zoom`, `dbs.sidebarW`, `dbs.edH`, `dbs.todoDone`.

## Commands

Run from `app/`:

| Task | Command |
|------|---------|
| Install | `pnpm install` |
| Dev app (hot reload) | `pnpm tauri dev` |
| Type check (frontend) | `pnpm check` |
| Rust unit tests | `cd src-tauri && cargo test` |
| Live tests (local SQL Server only) | `cd src-tauri && DBS_SA_PW=<sa password> cargo test live -- --ignored` |
| Release app | `pnpm app`, which writes `src-tauri/target/release/bundle/macos/DB Studio.app` |

The live tests make their own temp tables in `tempdb`, so they need no test database.

## Browser testing (dev bridge)

```sh
DBSTUDIO_DEV_BRIDGE=1 DBSTUDIO_DEV_PASSWORD=<local sa password> pnpm tauri dev
```

- The bridge serves the Tauri commands on `127.0.0.1:1421`, so the UI at `http://localhost:1420` works in a
  headless browser (for example Playwright with system Chrome).
- It is compiled into **debug builds only**. It accepts only the page origin `http://localhost:1420` and only
  connections whose server is a loopback address.
- `DBSTUDIO_DEV_PASSWORD` (debug only) skips the Keychain. Each debug rebuild has a new signature, and the
  Keychain prompt would block the bridge.

## Gotchas

- `[profile.dev.package."*"] opt-level = 3` in Cargo.toml: a debug `tiberius` is about 10x slower. Keep it.
- `pnpm app` signs with the first "Apple Development" identity, or with `$APPLE_SIGNING_IDENTITY`.
  An ad-hoc build makes macOS ask for Keychain access after every rebuild.
- Stop `pnpm tauri dev` (ports 1420/1421) before you start it again; a stale vite blocks the next start.
- Close the running app before you replace the release bundle: `pkill -x app`.
- `connections.json` is read on start and on the tree's refresh button. Edit it with a write-then-rename.
- A new frontend permission needs an entry in `capabilities/default.json`; without one, the call fails silently in the UI.
