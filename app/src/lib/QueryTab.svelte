<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorView, keymap, lineNumbers, drawSelection, highlightActiveLine } from '@codemirror/view';
  import { EditorState, Compartment } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap, selectAll, toggleComment } from '@codemirror/commands';
  import { autocompletion, closeBrackets, closeBracketsKeymap, startCompletion } from '@codemirror/autocomplete';
  import { syntaxHighlighting, HighlightStyle, bracketMatching } from '@codemirror/language';
  import { tags as t } from '@lezer/highlight';
  import { api, clipboard, type QueryResult } from './api';
  import { app, connOf, databases, execute, CANCELLED, drag, ui, qid, copyText, KEYWORDS, intel, schemaKey, schemaOf, loadSchema, clearSchema, type Tab } from './store.svelte';
  import Grid from './Grid.svelte';
  import Icon from './Icon.svelte';
  import ContextMenu, { SEP, type MenuItem } from './ContextMenu.svelte';

  const LIMITS: [number, string][] = [[100, 'Limit: 100 (default)'], [1000, 'Limit: 1,000'], [10000, 'Limit: 10,000'], [100000, 'Limit: 100,000'], [0, 'No limit']];

  let { tab = $bindable(), active }: { tab: Tab; active: boolean } = $props();

  let host: HTMLDivElement;
  let view: EditorView;
  const lang = new Compartment(); // SQL language + schema; reconfigured when conn/db changes, no editor rebuild
  let wanted = false; // IntelliSense loads lazily: first focus or first keystroke
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let timer: ReturnType<typeof setTimeout>;
  let tbEl: HTMLDivElement;
  let running = $state(false);
  let res = $state.raw<QueryResult | null>(null);
  let err = $state('');
  let pane = $state(0); // index of result set, -1 = Messages
  let dbs = $state<string[]>([]);
  let dbErr = $state('');
  let ms = $state(0);
  let cancelled = $state(false);
  let el = $state(0); // elapsed seconds while running
  let rid = 0; // run token: a late result of an older run is ignored

  const conn = $derived(connOf(tab.connId));
  $effect(() => { if (conn?.database && tab.database) tab.database = ''; }); // pinned: always the connection's database
  const prod = $derived(conn?.env === 'prod');
  const ro = $derived(!!conn?.read_only || tab.readOnly);
  const rs = $derived(res && pane >= 0 ? res.results[pane] : null);
  const messages = $derived(cancelled ? ['Query cancelled.'] : err ? [err] : (res?.messages ?? []));
  const dbName = $derived(tab.database || conn?.database || 'default');

  const hl = HighlightStyle.define([
    { tag: [t.keyword, t.operatorKeyword, t.modifier], color: 'var(--syn-kw)' },
    { tag: [t.string, t.special(t.string)], color: 'var(--syn-str)' },
    { tag: [t.number, t.bool, t.null], color: 'var(--syn-num)' },
    { tag: [t.lineComment, t.blockComment], color: 'var(--syn-cm)', fontStyle: 'italic' },
    { tag: [t.typeName, t.standard(t.name)], color: 'var(--syn-ty)' },
  ]);

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: tab.sql,
        extensions: [
          lineNumbers(), history(), drawSelection(), highlightActiveLine(), bracketMatching(),
          closeBrackets(),
          autocompletion({ activateOnTyping: true, maxRenderedOptions: 100, icons: true }),
          // Open the list after "dbo." and after "FROM " / "JOIN " etc., like SSMS.
          EditorView.updateListener.of((u) => {
            if (!u.docChanged || !u.transactions.some((tr) => tr.isUserEvent('input.type'))) return;
            const pos = u.state.selection.main.head;
            const before = u.state.sliceDoc(Math.max(0, pos - 12), pos);
            if (/\.$|\b(FROM|JOIN|UPDATE|INTO|TABLE|EXEC|EXECUTE)\s$/i.test(before)) queueMicrotask(() => startCompletion(u.view));
          }),
          keymap.of([{ key: 'Mod-Enter', run: () => (run(), true) }, { key: 'Mod-Shift-r', run: () => (refreshIntel(), true), preventDefault: true }, { key: 'Escape', run: () => running && (cancel(), true) }, ...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap]),
          lang.of(KEYWORDS),
          syntaxHighlighting(hl),
          EditorView.theme({
            '&': { height: '100%', background: 'var(--bg)', color: 'var(--fg)', fontSize: '13px' },
            '.cm-scroller': { fontFamily: 'var(--mono)' },
            '.cm-gutters': { background: 'var(--bg2)', color: 'var(--dim)', border: 'none' },
            '.cm-activeLine, .cm-activeLineGutter': { background: 'color-mix(in srgb, var(--fg) 6%, transparent)' },
            '.cm-cursor': { borderLeftColor: 'var(--fg)' },
            '.cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': { background: 'var(--sel)' },
            '&.cm-focused': { outline: 'none' },
            '.cm-tooltip': { background: 'var(--bg)', color: 'var(--fg)', border: '1px solid var(--bd2)', borderRadius: '6px' },
            '.cm-tooltip-autocomplete ul': { fontFamily: 'var(--mono)', fontSize: '12.5px' },
            '.cm-tooltip-autocomplete ul li[aria-selected]': { background: 'var(--accent)', color: '#fff' },
          }),
          EditorView.domEventHandlers({ focus: () => { want(); } }),
          EditorView.updateListener.of((u) => {
            if (!u.docChanged) return;
            want();
            clearTimeout(timer);
            timer = setTimeout(() => (tab.sql = view.state.doc.toString()), 300);
          }),
        ],
      }),
    });
    if (tab.autorun) { tab.autorun = false; run(); }
    return () => { clearTimeout(timer); view.destroy(); };
  });

  $effect(() => { if (active) { view?.requestMeasure(); view?.focus(); } });

  // IntelliSense: schema comes from the shared per-conn+db cache; swapping it is one compartment reconfigure.
  function want() { if (!wanted) { wanted = true; applySchema(); } }
  async function applySchema() {
    const key = schemaKey(tab.connId, tab.database);
    if (!wanted || !view) return;
    if (!key) { view.dispatch({ effects: lang.reconfigure(KEYWORDS) }); return; }
    try {
      const { ext } = await loadSchema(key);
      if (key === schemaKey(tab.connId, tab.database)) view.dispatch({ effects: lang.reconfigure(ext) });
    } catch (e) { app.msg = `IntelliSense: ${e}`; }
  }
  $effect(() => {
    tab.connId; tab.database; conn?.database; intel.gen;
    if (!wanted) return;
    // Keywords only until the new schema arrives (never offer the old database's tables).
    view?.dispatch({ effects: lang.reconfigure(KEYWORDS) });
    applySchema();
  });
  function refreshIntel() {
    const key = schemaKey(tab.connId, tab.database);
    if (!key) return;
    wanted = true;
    clearSchema(tab.connId, key.slice(key.indexOf('|') + 1)); // bumps intel.gen: the effect above reloads
    app.msg = 'Refreshing IntelliSense…';
  }

  // "Expand *": cursor on the * of `SELECT [TOP..] * ... FROM [schema.]table`; columns come from the loaded cache.
  const ID = String.raw`(?:\[(?:[^\]]|\]\])+\]|"[^"]+"|\w+)`;
  const SELECT_BEFORE = /(?:\bSELECT(?:\s+(?:DISTINCT|ALL))?(?:\s+TOP\s*(?:\(\s*\w+\s*\)|\d+)(?:\s+PERCENT)?)?|,)\s*$/i;
  const FROM_AFTER = new RegExp(String.raw`\bFROM\s+(${ID}(?:\s*\.\s*${ID}){0,2})`, 'i');
  const unq = (s: string) => (s[0] === '[' ? s.slice(1, -1).replaceAll(']]', ']') : s[0] === '"' ? s.slice(1, -1) : s);
  function expandStar(pos: number | null) {
    const schema = schemaOf(schemaKey(tab.connId, tab.database));
    if (!schema || pos === null) return null;
    const d = view.state.doc;
    const at = [pos, pos - 1].find((p) => p >= 0 && d.sliceString(p, p + 1) === '*');
    if (at === undefined || !SELECT_BEFORE.test(d.sliceString(Math.max(0, at - 200), at))) return null;
    const m = FROM_AFTER.exec(d.sliceString(at + 1, Math.min(d.length, at + 5001)));
    if (!m) return null;
    const parts = (m[1].match(new RegExp(ID, 'g')) ?? []).map(unq); // [schema.]table; a 3-part name (other database) is not supported
    if (parts.length > 2) return null;
    const [sch, tbl] = parts.length === 2 ? parts : ['dbo', parts[0]];
    const ci = (o: Record<string, unknown>, k: string) => Object.keys(o).find((x) => x.toLowerCase() === k.toLowerCase());
    const s1 = ci(schema, sch), t1 = s1 && ci(schema[s1], tbl), cols = s1 && t1 ? schema[s1][t1] : null;
    return cols?.length ? { at, text: cols.map(qid).join(', ') } : null;
  }

  function editorMenu(e: MouseEvent) {
    e.preventDefault();
    const sm = view.state.selection.main;
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY }, false);
    if (pos !== null && (sm.empty || pos < sm.from || pos > sm.to)) view.dispatch({ selection: { anchor: pos } });
    const m = view.state.selection.main, has = !m.empty, text = view.state.sliceDoc(m.from, m.to);
    const star = expandStar(m.head);
    const f = (fn: () => void) => () => { view.focus(); fn(); };
    menu = { x: e.clientX, y: e.clientY, items: [
      { label: 'Run', icon: 'run', hint: 'Cmd+Enter', run: () => run() },
      { label: 'Run selection', run: () => run(), disabled: !has },
      SEP,
      { label: 'Cut', disabled: !has, run: f(() => { clipboard.write(text).then(() => view.dispatch({ changes: { from: m.from, to: m.to }, userEvent: 'delete.cut' }), (er) => (app.msg = String(er))); }) },
      { label: 'Copy', icon: 'copy', disabled: !has, run: f(() => { clipboard.write(text).catch((er) => (app.msg = String(er))); }) },
      { label: 'Paste', run: f(() => { clipboard.read().then((t) => t && view.dispatch({ ...view.state.replaceSelection(t), scrollIntoView: true, userEvent: 'input.paste' }), (er) => (app.msg = String(er))); }) },
      { label: 'Select all', run: f(() => { selectAll(view); }) },
      SEP,
      { label: 'Toggle comment', hint: 'Cmd+/', run: f(() => { toggleComment(view); }) },
      { label: 'Expand *', disabled: !star, run: f(() => star && view.dispatch({ changes: { from: star.at, to: star.at + 1, insert: star.text } })) },
      { label: 'Add to To-do', icon: 'todo', run: () => addTodo() },
      { label: 'Refresh IntelliSense', icon: 'refresh', hint: 'Cmd+Shift+R', run: () => refreshIntel(), disabled: !schemaKey(tab.connId, tab.database) },
    ] };
  }

  // Load database list lazily: only once the tab is active and has a connection.
  $effect(() => {
    const id = tab.connId;
    if (!active || !id) return;
    dbErr = '';
    databases(id).then((d) => id === tab.connId && (dbs = d), (e) => id === tab.connId && ((dbs = []), (dbErr = String(e))));
  });

  function currentSql() {
    const m = view.state.selection.main;
    const s = m.empty ? '' : view.state.sliceDoc(m.from, m.to);
    return s.trim() ? s : view.state.doc.toString();
  }

  $effect(() => {
    if (!running) return;
    const t0 = performance.now();
    const h = setInterval(() => (el = (performance.now() - t0) / 1000), 100);
    return () => clearInterval(h);
  });

  async function run() {
    if (running) return;
    const text = currentSql();
    if (!text.trim()) return;
    if (!conn) { err = 'Select a connection first.'; cancelled = false; pane = -1; return; }
    const my = ++rid;
    running = true; err = ''; cancelled = false; el = 0; const t0 = performance.now();
    try {
      const r = await execute(conn, tab.database, tab.id, text, tab.readOnly, tab.maxRows);
      if (my !== rid) return;
      res = r; ms = r.elapsed_ms;
      pane = r.results.length ? 0 : -1;
    } catch (e) {
      if (my !== rid) return;
      res = null; err = e === CANCELLED ? '' : String(e); pane = -1; ms = Math.round(performance.now() - t0);
    } finally {
      if (my === rid) running = false;
    }
  }

  // The backend drops the session connection, so the running call rejects.
  async function cancel() {
    const my = rid;
    try { await api.cancelQuery(tab.id); } catch (e) { err = String(e); return; }
    await new Promise((r) => setTimeout(r, 500)); // normally the run's own rejection (with the backend's note) lands first
    if (running && my === rid) { rid++; running = false; res = null; err = ''; cancelled = true; pane = -1; }
  }

  async function addTodo() {
    const text = view.state.doc.toString();
    if (!text.trim()) return;
    const title = text.trim().split('\n')[0].slice(0, 60); // no prompt(): unreliable in the macOS webview
    try {
      await api.addTodo({ title, target: conn?.name ?? null, database: tab.database || conn?.database || null, sql: text, by: 'me', card: null });
      app.msg = 'Added to To-do';
    } catch (e) { app.msg = String(e); }
  }

  function tsv() {
    if (!rs) return;
    const f = (v: string | null) => (v ?? '').replace(/[\t\r\n]+/g, ' ');
    copyText([rs.columns.join('\t'), ...rs.rows.map((r) => r.map(f).join('\t'))].join('\n'), 'result as TSV');
  }

  function split(e: PointerEvent) {
    const h0 = host.offsetHeight, max = (e.currentTarget as HTMLElement).parentElement!.clientHeight - tbEl.offsetHeight - 6 - 80;
    drag(e, (_, dy) => (ui.edH = Math.round(Math.max(80, Math.min(max, h0 + dy)))), () => localStorage.setItem('dbs.edH', String(ui.edH)));
  }

  function key(e: KeyboardEvent) {
    // Cmd+Enter when focus is outside the editor (grid, toolbar); the editor handles its own.
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && !e.defaultPrevented) { e.preventDefault(); run(); }
  }
