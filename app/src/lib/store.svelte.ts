// Shared app state. Results are NOT kept here (they live in each QueryTab, never persisted).
import { sql, MSSQL } from '@codemirror/lang-sql';
import type { Extension } from '@codemirror/state';
import { api, clipboard, type Conn, type Env, type QueryResult } from './api';

/** Copy text and say so in the status bar. */
export const copyText = (text: string, what: string) =>
  clipboard.write(text).then(() => (app.msg = `Copied ${what}`), (e) => (app.msg = String(e)));

export const qid = (s: string) => `[${s.replaceAll(']', ']]')}]`; // bracket-quote an identifier
/** Pointer-captured drag on e.currentTarget; onMove(dx, dy) runs at most once per animation frame, plus a final flush. */
export function drag(e: PointerEvent, onMove: (dx: number, dy: number) => void, onEnd?: () => void) {
  const el = e.currentTarget as HTMLElement, x0 = e.clientX, y0 = e.clientY;
  let f = 0, dx = 0, dy = 0;
  e.preventDefault();
  el.setPointerCapture(e.pointerId);
  const mv = (m: PointerEvent) => { dx = m.clientX - x0; dy = m.clientY - y0; f ||= requestAnimationFrame(() => { f = 0; onMove(dx, dy); }); };
  const up = () => {
    el.removeEventListener('pointermove', mv);
    el.removeEventListener('pointerup', up);
    el.removeEventListener('pointercancel', up);
    cancelAnimationFrame(f);
    onMove(dx, dy);
    onEnd?.();
  };
  el.addEventListener('pointermove', mv);
  el.addEventListener('pointerup', up);
  el.addEventListener('pointercancel', up);
}
/** Global pane sizes, persisted in localStorage. */
export const ui = $state({ edH: +(localStorage.getItem('dbs.edH') ?? '') || 300 });

/** Exact (case-insensitive, trimmed) connection-name match; must be unique. */
export const resolveTarget = (target: string | null): { conn?: Conn; err?: 'unknown target' | 'ambiguous target' } => {
  const x = (target ?? '').trim().toLowerCase();
  const m = x ? app.conns.filter((c) => c.name.trim().toLowerCase() === x) : [];
  return m.length === 1 ? { conn: m[0] } : { err: m.length ? 'ambiguous target' : 'unknown target' };
};

export const ENVS: Env[] = ['local', 'dev', 'qa', 'prod'];
export const envColor = (e?: Env) => (e ? `var(--${e})` : 'var(--local)');

export interface Tab {
  id: string;
  n: number;
  connId: string;
  database: string; // '' = connection default
  sql: string;
  readOnly: boolean;
  maxRows: number; // 0 = no limit
  autorun?: boolean; // run once on mount, not persisted
}

export const app = $state({
  conns: [] as Conn[],
  tabs: [] as Tab[],
  active: '',
  side: 'servers' as 'servers' | 'todo',
  msg: '',
  ready: false,
});

export const connOf = (id: string) => app.conns.find((c) => c.id === id);
export const activeTab = () => app.tabs.find((t) => t.id === app.active);

export async function loadConns() {
  try {
    app.conns = await api.listConnections();
  } catch (e) {
    app.msg = String(e);
  }
}

let counter = 0;
export function newTab(connId = '', database = '', sql = '', autorun = false) {
  // A blank new tab never opens on prod: keep the active tab's connection if it is not prod, else the safest env.
  const cur = connOf(activeTab()?.connId ?? '');
  const safe = ['local', 'dev', 'qa'].map((e) => app.conns.find((c) => c.env === e)).find(Boolean);
  const id = connId || (cur && cur.env !== 'prod' ? cur.id : safe?.id) || '';
  const t: Tab = { id: crypto.randomUUID(), n: ++counter, connId: id, database, sql, readOnly: connOf(id)?.env === 'prod', maxRows: 100, autorun };
  app.tabs.push(t);
  app.active = t.id;
  return t;
}

export function closeTab(id: string) {
  const i = app.tabs.findIndex((t) => t.id === id);
  if (i < 0) return;
  app.tabs.splice(i, 1);
  api.closeSession(id).catch(() => {});
  if (app.active === id) app.active = (app.tabs[i] ?? app.tabs[i - 1])?.id ?? '';
  if (!app.tabs.length) newTab(); // never leave zero tabs
}

