<script lang="ts">
  import { api, clipboard, type Column, type Conn, type DbObject, type Env } from './api';
  import { app, ENVS, envColor, newTab, loadConns, clearDatabases, databases, confirmDlg, qid, clearSchema, wask, execute, CANCELLED, phases, fmtSteps } from './store.svelte';
  import ConnDialog from './ConnDialog.svelte';
  import Icon from './Icon.svelte';
  import ContextMenu, { SEP, type MenuItem } from './ContextMenu.svelte';

  type Entry = { data?: unknown; err?: string };
  type Row = { key: string; depth: number; label: string; kind: string; open?: boolean; folder?: boolean; c?: Conn; db?: string; o?: DbObject; env?: Env; sub?: string; count?: number };

  let open = $state<Record<string, boolean>>({});
  let cache = $state.raw<Record<string, Entry>>({}); // key -> loaded data | error; missing = loading
  let editing = $state<Conn | null | undefined>(undefined); // undefined = closed, null = new
  let selected = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const KINDS: [DbObject['kind'], string][] = [['table', 'Tables'], ['view', 'Views'], ['procedure', 'Procedures'], ['function', 'Functions']];

  async function load(key: string, p: Promise<unknown>) {
    if (key in cache) return;
    cache = { ...cache, [key]: {} };
    try { cache = { ...cache, [key]: { data: await p } }; } catch (e) { cache = { ...cache, [key]: { err: String(e) } }; }
  }

  // While loading, show the live phase of the backend session (Azure token, TCP, login, query) and the steps done so far.
  function msg(r: Row[], key: string, depth: number, e: Entry, session: string) {
    const p = phases[session], live = p?.now ? [p.now + '...', fmtSteps(p.steps)].filter(Boolean).join(' · ') : 'Loading...';
    r.push({ key: key + '/m', depth, kind: 'msg', label: e.err ?? (e.data ? '(empty)' : live), sub: e.err ? 'err' : 'dim' });
  }
  const loadTime = (session: string) => (phases[session]?.now === null ? 'Last load: ' + fmtSteps(phases[session].steps) : undefined);

  const rows = $derived.by(() => {
    const r: Row[] = [];
    for (const env of ENVS) {
      const cs = app.conns.filter((c) => c.env === env);
      if (!cs.length) continue;
      r.push({ key: 'env:' + env, depth: 0, label: env, kind: 'env', env });
      for (const c of cs) {
        const ck = c.id, ro = open[ck];
        r.push({ key: ck, depth: 1, label: c.name, kind: 'conn', open: ro, folder: true, c, env });
        if (!ro) continue;
        // A connection with a database shows only that database (open by default); without one, the server's "Databases" folder.
        let d = 2, dl: string[] | undefined = c.database ? [c.database] : undefined;
        if (!dl) {
          const dk = ck + '/dbs', dopen = open[dk] ?? true;
          r.push({ key: dk, depth: 2, label: 'Databases', kind: 'dir', open: dopen, folder: true, c });
          if (!dopen) continue;
          const dbs = cache[ck];
          dl = dbs?.data as string[] | undefined;
          if (!dl?.length) { msg(r, dk, 3, dbs ?? {}, `explorer:${c.id}:`); continue; }
          d = 3;
        }
        for (const db of dl) {
          const bk = `${ck}|${db}`, bopen = open[bk] ?? !!c.database;
          r.push({ key: bk, depth: d, label: db, kind: 'db', open: bopen, folder: true, c, db });
          if (!bopen) continue;
          const objs = cache[bk];
          if (!objs?.data) { msg(r, bk, d + 1, objs ?? {}, `explorer:${c.id}:${db}`); continue; }
          for (const [kind, title] of KINDS) {
            const list: DbObject[] = (objs.data as DbObject[]).filter((x) => x.kind === kind);
            const gk = `${bk}|${kind}`;
            r.push({ key: gk, depth: d + 1, label: title, count: list.length, kind: 'dir', open: open[gk], folder: !!list.length, c, db });
            if (!open[gk]) continue;
            for (const o of list) {
              const ok = `${gk}|${o.schema}.${o.name}`;
              r.push({ key: ok, depth: d + 2, label: o.name, sub: o.schema, kind: 'obj', open: open[ok], folder: kind === 'table', c, db, o });
              if (!open[ok] || kind !== 'table') continue;
              const cols = cache[ok];
              if (!cols?.data) { msg(r, ok, d + 3, cols ?? {}, `explorer:${c.id}:${db}`); continue; }
              for (const col of cols.data as Column[]) r.push({ key: `${ok}|${col.name}`, depth: d + 3, label: col.name, kind: 'col', sub: `${col.type}, ${col.nullable ? 'null' : 'not null'}` });
            }
          }
        }
      }
    }
    let env: Env | undefined; // every row carries its group's env for the colour bar
    for (const x of r) x.kind === 'env' ? (env = x.env) : (x.env = env);
    return r;
  });

  function toggle(row: Row) {
    if (!row.folder) return;
    const k = row.key;
    open[k] = !(open[k] ?? ((row.kind === 'dir' && row.label === 'Databases') || (row.kind === 'db' && row.db === row.c?.database)));
    if (!open[k]) return;
    const c = row.c!;
    if (row.kind === 'conn') { open[k + '/dbs'] = true; loadConn(c); }
    else if (row.kind === 'dir' && row.label === 'Databases') load(c.id, databases(c.id));
    else if (row.kind === 'db') load(k, api.listObjects(c.id, row.db!));
    else if (row.kind === 'obj') load(k, api.listColumns(c.id, row.db!, row.o!.schema, row.o!.name));
  }

  // A pinned connection loads its database's objects; a server connection loads its database list.
  const loadConn = (c: Conn) => (c.database ? load(`${c.id}|${c.database}`, api.listObjects(c.id, c.database)) : load(c.id, databases(c.id)));

  const fq = (o: DbObject) => `${qid(o.schema)}.${qid(o.name)}`;
  const top100 = (r: Row) => newTab(r.c!.id, r.db!, `SELECT TOP (100) * FROM ${fq(r.o!)}`, true);
  const fail = (e: unknown) => (app.msg = String(e));

  async function scriptSelect(r: Row) {
    try {
      const cols = await api.listColumns(r.c!.id, r.db!, r.o!.schema, r.o!.name);
      newTab(r.c!.id, r.db!, `SELECT\n${cols.map((c) => '    ' + qid(c.name)).join(',\n')}\nFROM ${fq(r.o!)}`);
    } catch (e) { fail(e); }
  }
  const definition = (r: Row) =>
    api.getDefinition(r.c!.id, r.db!, r.o!.schema, r.o!.name).then((t) => newTab(r.c!.id, r.db!, t), fail);

  // Drop cached children of a node and reload it (if it is open).
  function reload(r: Row) {
    const k = r.key;
    cache = Object.fromEntries(Object.entries(cache).filter(([x]) => x !== k && !x.startsWith(k + '|')));
    open = Object.fromEntries(Object.entries(open).filter(([x]) => !x.startsWith(k + '|')));
    if (r.kind === 'conn') { clearDatabases(r.c!.id); clearSchema(r.c!.id); if (open[k]) loadConn(r.c!); }
    else if (open[k]) load(k, api.listObjects(r.c!.id, r.db!));
  }

  // Re-fetch a database's objects and drop its IntelliSense cache. Keeps folders open; collapses expanded tables (their columns are refetched on open).
  function reloadDb(c: Conn, db: string) {
    const bk = `${c.id}|${db}`;
    cache = Object.fromEntries(Object.entries(cache).filter(([x]) => x !== bk && !x.startsWith(bk + '|')));
    for (const x of Object.keys(open)) if (x.startsWith(bk + '|') && /^(table|view|procedure|function)\|/.test(x.slice(bk.length + 1))) open[x] = false;
    clearSchema(c.id, db);
    if (open[bk]) load(bk, api.listObjects(c.id, db));
  }

  function menuFor(r: Row): MenuItem[] {
    const c = r.c!, o = r.o;
    if (r.kind === 'conn') return [
      { label: 'New query', icon: 'plus', run: () => newTab(c.id, c.database ?? '') },
      { label: 'Refresh', icon: 'refresh', run: () => reload(r) },
      { label: 'Edit', icon: 'edit', run: () => (editing = c) },
      { label: 'Delete', icon: 'trash', danger: true, run: () => del(c) },
    ];
    if (r.kind === 'db') return [
      { label: 'New query', icon: 'plus', run: () => newTab(c.id, r.db!) },
      { label: 'Refresh', icon: 'refresh', run: () => reloadDb(c, r.db!) },
      ...(c.database ? [] : [{ label: 'Add as connection...', icon: 'plus', run: () => (editing = { ...c, id: '', name: r.db!, database: r.db! }) }]),
      SEP,
      { label: 'Drop database...', icon: 'trash', danger: true, disabled: c.read_only || SYSTEM_DBS.includes(r.db!.toLowerCase()), run: () => dropDb(c, r.db!) },
    ];
    if (r.kind === 'dir' && r.db) return [{ label: 'Refresh', icon: 'refresh', run: () => reloadDb(c, r.db!) }];
    if (!o) return [];
    const top = { label: 'Select Top 100', icon: 'run', run: () => top100(r) };
    const def = { label: 'View definition', icon: 'file', run: () => definition(r) };
    if (o.kind === 'table') return [top, { label: 'Script as SELECT', icon: 'file', run: () => scriptSelect(r) }, { label: 'Copy name', icon: 'copy', run: () => clipboard.write(fq(o)).catch(fail) }];
    if (o.kind === 'view') return [top, def];
    if (o.kind === 'procedure') return [def, { label: 'Script as EXEC', icon: 'file', run: () => newTab(c.id, r.db!, `EXEC ${fq(o)}`) }];
    return [def];
  }

  function ctx(e: MouseEvent, r: Row, at?: HTMLElement) {
    const items = menuFor(r);
    if (!items.length) return;
    e.preventDefault(); e.stopPropagation();
    selected = r.key;
    const b = at?.getBoundingClientRect();
    menu = { x: b ? b.left : e.clientX, y: b ? b.bottom : e.clientY, items };
  }

  function refresh() {
    open = {}; cache = {};
    clearDatabases();
    clearSchema();
    loadConns();
  }

  const SYSTEM_DBS = ['master', 'model', 'msdb', 'tempdb'];

  // Typed database name first; a prod connection then also asks for "execute prod" (backend guard).
  async function dropDb(c: Conn, db: string) {
    const name = qid(db), lit = name.replaceAll("'", "''");
    // SINGLE_USER closes other sessions (SSMS "Close existing connections"); Azure SQL Database (edition 5) has no SINGLE_USER.
    const sql = `IF SERVERPROPERTY('EngineEdition') <> 5 EXEC('ALTER DATABASE ${lit} SET SINGLE_USER WITH ROLLBACK IMMEDIATE');\nDROP DATABASE ${name};`;
    const typed = await new Promise<string | null>((resolve) => (wask.req = {
      conn: c.name, db, sql, card: null, phrase: db, title: 'Drop database', action: 'Drop database', resolve,
      reason: `This deletes the database "${db}" and all of its data. You cannot undo this.`,
    }));
    if (typed === null) return;
    const s = `drop:${c.id}`;
    await Promise.all([`explorer:${c.id}:${db}`, ...app.tabs.filter((t) => t.connId === c.id && (t.database || c.database) === db).map((t) => t.id)].map((x) => api.closeSession(x).catch(() => {})));
    try {
      await execute(c, 'master', s, sql, false, 100);
      app.msg = `Database "${db}" dropped.`;
    } catch (e) { if (e !== CANCELLED) fail(e); }
    api.closeSession(s).catch(() => {});
    reload(rows.find((x) => x.key === c.id)!);
  }

  async function del(c: Conn) {
    if (!(await confirmDlg(`Delete connection "${c.name}"?`))) return;
    try { await api.deleteConnection(c.id); await loadConns(); } catch (e) { app.msg = String(e); }
  }
