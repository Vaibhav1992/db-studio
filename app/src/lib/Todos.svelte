<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Todo } from './api';
  import { app, newTab, guarded, confirmDlg, CANCELLED, resolveTarget, envColor, copyText } from './store.svelte';
  import Icon from './Icon.svelte';
  import ContextMenu, { SEP, type MenuItem } from './ContextMenu.svelte';

  let todos = $state.raw<Todo[]>([]);
  let adding = $state(false);
  let form = $state({ title: '', target: '', database: '', sql: '', card: '' });
  let busy = $state('');

  const tm = (s: string | null) => Date.parse((s ?? '').replace(' ', 'T')) || 0;
  const sorted = (st: string) => todos.filter((t) => t.status === st).sort((a, b) => tm(b.created) - tm(a.created) || b.id.localeCompare(a.id));
  const open = $derived(sorted('todo'));
  const done = $derived(sorted('done'));
  const failed = $derived(sorted('failed'));

  let doneOpen = $state(localStorage.getItem('dbs.todoDone') === '1'); // Done group is collapsed by default
  const toggleDone = () => localStorage.setItem('dbs.todoDone', (doneOpen = !doneOpen) ? '1' : '0');
  let more = $state<Record<string, boolean>>({}); // expanded result text, by todo id
  let showSql = $state<Record<string, boolean>>({});
  let exp = $state<Record<string, boolean>>({}); // done cards are one compact block until clicked

  // Relative only for the past (< 7 days); future or older timestamps get a short date.
  function when(s: string | null) {
    const t = tm(s), d = (Date.now() - t) / 1000;
    if (!t) return s ?? '';
    if (d >= 0 && d < 7 * 86400) return d < 60 ? 'just now' : d < 3600 ? `${~~(d / 60)} min ago` : d < 86400 ? `${~~(d / 3600)} h ago` : `${~~(d / 86400)} d ago`;
    return new Date(t).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  }

  // Backend note: "<result> · <connection>/<database> · <YYYY-MM-DD HH:MM>"; anything else is shown as-is.
  const WARN = /\s*Statements before the error may have run and committed\.?/;
  const MSG = /^(Msg \d+), Level \d+, State \d+(?:, Procedure [^,]+)?(?:, Line (\d+))?\s+([\s\S]*)$/;
  function parse(note: string | null) {
    const n = (note ?? '').trim();
    if (!n) return null;
    const p = n.split(' · ');
    const ok = p.length >= 3 && /^\d{4}-\d\d-\d\d \d\d:\d\d$/.test(p[p.length - 1]);
    let msg = ok ? p.slice(0, -2).join(' · ') : n, code = '';
    const warn = WARN.test(msg);
    msg = msg.replace(WARN, '').trim();
    const m = MSG.exec(msg);
    if (m) { msg = m[3]; code = m[1] + (m[2] ? ' · Line ' + m[2] : ''); }
    return { msg, code, warn, place: ok ? p[p.length - 2] : '', time: ok ? p[p.length - 1] : '' };
  }

  async function refresh() {
    try { todos = await api.listTodos(); } catch (e) { app.msg = String(e); }
  }

  // Agents write todo files on disk: poll while this panel is mounted, and on window focus.
  onMount(() => {
    refresh();
    const h = setInterval(refresh, 5000);
    addEventListener('focus', refresh);
    return () => { clearInterval(h); removeEventListener('focus', refresh); };
  });

  // Preview skips blank and banner-only comment lines ("--", "-- #####").
  const preview = (sql: string, full: boolean) => (full ? sql : sql.split('\n').filter((l) => !/^\s*(--[\s#=*-]*)?$/.test(l)).slice(0, 3).join('\n'));

  function openIn(t: Todo) {
    const c = resolveTarget(t.target).conn;
    if (c) newTab(c.id, t.database ?? '', t.sql);
  }

  // The backend re-reads the file, resolves the target, runs, and writes status + note itself.
  async function run(t: Todo) {
    const c = resolveTarget(t.target).conn;
    busy = t.id;
    try {
      await guarded({ conn: c?.name ?? '', db: t.database || c?.database || 'default', sql: t.sql, card: t.card || null }, (confirm) => api.runTodo(t.id, confirm));
    } catch (e) {
      if (e !== CANCELLED) app.msg = String(e).split('\n')[0];
    } finally {
      busy = '';
      refresh();
    }
  }

  const setStatus = (t: Todo, s: 'todo' | 'done') => api.setTodoStatus(t.id, s, t.note).then(refresh, (e) => (app.msg = String(e)));

  async function del(t: Todo) {
    if (await confirmDlg(`Delete to-do "${t.title}"?`)) api.deleteTodo(t.id).then(refresh, (e) => (app.msg = String(e)));
  }

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  function ctx(e: MouseEvent, t: Todo, at?: HTMLElement) {
    e.preventDefault(); e.stopPropagation();
    const b = at?.getBoundingClientRect();
    const ok = !!resolveTarget(t.target).conn, done = t.status === 'done';
    menu = { x: b ? b.right - 160 : e.clientX, y: b ? b.bottom : e.clientY, items: [
      { label: 'Open', icon: 'file', run: () => openIn(t), disabled: !ok },
      { label: 'Run', icon: 'run', run: () => run(t), disabled: !ok || !!busy },
      { label: 'Copy SQL', icon: 'copy', run: () => copyText(t.sql, 'SQL') },
      SEP,
      { label: done ? 'Reopen' : 'Mark done', icon: 'check', run: () => setStatus(t, done ? 'todo' : 'done') },
      { label: 'Delete', icon: 'trash', danger: true, run: () => del(t) },
    ] };
  }

  async function add() {
    if (!form.title.trim() || !form.sql.trim()) return;
    try {
      await api.addTodo({ title: form.title, target: form.target || null, database: form.database || null, sql: form.sql, by: 'me', card: form.card.trim() || null });
      form = { title: '', target: '', database: '', sql: '', card: '' };
      adding = false;
      refresh();
    } catch (e) { app.msg = String(e); }
  }
</script>

{#snippet item(t: Todo)}
  {@const r = resolveTarget(t.target)}
  {@const n = parse(t.note)}
  {@const isDone = t.status === 'done'}
  {@const full = !isDone || exp[t.id]}
  {@const db = t.database || r.conn?.database || 'default'}
  {@const place = n?.place && !(r.conn && (n.place === `${r.conn.name}/${db}` || n.place === `${r.conn.name}/default`)) ? n.place : ''}
  {@const meta = [t.by, t.created && when(t.created)].filter(Boolean).join(' · ')}
  <div class="t {t.status}" class:compact={!full} oncontextmenu={(e) => ctx(e, t)} role="presentation" data-testid="todo-card" data-id={t.id} data-status={t.status} data-title={t.title}>
    {#if isDone}
      <button class="head" onclick={() => (exp[t.id] = !exp[t.id])} aria-expanded={!!exp[t.id]} data-testid="todo-expand">
        <span class="ic"><Icon name="check" size={13} /></span><span class="ttl" title={t.title}>{t.title}</span>{#if n?.time}<span class="dim sm when">{n.time.slice(5)}</span>{/if}
      </button>
    {:else}
      <div class="ttl" title={t.title}>{t.title}</div>
    {/if}
    <div class="chips">
      {#if r.conn}<span class="chip env" style="--e:{envColor(r.conn.env)}" data-testid="todo-target">{r.conn.name}{t.database && t.database !== r.conn.database ? ` · ${t.database}` : ''}</span>
      {:else}<span class="chip bad" data-testid="todo-target">{r.err}{t.target ? `: ${t.target}` : ''}</span>{/if}
      {#if t.card}<span class="chip" data-testid="todo-card-id">{t.card}</span>{/if}
      {#if !full && n}<span class="dim sm one">{n.msg}</span>{:else if meta}<span class="dim sm one meta">{meta}</span>{/if}
    </div>
    {#if full}
      {#if n}
        {@const long = n.msg.length > 80}
        <div class="note" class:bad={t.status === 'failed'} data-testid="todo-note">
          {#if t.status === 'failed'}<span class="ic bad"><Icon name="close" size={13} /></span>{/if}
          <div class="nb">
            <div class="msg" class:clamp={!more[t.id]} class:bad={t.status === 'failed'}>{n.msg}{#if n.code}<span class="dim sfx"> {n.code}</span>{/if}</div>
            {#if n.warn}<div class="dim sm">Statements before the error may have run and committed.</div>{/if}
            {#if long}<button class="lnk" onclick={() => (more[t.id] = !more[t.id])}>{more[t.id] ? 'less' : 'more'}</button>{/if}
            {#if place || (n.time && !isDone)}<div class="dim sm">{place ? `ran on ${place} · ` : 'ran '}{n.time}</div>{/if}
          </div>
        </div>
      {/if}
      <pre class="mono" class:full={showSql[t.id]} title="Click to show all SQL" onclick={() => (showSql[t.id] = !showSql[t.id])} role="presentation" data-testid="todo-sql-toggle">{preview(t.sql, !!showSql[t.id])}</pre>
      <div class="btns">
        {#if !isDone}
          <button class="primary" onclick={() => run(t)} disabled={!r.conn || !!busy} data-testid="todo-run">
            {#if busy === t.id}<span class="spin"></span>Running…{:else}<Icon name="run" size={11} />{t.status === 'failed' ? 'Retry' : 'Run'}{/if}
          </button>
        {/if}
        <button onclick={() => openIn(t)} disabled={!r.conn} data-testid="todo-open">Open</button>
        {#if isDone}
          <button onclick={() => run(t)} disabled={!r.conn || !!busy} data-testid="todo-run">{#if busy === t.id}<span class="spin"></span>Running…{:else}Run again{/if}</button>
        {:else}
          <button onclick={() => setStatus(t, 'done')} data-testid="todo-toggle">Mark done</button>
        {/if}
        <button class="icon" title="More" onclick={(e) => ctx(e, t, e.currentTarget)} data-testid="todo-more"><Icon name="more" size={14} /></button>
      </div>
    {/if}
  </div>
{/snippet}

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}
<div class="bar">
  <button onclick={() => (adding = !adding)} data-testid="todo-add"><Icon name="plus" size={14} />Add</button>
  <span class="grow"></span>
  <button class="icon" onclick={refresh} title="Refresh"><Icon name="refresh" size={14} /></button>
</div>
<div class="list">
  {#if adding}
    <form class="t add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <input placeholder="Title" bind:value={form.title} />
      <input placeholder="Target (exact connection name)" list="targets" bind:value={form.target} />
      <datalist id="targets">{#each app.conns as c}<option value={c.name}></option>{/each}</datalist>
      <input placeholder="Database" bind:value={form.database} />
      <input placeholder="Card (ticket id, optional)" bind:value={form.card} />
      <textarea class="mono" rows="5" placeholder="SQL" bind:value={form.sql}></textarea>
      <div class="btns"><button class="primary">Add</button><button type="button" onclick={() => (adding = false)}>Cancel</button></div>
    </form>
  {/if}
  <section>
    <div class="h">To do <span class="n">{open.length}</span></div>
    {#each open as t (t.id)}{@render item(t)}{/each}
  </section>
  {#if failed.length}
    <section>
      <div class="h">Failed <span class="n">{failed.length}</span></div>
      {#each failed as t (t.id)}{@render item(t)}{/each}
    </section>
  {/if}
  <section>
    <button class="h btn" onclick={toggleDone} aria-expanded={doneOpen} data-testid="todo-done-toggle">
      <span class="chev" class:open={doneOpen}><Icon name="chevron" size={12} /></span>Done <span class="n">{done.length}</span>
    </button>
    {#if doneOpen}{#each done as t (t.id)}{@render item(t)}{/each}{/if}
  </section>
</div>

<style>
  .bar { display: flex; align-items: center; gap: 4px; padding: 6px 8px; border-bottom: 1px solid var(--bd); }
  .list { flex: 1; overflow: auto; padding-bottom: 8px; }
  .h { position: sticky; top: 0; z-index: 2; display: flex; align-items: center; width: 100%; height: auto; padding: 8px 10px 6px; background: var(--bg2); font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: .6px; color: var(--dim); }
  .h.btn { border: none; border-radius: 0; gap: 0; }
  .h.btn:hover:not(:disabled) { background: var(--bg2); color: var(--fg); }
  .chev { display: inline-flex; margin-right: 4px; transition: transform .1s; }
  .chev.open { transform: rotate(90deg); }
  .n { background: var(--bg3); border-radius: 8px; padding: 0 6px; margin-left: 6px; font-weight: 500; }
  .t { --sc: var(--accent); margin: 0 8px 8px; padding: 12px 12px 12px 14px; background: var(--bg); border: 1px solid var(--bd); border-left: 3px solid var(--sc); border-radius: 6px; display: flex; flex-direction: column; align-items: stretch; gap: 8px; font-size: 13px; user-select: text; -webkit-user-select: text; }
  .t.failed { --sc: var(--err); }
  .t.done { --sc: var(--ok); }
  .t:hover { border-color: var(--bd2); border-left-color: var(--sc); }
  .t.compact { gap: 4px; padding-top: 8px; padding-bottom: 8px; }
  .head { display: flex; align-items: center; gap: 6px; height: auto; padding: 0; border: none; background: none; text-align: left; width: 100%; }
  .head:hover:not(:disabled) { background: none; }
  .head .ttl { flex: 1; min-width: 0; -webkit-line-clamp: 1; line-clamp: 1; }
  .when { flex: none; }
  .one { min-width: 0; flex: 1 1 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { text-align: right; }
  .ttl { font-size: 13px; font-weight: 600; line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word; }
  .chips { display: flex; gap: 6px; align-items: center; }
  .sm { font-size: 11.5px; }
  .note { display: flex; gap: 6px; font-size: 12.5px; }
  .nb { min-width: 0; display: flex; flex-direction: column; gap: 2px; align-items: flex-start; }
  .ic { color: var(--ok); display: inline-flex; margin-top: 2px; flex: none; }
  .ic.bad, .msg.bad { color: var(--err); }
  .msg { word-break: break-word; line-height: 1.4; }
  .msg.clamp { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .sfx { font-size: 11.5px; }
  .lnk { height: auto; padding: 0; border: none; background: none; color: var(--accent); font-size: 11.5px; align-self: flex-start; }
  .lnk:hover:not(:disabled) { background: none; text-decoration: underline; }
  pre { cursor: pointer; margin: 0; padding: 5px 8px; background: var(--bg2); border-radius: 4px; overflow: hidden; font-size: 11.5px; line-height: 1.4; max-height: calc(3 * 1.4em + 10px); white-space: pre; -webkit-mask-image: linear-gradient(to right, #000 calc(100% - 28px), transparent); mask-image: linear-gradient(to right, #000 calc(100% - 28px), transparent); }
  pre.full { overflow: auto; max-height: 300px; -webkit-mask-image: none; mask-image: none; }
  .btns { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
  .btns button { height: 24px; padding: 0 9px; font-size: 12px; }
  .btns .primary { gap: 4px; }
  .btns .icon { margin-left: auto; width: 24px; padding: 0; }
</style>
