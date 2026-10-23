<script lang="ts">
  import '$lib/app.css';
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { app, ask, activeTab, connOf, closeTab, newTab, loadConns, restoreTabs, saveTabs, envColor, drag, copyText, type Tab } from '$lib/store.svelte';
  import QueryTab from '$lib/QueryTab.svelte';
  import Servers from '$lib/Servers.svelte';
  import Todos from '$lib/Todos.svelte';
  import ProdConfirm from '$lib/ProdConfirm.svelte';
  import Icon from '$lib/Icon.svelte';
  import ContextMenu, { SEP, type MenuItem } from '$lib/ContextMenu.svelte';

  const SW = 'dbs.sidebarW', SW0 = 272;
  let appEl: HTMLDivElement;
  const clampW = (w: number) => Math.round(Math.min(innerWidth * 0.6, Math.max(180, w)));
  const setW = (w: number) => appEl.style.setProperty('--sw', w + 'px'); // CSS var: no re-render, one write per frame

  function sideDrag(e: PointerEvent) {
    const w0 = clampW(appEl.querySelector('aside')!.getBoundingClientRect().width);
    let w = w0;
    drag(e, (dx) => setW((w = clampW(w0 + dx))), () => localStorage.setItem(SW, String(w)));
  }
  const sideReset = () => { setW(SW0); localStorage.removeItem(SW); };

  // Theme: System follows macOS; the title bar follows the choice too.
  const THEMES = ['System', 'Light', 'Dark'];
  let theme = $state(THEMES.find((t) => t.toLowerCase() === localStorage.getItem('dbs.theme')) ?? 'System');
  function applyTheme() {
    (window as unknown as { dbsTheme: () => void }).dbsTheme();
    getCurrentWindow().setTheme(theme === 'System' ? null : (theme.toLowerCase() as 'light' | 'dark')).catch(() => {});
  }
  function nextTheme() {
    theme = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length];
    localStorage.setItem('dbs.theme', theme.toLowerCase());
    applyTheme();
  }

  // Whole-UI zoom (native webview zoom, so editor and grid hit-testing stay correct). 70%..200% in 10% steps; 110% default.
  const ZOOM = 1.1; // default and Cmd+0 level
  let zoom = $state(Number(localStorage.getItem('dbs.zoom')) || ZOOM);
  function setZoom(z: number) {
    zoom = Math.min(2, Math.max(0.7, Math.round(z * 10) / 10));
    localStorage.setItem('dbs.zoom', String(zoom));
    getCurrentWebview().setZoom(zoom).catch(() => {});
  }

  onMount(async () => {
    applyTheme();
    setZoom(zoom);
    setW(+(localStorage.getItem(SW) ?? '') || SW0);
    await loadConns();
    restoreTabs();
  });

  // Debounced persistence: the effect re-runs on any tab change, the cleanup cancels the pending write.
  $effect(() => {
    if (!app.ready) return;
    JSON.stringify([app.tabs.map((t) => [t.sql, t.connId, t.database, t.readOnly, t.maxRows]), app.active]);
    const h = setTimeout(saveTabs, 500);
    return () => clearTimeout(h);
  });

  // Safety net: the strip must never be empty once tabs are restored.
  $effect(() => { if (app.ready && !app.tabs.length) newTab(); });

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const closeMany = (ts: Tab[]) => ts.forEach((x) => closeTab(x.id));
  function tabMenu(e: MouseEvent, t: Tab) {
    e.preventDefault();
    const i = app.tabs.findIndex((x) => x.id === t.id), right = app.tabs.slice(i + 1), c = connOf(t.connId);
    const keep = (rest: Tab[]) => { if (rest.some((x) => x.id === app.active)) app.active = t.id; closeMany(rest); };
    menu = { x: e.clientX, y: e.clientY, items: [
      { label: 'Close', icon: 'close', hint: 'Cmd+W', run: () => closeTab(t.id) },
      { label: 'Close others', run: () => keep(app.tabs.filter((x) => x.id !== t.id)), disabled: app.tabs.length < 2 },
      { label: 'Close tabs to the right', run: () => keep(right), disabled: !right.length },
      { label: 'Close all', run: () => closeMany([...app.tabs]) },
      SEP,
      { label: 'Duplicate tab', icon: 'copy', run: () => { newTab(t.connId, t.database, t.sql); Object.assign(app.tabs[app.tabs.length - 1], { readOnly: t.readOnly, maxRows: t.maxRows }); } },
      { label: 'Copy connection/database name', icon: 'copy', run: () => copyText(`${c?.name ?? ''}/${t.database || c?.database || 'default'}`, 'name') },
    ] };
  }

  const answer = (ok: boolean) => { const r = ask.resolve; ask.resolve = null; r?.(ok); };
  const modal = (d: HTMLDialogElement) => d.showModal();

  const conn = $derived(connOf(activeTab()?.connId ?? ''));
  const title = (t: (typeof app.tabs)[number]) => `SQLQuery_${t.n} - ${connOf(t.connId)?.name ?? 'no connection'}`;

  function key(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey)) return;
    if (e.key === 't') { e.preventDefault(); newTab(); }
    else if (e.key === 'w') { e.preventDefault(); closeTab(app.active); }
    else if (e.key === '=' || e.key === '+') { e.preventDefault(); setZoom(zoom + 0.1); }
    else if (e.key === '-') { e.preventDefault(); setZoom(zoom - 0.1); }
    else if (e.key === '0') { e.preventDefault(); setZoom(ZOOM); }
  }
</script>

<svelte:window onkeydown={key} />

