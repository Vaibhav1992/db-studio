// Typed wrappers over the Rust commands in src-tauri/src/lib.rs. Keep the two in sync.
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';

// In `pnpm dev` inside a normal browser (no Tauri), call the debug-only HTTP bridge
// (DBSTUDIO_DEV_BRIDGE=1, localhost connections only) so the UI can be tested end to end.
const bridged = import.meta.env.DEV && typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!bridged) return tauriInvoke<T>(cmd, args);
  const r = await fetch(`http://127.0.0.1:1421/${cmd}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(args ?? {}) });
  const text = await r.text();
  if (!r.ok) throw text;
  return (text ? JSON.parse(text) : null) as T;
}

export type Env = 'local' | 'dev' | 'qa' | 'prod';
export type Auth = 'sql' | 'azure_cli';

export interface Conn {
  id: string; // empty string on create; backend assigns
  name: string;
  server: string; // host only, e.g. "myserver.database.windows.net" or "localhost"
  port: number; // 1433
  database: string | null; // default database, null = server default
  auth: Auth; // 'azure_cli' = token from `az account get-access-token` (Entra ID / MFA via az login)
  user: string | null; // SQL auth only
  tenant: string | null; // azure_cli only, optional tenant id
  env: Env; // drives colour + prod safety
  read_only: boolean; // hard read-only: backend blocks any write
  trust_cert: boolean;
}

export interface DbObject {
  schema: string;
  name: string;
  kind: 'table' | 'view' | 'procedure' | 'function';
}

export interface Column {
  name: string;
  type: string; // e.g. "nvarchar(50)"
  nullable: boolean;
}

export interface ResultSet {
  columns: string[];
  rows: (string | null)[][]; // every value pre-formatted as text; null = SQL NULL
  truncated: boolean; // true when row cap hit
}

export interface QueryResult {
  results: ResultSet[];
  messages: string[]; // e.g. "(3 rows affected)", "Batch 2: ..."
  elapsed_ms: number;
  steps: Steps;
}

/** Where the time went: [step, ms], e.g. ["Azure token", 1800], ["TCP", 40], ["Login (TLS + auth)", 600], ["Server", 90], ["Rows", 20]. */
export type Steps = [string, number][];
/** Live progress of one session (`dbs-phase` event). now = current phase, null = finished. */
export interface Phase { session: string; now: string | null; steps: Steps }

export type TodoStatus = 'todo' | 'done' | 'failed';

export interface Todo {
  id: string; // file stem in ~/.dbstudio/todos
  title: string;
  target: string | null; // exact connection name
  database: string | null;
  status: TodoStatus;
  created: string | null;
  by: string | null; // who added it (agent name / user)
  note: string | null; // last run result or comment
  card: string | null; // work item / ticket id set by the agent; prod runs require typing `execute prod <card>`
  sql: string;
}

export const api = {
  listConnections: () => invoke<Conn[]>('list_connections'),
  /** password only for auth='sql'; stored in macOS Keychain, never in JSON. */
  saveConnection: (conn: Conn, password: string | null) => invoke<Conn>('save_connection', { conn, password }),
  deleteConnection: (id: string) => invoke<void>('delete_connection', { id }),
  /** Returns server version string. Does not save. */
  testConnection: (conn: Conn, password: string | null) => invoke<string>('test_connection', { conn, password }),

  listDatabases: (connId: string) => invoke<string[]>('list_databases', { connId }),
  listObjects: (connId: string, database: string) => invoke<DbObject[]>('list_objects', { connId, database }),
  listColumns: (connId: string, database: string, schema: string, table: string) =>
    invoke<Column[]>('list_columns', { connId, database, schema, table }),

  /** Tables/views/TVFs with columns for autocomplete: { schema: { table: [columns] } }. One query; cache it per conn+db. */
  listSchema: (connId: string, database: string) =>
    invoke<Record<string, Record<string, string[]>>>('list_schema', { connId, database }),

  /**
   * session: one TDS connection per session (use the tab id; 'explorer' for the tree).
   * readOnly: extra per-tab read-only switch (connection read_only always applies).
   * maxRows: row limit per result set, default 100; 0 = no limit (backend hard cap 200k).
   *   Applied server-side for read batches, so SELECT * on a huge table stays fast.
   * confirm: typed phrase. A write on an env='prod' connection needs a phrase starting with
   *   "execute prod"; otherwise the backend rejects with an error starting with "CONFIRM_WRITE:"
   *   (rest = reason) so the UI can ask the user to type it and retry.
   * Splits on GO lines.
   */
  runQuery: (args: { connId: string; database: string | null; session: string; sql: string; readOnly: boolean; maxRows: number; confirm: string | null }) =>
    invoke<QueryResult>('run_query', args),
  /** Procedure/view/function/trigger source as `CREATE OR ALTER ...`. */
  getDefinition: (connId: string, database: string, schema: string, name: string) =>
    invoke<string>('get_definition', { connId, database, schema, name }),
  /** Live connect/query phases for every session. No-op in the browser bridge. */
  onPhase: (cb: (p: Phase) => void) => { if (!bridged) listen<Phase>('dbs-phase', (e) => cb(e.payload)); },
  /** Drops the session's connection, which aborts a running query. */
  cancelQuery: (session: string) => invoke<void>('cancel_query', { session }),
  closeSession: (session: string) => invoke<void>('close_session', { session }),

  listTodos: () => invoke<Todo[]>('list_todos'),
  addTodo: (todo: { title: string; target: string | null; database: string | null; sql: string; by: string | null; card: string | null }) =>
    invoke<Todo>('add_todo', { todo }),
  /**
   * Run a to-do in the backend: it re-reads the file from disk, resolves `target` by EXACT connection
   * name (must match exactly one connection; env names are not accepted), runs with session `todo:<id>`
   * and a 100-row limit, and writes status done/failed + note itself. Refresh the list afterwards.
   * Prod write: rejects with "CONFIRM_WRITE:<reason> Type \"execute prod <card>\" to run." (or
   * "execute prod" without a card); retry with confirm = the typed text.
   */
  runTodo: (id: string, confirm: string | null) => invoke<QueryResult>('run_todo', { id, confirm }),
  setTodoStatus: (id: string, status: TodoStatus, note: string | null) => invoke<void>('set_todo_status', { id, status, note }),
  deleteTodo: (id: string) => invoke<void>('delete_todo', { id }),
};

/** Clipboard that works in the macOS webview (navigator.clipboard.readText is blocked there) and in a dev browser. */
export const clipboard = {
  read: (): Promise<string> => (bridged ? navigator.clipboard.readText() : readText()),
  write: (text: string): Promise<void> => (bridged ? navigator.clipboard.writeText(text) : writeText(text)),
};