</script>

{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}
<div class="tab" hidden={!active} onkeydown={key} role="presentation">
  <div class="tb" bind:this={tbEl}>
    <select bind:value={tab.connId} onchange={(e) => { tab.database = ''; if (connOf(e.currentTarget.value)?.env === 'prod') tab.readOnly = true; }} title="Connection" data-testid="conn-select">
      {#if !conn}<option value={tab.connId}>(no connection)</option>{/if}
      {#each app.conns as c}<option value={c.id}>{c.name}</option>{/each}
    </select>
    {#if !conn?.database}<!-- a connection pinned to one database has no database choice -->
    <select bind:value={tab.database} title={dbErr || 'Database'} data-testid="db-select">
      <option value="">(default)</option>
      {#if tab.database && !dbs.includes(tab.database)}<option value={tab.database}>{tab.database}</option>{/if}
      {#each dbs as d}<option value={d}>{d}</option>{/each}
    </select>
    {/if}
    <span class="sep"></span>
    {#if running}
      <button class="stop" onclick={cancel} title="Cancel (Esc)" data-testid="cancel"><Icon name="stop" size={14} />Cancel</button>
      <span class="dim" data-testid="elapsed">Running… {el.toFixed(1)} s</span>
    {:else}
      <button class="run" onclick={run} title="Run (Cmd+Enter; selection if any)" data-testid="run"><Icon name="run" size={14} />Run</button>
    {/if}
    <span class="grow"></span>
    <select bind:value={tab.maxRows} title="Row limit per result set" data-testid="limit-select">
      {#each LIMITS as [v, l]}<option value={v}>{l}</option>{/each}
    </select>
    <button class="lock" class:rw={!ro} class:prod={prod} disabled={!!conn?.read_only} onclick={() => (tab.readOnly = !tab.readOnly)} data-testid="lock-toggle" data-locked={ro}
      title={conn?.read_only ? 'This connection is read-only' : 'Read-only: only SELECT-style queries run. Read-write: writes allowed (prod still needs typed confirmation).'}>
      <Icon name={ro ? 'lock' : 'unlock'} size={14} />{ro ? 'Read-only' : 'Read-write'}
    </button>
    <button onclick={addTodo} title="Save this SQL as a to-do" data-testid="add-todo"><Icon name="todo" size={14} />Add to To-do</button>
  </div>

  <div class="ed" style="height:{ui.edH}px" bind:this={host} oncontextmenu={editorMenu} role="presentation"></div>
  <div class="split" onpointerdown={split} role="separator" aria-orientation="horizontal" data-testid="editor-resizer"></div>

  <div class="res">
    <div class="rt">
      {#if res}
        {#each res.results as _, i}
          <button class="rtab" class:on={pane === i} onclick={() => (pane = i)} data-testid="results-tab">Results {i + 1}</button>
        {/each}
      {/if}
      <button class="rtab" class:on={pane === -1} onclick={() => (pane = -1)} data-testid="messages-tab">Messages</button>
      <span class="grow"></span>
      {#if rs}<button class="ghost" onclick={tsv} title="Copy result as TSV"><Icon name="copy" size={14} />Copy as TSV</button>{/if}
    </div>
    {#if rs?.truncated}
      <div class="banner" data-testid="truncated-banner">Showing first {rs.rows.length.toLocaleString()} rows (row limit). Change the limit to load more.</div>
    {/if}
    <div class="out">
      {#if rs}
        {#key rs}<Grid {rs} />{/key}
      {:else}
        <div class="msgs mono" data-testid="messages">
          {#each messages as m}<div class:err={!!err && !err.startsWith('Query cancelled.')}>{m}</div>{/each}
        </div>
      {/if}
    </div>
    <div class="st dim" data-testid="status">
      {#if rs}{rs.rows.length.toLocaleString()} rows{rs.truncated ? ' (truncated)' : ''} · {/if}{res || err || cancelled ? ms + ' ms · ' : ''}{conn?.name ?? '-'}/{dbName}
    </div>
  </div>
</div>

<style>
  .tab { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .tab[hidden] { display: none; }
  .tb { display: flex; gap: 6px; align-items: center; padding: 6px 10px; background: var(--bg2); border-bottom: 1px solid var(--bd); }
  .lock.rw.prod { color: var(--prod); border-color: color-mix(in srgb, var(--prod) 50%, var(--bd2)); }
  .lock:disabled { opacity: 1; cursor: not-allowed; }
  .ed { flex: 0 1 auto; min-height: 80px; }
  .split { flex: none; position: relative; height: 6px; cursor: row-resize; touch-action: none; }
  .split::after { content: ''; position: absolute; left: 0; right: 0; top: 2px; height: 1px; background: var(--bd2); }
  .split:hover::after { top: 1px; height: 3px; background: var(--accent); }
  .res { flex: 1; min-height: 80px; display: flex; flex-direction: column; }
  .rt { display: flex; gap: 2px; padding: 0 8px; background: var(--bg2); border-bottom: 1px solid var(--bd); align-items: center; height: 30px; flex: none; }
  .rtab { background: none; border: none; border-bottom: 2px solid transparent; border-radius: 0; height: 30px; color: var(--dim); }
  .rtab:hover:not(:disabled) { background: none; color: var(--fg); }
  .rtab.on { border-bottom-color: var(--accent); color: var(--fg); }
  .banner { flex: none; padding: 4px 12px; background: color-mix(in srgb, var(--qa) 18%, var(--bg)); border-bottom: 1px solid color-mix(in srgb, var(--qa) 40%, transparent); font-size: 12px; }
  .out { flex: 1; min-height: 0; overflow: hidden; }
  .msgs { padding: 8px 12px; height: 100%; overflow: auto; user-select: text; -webkit-user-select: text; white-space: pre-wrap; font-size: 12px; }
  .st { padding: 2px 12px; border-top: 1px solid var(--bd); background: var(--bg2); font-size: 12px; flex: none; }
</style>