</script>

<div class="bar">
  <button onclick={() => (editing = null)} data-testid="new-conn"><Icon name="plus" size={14} />New connection</button>
  <span class="grow"></span>
  <button class="icon" onclick={refresh} title="Refresh all" data-testid="tree-refresh"><Icon name="refresh" size={14} /></button>
</div>
<div class="tree" role="tree">
  {#each rows as r (r.key)}
    {@const kind = r.o?.kind ?? r.kind}
    <div class="n" class:env={r.kind === 'env'} class:sel={selected === r.key} style="--d:{r.depth};--e:{envColor(r.env)}"
      title={r.kind === 'db' ? loadTime(`explorer:${r.c!.id}:${r.db}`) : r.kind === 'conn' && !r.c?.database ? loadTime(`explorer:${r.c!.id}:`) : undefined}
      data-testid="tree-row" data-kind={kind} data-name={r.o ? `${r.o.schema}.${r.o.name}` : r.kind === 'dir' || r.kind === 'msg' ? r.label : r.kind === 'db' ? r.db : r.label}
      onclick={(e) => { selected = r.key; if (e.detail < 2) toggle(r); }} ondblclick={() => (r.o?.kind === 'table' || r.o?.kind === 'view') && top100(r)}
      oncontextmenu={(e) => ctx(e, r)} role="treeitem" aria-selected={selected === r.key} aria-expanded={r.folder ? !!r.open : undefined} tabindex="-1" onkeydown={() => {}}>
      <span class="gd"></span>
      <span class="ar" class:open={r.open}>{#if r.folder}<Icon name="chevron" size={12} />{/if}</span>
      {#if r.kind === 'env'}<span class="dot"></span>
      {:else if r.kind === 'conn'}<span class="ic env"><Icon name="server" size={15} /></span>
      {:else if r.kind === 'db'}<span class="ic"><Icon name="database" size={15} /></span>
      {:else if r.kind === 'dir'}<span class="ic"><Icon name="folder" size={15} /></span>
      {:else if r.o}<span class="ic"><Icon name={r.o.kind} size={15} /></span>
      {:else if r.kind === 'col'}<span class="ic"><Icon name="column" size={14} /></span>{/if}
      {#if r.kind === 'conn' && r.c?.read_only}<span class="ic" title="Read-only"><Icon name="lock" size={12} /></span>{/if}
      <span class="l" class:dim={r.kind === 'env' || r.sub === 'dim'} class:err={r.sub === 'err'}>{#if r.o}<span class="dim">{r.sub}.</span>{/if}{r.label}</span>
      {#if r.count !== undefined}<span class="dim cnt">{r.count}</span>{/if}
      {#if r.kind === 'col'}<span class="dim mono ty">{r.sub}</span>{/if}
      {#if ['conn', 'db', 'obj'].includes(r.kind)}<button class="icon more" title="Actions" onclick={(e) => ctx(e, r, e.currentTarget)}><Icon name="more" size={14} /></button>{/if}
    </div>
  {/each}
  {#if !app.conns.length}<div class="dim pad">No connections yet. Click "New connection".</div>{/if}
</div>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}
{#if editing !== undefined}
  <ConnDialog conn={editing} onclose={() => (editing = undefined)} />
{/if}

<style>
  .bar { display: flex; align-items: center; gap: 4px; padding: 6px 8px; border-bottom: 1px solid var(--bd); }
  .tree { flex: 1; overflow: auto; padding: 4px 0; }
  .n { display: flex; align-items: center; gap: 4px; height: 24px; white-space: nowrap; padding-right: 4px; }
  .n:hover { background: var(--hover); }
  .n.sel { background: var(--sel); }
  .gd { flex: none; width: calc(var(--d) * 14px + 4px); height: 100%; background: repeating-linear-gradient(to right, transparent 0 10px, var(--guide) 10px 11px, transparent 11px 14px); }
  .env { text-transform: uppercase; font-size: 11px; font-weight: 600; letter-spacing: .6px; color: var(--dim); height: 22px; margin-top: 4px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--e); margin: 0 3px; flex: none; }
  .ar { width: 12px; flex: none; color: var(--dim); display: inline-flex; }
  .ar.open :global(svg) { transform: rotate(90deg); }
  .ic { display: inline-flex; flex: none; color: var(--dim); }
  .ic.env { color: var(--e); }
  .l { overflow: hidden; text-overflow: ellipsis; min-width: 3em; }
  .cnt { font-size: 11px; }
  .ty { font-size: 11px; margin-left: 4px; overflow: hidden; text-overflow: ellipsis; flex: 1 1 0; min-width: 0; } /* type gets only the space the name leaves */
  .more { margin-left: auto; display: none; }
  .n:hover .more, .n.sel .more { display: inline-flex; }
  .pad { padding: 12px; }
</style>