<div class="app" bind:this={appEl}>
  <aside>
    <div class="tabs">
      <button class:on={app.side === 'servers'} onclick={() => (app.side = 'servers')} data-testid="side-servers"><Icon name="server" size={14} />Servers</button>
      <button class:on={app.side === 'todo'} onclick={() => (app.side = 'todo')} data-testid="side-todo"><Icon name="todo" size={14} />To-do</button>
    </div>
    <div class="pane">
      {#if app.side === 'servers'}<Servers />{:else}<Todos />{/if}
    </div>
    <div class="rz" onpointerdown={sideDrag} ondblclick={sideReset} role="separator" aria-orientation="vertical" data-testid="sidebar-resizer"></div>
  </aside>

  <main>
    <div class="strip">
      {#each app.tabs as t (t.id)}
        {@const c = connOf(t.connId)}
        <div class="st" class:on={t.id === app.active} style="--e:{envColor(c?.env)}" onclick={() => (app.active = t.id)} oncontextmenu={(e) => tabMenu(e, t)} onkeydown={() => {}} role="tab" tabindex="-1" aria-selected={t.id === app.active} data-testid="tab">
          <span>{title(t)}</span>
          <button class="icon" onclick={(e) => { e.stopPropagation(); closeTab(t.id); }} title="Close (Cmd+W)"><Icon name="close" size={12} /></button>
        </div>
      {/each}
      <button class="icon add" onclick={() => newTab()} title="New query (Cmd+T)" data-testid="new-tab"><Icon name="plus" size={14} /></button>
    </div>
    <div class="tabs-body">
      {#each app.tabs as t, i (t.id)}<QueryTab bind:tab={app.tabs[i]} active={t.id === app.active} />{/each}
    </div>
  </main>

  <footer style="background:{activeTab() ? envColor(conn?.env) : '#5b6068'}" data-testid="statusbar" data-env={conn?.env ?? ''}>
    {#if !activeTab()}<span>No query open — Cmd+T or right-click a database → New query</span>
    {:else if conn}
      <span><Icon name="server" size={13} />{conn.name}</span>
      <span><Icon name="database" size={13} />{activeTab()?.database || conn.database || 'default'}</span>
      <span><Icon name={conn.read_only || activeTab()?.readOnly ? 'lock' : 'unlock'} size={13} />{conn.read_only || activeTab()?.readOnly ? 'Read-only' : 'Read-write'}</span>
      {#if conn.env === 'prod'}<b class="badge">PRODUCTION</b>{/if}
    {:else}<span>Not connected</span>{/if}
    <span class="grow"></span>
    <span>{app.msg}</span>
    {#if zoom !== ZOOM}<button class="theme" onclick={() => setZoom(ZOOM)} title="Reset zoom (Cmd+0)" data-testid="zoom">{Math.round(zoom * 100)}%</button>{/if}
    <button class="theme" onclick={nextTheme} title="Change theme" data-testid="theme">Theme: {theme}</button>
  </footer>
</div>

{#if ask.resolve}
  <dialog use:modal onclose={() => answer(false)} data-testid="ask-dialog">
    <div class="ask">{ask.msg}</div>
    <div class="askb"><button onclick={() => answer(false)} data-testid="ask-cancel">Cancel</button><button class="primary" onclick={() => answer(true)} data-testid="ask-ok">Continue</button></div>
  </dialog>
{/if}
<ProdConfirm />
{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}

<style>
  .app { display: grid; grid-template-columns: clamp(180px, var(--sw, 272px), 60vw) 1fr; grid-template-rows: 1fr 24px; height: 100vh; }
  aside { position: relative; z-index: 4; display: flex; flex-direction: column; border-right: 1px solid var(--bd); background: var(--bg2); min-height: 0; }
  .rz { position: absolute; top: 0; bottom: 0; right: -4px; width: 8px; cursor: col-resize; touch-action: none; }
  .rz::after { content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 2px; transition: background .1s; }
  .rz:hover::after { background: var(--accent); }
  .tabs { display: flex; border-bottom: 1px solid var(--bd); }
  .tabs button { flex: 1; justify-content: center; border: none; border-bottom: 2px solid transparent; border-radius: 0; background: none; height: 34px; color: var(--dim); }
  .tabs button:hover { background: var(--hover) !important; color: var(--fg); }
  .tabs button.on { border-bottom-color: var(--accent); color: var(--fg); font-weight: 600; }
  .pane { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  main { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .strip { display: flex; align-items: flex-end; background: var(--bg2); border-bottom: 1px solid var(--bd); overflow-x: auto; flex: none; height: 34px; }
  .st { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 6px 0 12px; border-top: 2px solid var(--e); border-right: 1px solid var(--bd); white-space: nowrap; cursor: pointer; color: var(--dim); background: var(--bg2); }
  .st:hover { background: var(--hover); }
  .st.on { background: var(--bg); color: var(--fg); height: 34px; margin-bottom: -1px; }
  .add { margin: 0 6px 5px; flex: none; }
  .tabs-body { flex: 1; min-height: 0; }
  footer { grid-column: 1 / -1; display: flex; gap: 16px; align-items: center; padding: 0 12px; color: #fff; font-size: 12px; }
  footer span { display: inline-flex; align-items: center; gap: 5px; }
  .theme { height: 18px; padding: 0 6px; background: none; border: none; color: inherit; font-size: 12px; }
  .theme:hover:not(:disabled) { background: rgba(255, 255, 255, .18); }
  .badge { background: rgba(0, 0, 0, .25); padding: 0 6px; border-radius: 3px; font-size: 11px; letter-spacing: .5px; }
  .ask { max-width: 380px; padding-bottom: 14px; white-space: pre-wrap; }
  .askb { display: flex; gap: 8px; justify-content: flex-end; }
</style>