const KEY = 'dbs.tabs';
export function restoreTabs() {
  try {
    const s = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    const old = s.v !== 2; // v1 saved the old 1,000 default into every tab
    for (const t of s.tabs ?? []) app.tabs.push({ id: t.id, n: t.n, connId: t.connId, database: t.database ?? '', sql: t.sql ?? '', readOnly: !!t.readOnly || connOf(t.connId)?.env === 'prod', maxRows: Number.isFinite(t.maxRows) && !(old && t.maxRows === 1000) ? t.maxRows : 100 }); // prod tabs always restore locked
    counter = Math.max(0, ...app.tabs.map((t) => t.n));
    app.active = app.tabs.some((t) => t.id === s.active) ? s.active : (app.tabs[0]?.id ?? '');
  } catch {}
  if (!app.tabs.length) newTab();
  app.ready = true;
}
export const saveTabs = () => localStorage.setItem(KEY, JSON.stringify({ v: 2, tabs: app.tabs.map(({ autorun, ...t }) => t), active: app.active }));

// Own confirm dialog: window.confirm is unreliable in the macOS webview.
export const ask = $state({ msg: '', resolve: null as null | ((ok: boolean) => void) });
export const confirmDlg = (msg: string) => new Promise<boolean>((resolve) => { ask.msg = msg; ask.resolve = resolve; });

export const CANCELLED = 'Cancelled';

// Typed-phrase modal for prod writes (rendered by ProdConfirm.svelte).
export const wask = $state({ req: null as null | { reason: string; conn: string; db: string; sql: string; card: string | null; phrase?: string; title?: string; action?: string; resolve: (typed: string | null) => void } });

/** Calls `call`; on CONFIRM_WRITE asks for the typed phrase and retries with it. Throws an error string (CANCELLED if the user backs out). */
export async function guarded<T>(info: { conn: string; db: string; sql: string; card: string | null }, call: (confirm: string | null) => Promise<T>): Promise<T> {
  let confirm: string | null = null;
  for (;;) {
    try {
      return await call(confirm);
    } catch (e) {
      const s = String(e);
      if (!s.startsWith('CONFIRM_WRITE:')) throw s;
      const typed = await new Promise<string | null>((resolve) => (wask.req = { ...info, reason: s.slice(14).trim(), resolve }));
      if (typed === null) throw CANCELLED;
      confirm = typed;
    }
  }
}

export const execute = (c: Conn, database: string, session: string, sql: string, readOnly: boolean, maxRows: number): Promise<QueryResult> =>
  guarded({ conn: c.name, db: database || c.database || 'default', sql, card: null }, (confirm) =>
    api.runQuery({ connId: c.id, database: database || null, session, sql, readOnly: readOnly || c.read_only, maxRows, confirm }));

// database list cache per connection
const dbCache = new Map<string, Promise<string[]>>();
export const databases = (connId: string) => {
  let p = dbCache.get(connId);
  if (!p) dbCache.set(connId, (p = api.listDatabases(connId).catch((e) => { dbCache.delete(connId); throw e; })));
  return p;
};
export const clearDatabases = (id?: string) => (id ? dbCache.delete(id) : dbCache.clear());

// IntelliSense schema cache: one entry per connection + database, shared by all tabs. Plain objects, never reactive.
type Schema = Record<string, Record<string, string[]>>;
type Intel = { schema: Schema; ext: Extension };
export const KEYWORDS: Extension = sql({ dialect: MSSQL, upperCaseKeywords: true });
const schemaCache = new Map<string, Promise<Intel>>();
const schemaDone = new Map<string, Schema>(); // resolved copies, for sync lookups (Expand *)
/** Bumped by clearSchema so open editors reload. */
export const intel = $state({ gen: 0 });

/** null = no database known: keywords only. */
export const schemaKey = (connId: string, database: string) => {
  const db = database || connOf(connId)?.database;
  return db && connId ? `${connId}|${db}` : null;
};
export const schemaOf = (key: string | null) => (key ? schemaDone.get(key) : undefined);
export function loadSchema(key: string): Promise<Intel> {
  let p = schemaCache.get(key);
  if (!p) {
    const i = key.indexOf('|');
    p = api.listSchema(key.slice(0, i), key.slice(i + 1)).then((schema) => {
      schemaDone.set(key, schema);
      return { schema, ext: sql({ dialect: MSSQL, schema, defaultSchema: 'dbo', upperCaseKeywords: true }) };
    });
    schemaCache.set(key, p);
    p.catch(() => schemaCache.get(key) === p && schemaCache.delete(key));
  }
  return p;
}
/** No args = everything; no database = every database of the connection. */
export function clearSchema(connId?: string, database?: string) {
  const key = connId && database ? schemaKey(connId, database) : null;
  for (const k of [...schemaCache.keys()]) if (!connId || k === key || (!database && k.startsWith(connId + '|'))) { schemaCache.delete(k); schemaDone.delete(k); }
  intel.gen++;
}
