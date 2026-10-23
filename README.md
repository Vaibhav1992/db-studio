# DB Studio

A small, fast SQL Server client for macOS: object explorer, query editor with IntelliSense, a virtualized
results grid, a read-only mode, a typed confirmation for production writes, and a to-do queue that coding
agents fill and you run after review.

Built with Tauri v2 (Rust core, TDS via `tiberius`), Svelte 5 and CodeMirror 6. About 6 MB.

Working on the code? Read [AGENTS.md](AGENTS.md) first.

## Install and run

Needs: macOS, Rust (stable), Node 22+, pnpm, and the Azure CLI (`az`) for Entra ID connections.

```sh
cd app
pnpm install
pnpm app          # release build: app/src-tauri/target/release/bundle/macos/DB Studio.app
pnpm tauri dev    # development, with hot reload
```

`pnpm app` signs the app with your "Apple Development" certificate if you have one. With a stable
signature, macOS remembers "Always Allow" for Keychain passwords. Without it, every rebuild asks again.

## Connections

- One connection per database you use, grouped by environment: Local, Dev, QA, Prod.
  A connection with a database shows only that database. A connection without one shows the whole server.
- Right-click a database > **Add as connection...** to make a connection for it.
- **SQL login**: server, port, user, password. Local Docker SQL Server needs "Trust server certificate".
- **Microsoft Entra ID (Azure CLI)**: uses your `az login` session (the Microsoft access-token flow).
- Stored in `~/.dbstudio/connections.json`. Passwords are in the macOS Keychain (service `dbstudio`),
  never in the file. Connections with the same server, port and user share a saved password.

## Safety

- **Read-only** (per connection, or per tab with the lock button): only read batches run. A keyword
  guard blocks writes, and the batch also runs in a transaction that is always rolled back.
  For a hard guarantee, use a database login that has only read permissions.
- **Production**: tabs open read-only. A write runs only after you type `execute prod`
  (for a to-do with a card: `execute prod <card>`). The check is in the Rust core, not only in the UI.
- A blank new tab never opens on a prod connection.
- **Drop database**: you type the database name; on prod you also type `execute prod`.
- **Row limit**: 100 rows per result set by default (1k / 10k / 100k / no limit in the toolbar).
  Applied on the server for read batches only, so writes are never limited.

## Shortcuts

| Keys | Action |
|------|--------|
| Cmd+Enter | Run (the selection, or the whole editor) |
| Esc | Cancel the running query |
| Cmd+T / Cmd+W | New tab / close tab |
| Cmd+Shift+R | Reload IntelliSense for the tab's database |
| Cmd+= / Cmd+- / Cmd+0 | Zoom in / out / reset (110%) |
| Double-click a table | `SELECT TOP (100)` in a new tab |

Theme: click "Theme" in the bottom bar (System, Light, Dark).

## To-do queue (for you and for agents)

Flow: write a proc, run it on local. An agent queues it for QA. After deploy, it queues it for prod.
Agents cannot run anything; you review and click **Run**.

Each to-do is one file: `~/.dbstudio/todos/<YYYYMMDD-HHMMSS>-<slug>.sql`. Write `<name>.sql.tmp`,
then rename it, so the app never reads half a file. The app shows new files within 5 seconds.

```sql
-- title: Normalize customer emails
-- target: MainQa
-- card: AB#1234
-- status: todo
-- created: 2026-10-23T21:00:00+01:00
-- by: claude

CREATE OR ALTER PROCEDURE dbo.usp_NormalizeEmails AS ...
GO
EXEC dbo.usp_NormalizeEmails;
```

- `target`: the **exact connection name**. It must match exactly one connection.
- `database`: optional. Leave it out when the connection already points to one database.
- `card`: work item id. A prod run needs `execute prod <card>`.
- `status`: `todo`, `done` or `failed`. Only the app writes `done`/`failed` and the `note` line.
- Unknown header keys are kept. Separate batches with `GO` lines.
