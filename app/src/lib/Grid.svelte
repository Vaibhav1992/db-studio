<script lang="ts">
  import type { ResultSet } from './api';
  import { drag, copyText, qid } from './store.svelte';
  import ContextMenu, { SEP, type MenuItem } from './ContextMenu.svelte';

  const RH = 24, OVER = 8, GUTTER = 50;
  let { rs }: { rs: ResultSet } = $props();

  let el: HTMLDivElement;
  let top = $state(0);
  let vh = $state(400);
  let sel = $state.raw<{ r0: number; c0: number; r1: number; c1: number } | null>(null);
  let resized = $state.raw<Record<number, number>>({});

  // rs.rows is only ever read, never proxied (props are not deep reactive).
  const est = $derived(
    rs.columns.map((h, c) => {
      let m = 0;
      for (let r = 0, n = Math.min(100, rs.rows.length); r < n; r++) m = Math.max(m, (rs.rows[r][c] ?? 'NULL').length);
      // Header is bold proportional text (~8.5px/char), cells are monospace (~7.3px/char).
      return Math.min(400, Math.max(60, Math.round(h.length * 8.5) + 24, Math.round(m * 7.3) + 18));
    })
  );
  const NUM = /^-?(0|[1-9]\d*)(\.\d+)?(e[+-]?\d+)?$/i;
  const num = $derived(
    rs.columns.map((_, c) => {
      let n = 0;
      for (let r = 0, m = Math.min(100, rs.rows.length); r < m; r++) {
        const v = rs.rows[r][c];
        if (v === null) continue;
        if (!NUM.test(v)) return false;
        n++;
      }
      return n > 0;
    })
  );
  const widths = $derived(est.map((w, i) => resized[i] ?? w));
  const cols = $derived(`${GUTTER}px ${widths.map((w) => w + 'px').join(' ')}`);
  const total = $derived(GUTTER + widths.reduce((a, b) => a + b, 0));
  const start = $derived(Math.max(0, Math.floor(top / RH) - OVER));
  const end = $derived(Math.min(rs.rows.length, Math.ceil((top + vh) / RH) + OVER));
  const range = $derived(Array.from({ length: Math.max(0, end - start) }, (_, i) => start + i));

  const resize = (e: PointerEvent, i: number) => {
    const w0 = widths[i];
    drag(e, (dx) => (resized = { ...resized, [i]: Math.max(40, w0 + dx) }));
  };

  let cx: CanvasRenderingContext2D | undefined;
  // Auto-fit: widest of header + first 200 values, measured off-DOM.
  function fit(i: number) {
    const f = getComputedStyle(el);
    cx ??= document.createElement('canvas').getContext('2d')!;
    cx.font = `600 ${f.fontSize} ${f.fontFamily}`;
    let w = cx.measureText(rs.columns[i]).width;
    cx.font = `${f.fontSize} ${f.fontFamily}`;
    for (let r = 0, n = Math.min(200, rs.rows.length); r < n; r++) w = Math.max(w, cx.measureText(rs.rows[r][i] ?? 'NULL').width);
    resized = { ...resized, [i]: Math.min(800, Math.max(40, Math.ceil(w) + 20)) };
  }

  // Selection = 4 numbers in display coordinates (anchor r0,c0; focus r1,c1). No per-cell state.
  const nr = $derived(rs.rows.length), nc = $derived(rs.columns.length);
  const b = $derived(sel && { r0: Math.min(sel.r0, sel.r1), r1: Math.max(sel.r0, sel.r1), c0: Math.min(sel.c0, sel.c1), c1: Math.max(sel.c0, sel.c1) });
  const inSel = (r: number, c: number) => !!b && r >= b.r0 && r <= b.r1 && c >= b.c0 && c <= b.c1;
  const cellSel = (r: number, c: number, ext: boolean) => { if (nr) sel = ext && sel ? { ...sel, r1: r, c1: c } : { r0: r, c0: c, r1: r, c1: c }; };
  const rowSel = (r: number, ext: boolean) => { if (nr) sel = ext && sel ? { ...sel, r1: r, c0: 0, c1: nc - 1 } : { r0: r, c0: 0, r1: r, c1: nc - 1 }; };
  const colSel = (c: number, ext: boolean) => { if (nr) sel = ext && sel ? { ...sel, r0: 0, r1: nr - 1, c1: c } : { r0: 0, c0: c, r1: nr - 1, c1: c }; };
  const allSel = () => { if (nr) sel = { r0: 0, c0: 0, r1: nr - 1, c1: nc - 1 }; };

  // Sorting: one index array (display row -> data row); the data is never reordered.
  let sort = $state.raw<{ c: number; dir: 1 | -1; ord: Uint32Array } | null>(null);
  const ord = $derived(sort?.ord);
  const val = (r: number, c: number) => rs.rows[ord ? ord[r] : r][c];

  function sortBy(c: number, dir: 1 | -1) {
    const rows = rs.rows, n = rows.length;
    let k: ArrayLike<number | string> = new Float64Array(n); // numeric-aware: all values numeric -> compare as numbers; NULL sorts lowest
    for (let i = 0; i < n; i++) {
      const v = rows[i][c];
      if (v === null) (k as Float64Array)[i] = -Infinity;
      else if (NUM.test(v)) (k as Float64Array)[i] = +v;
      else { k = rows.map((r) => (r[c] ?? '').toLowerCase()); break; }
    }
    const o = new Uint32Array(n);
    for (let i = 0; i < n; i++) o[i] = i;
    o.sort((x, y) => (k[x] < k[y] ? -dir : k[x] > k[y] ? dir : x - y));
    sort = { c, dir, ord: o };
    sel = null;
  }
  const clearSort = () => { sort = null; sel = null; };

  // Text builders (display order).
  const clean = (v: string | null) => (v ?? '').replace(/[\t\r\n]+/g, ' ');
  function tsv(r0: number, r1: number, c0: number, c1: number, headers: boolean) {
    const out: string[] = headers ? [rs.columns.slice(c0, c1 + 1).map(clean).join('\t')] : [];
    for (let r = r0; r <= r1; r++) {
      const row = rs.rows[ord ? ord[r] : r], cells: string[] = [];
      for (let c = c0; c <= c1; c++) cells.push(clean(row[c]));
      out.push(cells.join('\t'));
    }
    return out.join('\n');
  }
  const csvq = (v: string | null) => (v === null ? '' : /[",\r\n]/.test(v) ? `"${v.replaceAll('"', '""')}"` : v); // RFC 4180
  const csv = () => [rs.columns.map(csvq).join(','), ...Array.from({ length: nr }, (_, r) => rs.rows[ord ? ord[r] : r].map(csvq).join(','))].join('\r\n');
  const json = () => JSON.stringify(Array.from({ length: nr }, (_, r) => Object.fromEntries(rs.columns.map((h, c) => [h, val(r, c)]))), null, 2);
  const lit = (v: string | null) => (v === null ? 'NULL' : `N'${v.replaceAll("'", "''")}'`);
  const inserts = (r0: number, r1: number) =>
    Array.from({ length: r1 - r0 + 1 }, (_, i) => `INSERT INTO [TableName] (${rs.columns.map(qid).join(', ')}) VALUES (${rs.rows[ord ? ord[r0 + i] : r0 + i].map(lit).join(', ')});`).join('\n');
  const colValues = (c: number) => Array.from({ length: nr }, (_, r) => val(r, c) ?? '').join('\n');
  const copySel = (headers: boolean) => b && copyText(tsv(b.r0, b.r1, b.c0, b.c1, headers), `${b.r1 - b.r0 + 1} × ${b.c1 - b.c0 + 1} cells`);

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  function click(e: MouseEvent) {
    const t = e.target as HTMLElement;
    const x = t.closest('i') ? null : t.closest<HTMLElement>('[data-c],[data-rn],[data-h],[data-all]');
    if (x) {
      const d = x.dataset, r = +(x.parentElement!.dataset.r ?? 0);
      if (d.c !== undefined) cellSel(r, +d.c, e.shiftKey);
      else if (d.rn !== undefined) rowSel(r, e.shiftKey);
      else if (d.h !== undefined) colSel(+d.h, e.shiftKey);
      else allSel();
    }
    el.focus();
  }

  function ctx(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    const t = e.target as HTMLElement;
    const h = t.closest<HTMLElement>('[data-h]'), x = t.closest<HTMLElement>('[data-c],[data-rn]');
    let items: MenuItem[];
    if (h) {
      const c = +h.dataset.h!;
      items = [
        { label: 'Copy column name', icon: 'copy', run: () => copyText(rs.columns[c], 'column name') },
        { label: 'Copy column values', run: () => copyText(colValues(c), 'column values') },
        { label: 'Auto-fit column', run: () => fit(c) },
        SEP,
        { label: 'Sort ascending', run: () => sortBy(c, 1), disabled: !nr },
        { label: 'Sort descending', run: () => sortBy(c, -1), disabled: !nr },
        { label: 'Clear sort', run: clearSort, disabled: !sort },
      ];
    } else if (x && nr) {
      const r = +x.parentElement!.dataset.r!, isCell = x.dataset.c !== undefined, c = isCell ? +x.dataset.c! : 0;
      if (!inSel(r, c)) isCell ? cellSel(r, c, false) : rowSel(r, false);
      const s = b!;
      items = [
        { label: 'Copy', icon: 'copy', hint: 'Cmd+C', run: () => copySel(false) },
        { label: 'Copy with headers', run: () => copySel(true) },
        { label: 'Copy value', run: () => copyText(val(r, c) ?? '', 'value') },
        { label: 'Copy row(s)', run: () => copyText(tsv(s.r0, s.r1, 0, nc - 1, false), 'rows') },
        { label: 'Copy column values', run: () => copyText(colValues(c), 'column values') },
        { label: 'Copy column name', run: () => copyText(rs.columns[c], 'column name') },
        SEP,
        { label: 'Copy all as TSV', run: () => copyText(tsv(0, nr - 1, 0, nc - 1, true), 'all rows as TSV') },
        { label: 'Copy all as CSV', run: () => copyText(csv(), 'all rows as CSV') },
        { label: 'Copy all as JSON', run: () => copyText(json(), 'all rows as JSON') },
        SEP,
        { label: 'Copy as INSERT', run: () => copyText(inserts(s.r0, s.r1), 'INSERT statements') },
      ];
    } else return;
    menu = { x: e.clientX, y: e.clientY, items };
    el.focus();
  }

  function key(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === 'a') { e.preventDefault(); allSel(); return; }
    if (!sel) return;
    if (mod && e.key === 'c') { e.preventDefault(); copySel(false); return; }
    const d: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    const m = d[e.key];
    if (!m) return;
    e.preventDefault();
    const r = Math.min(nr - 1, Math.max(0, sel.r1 + m[0])), c = Math.min(nc - 1, Math.max(0, sel.c1 + m[1]));
    sel = e.shiftKey ? { ...sel, r1: r, c1: c } : { r0: r, c0: c, r1: r, c1: c };
    if (r * RH < el.scrollTop) el.scrollTop = r * RH;
    else if ((r + 2) * RH > el.scrollTop + vh) el.scrollTop = (r + 2) * RH - vh;
  }
</script>

<div class="gw">
  <div class="g" data-testid="grid" bind:this={el} bind:clientHeight={vh} onscroll={() => (top = el.scrollTop)} onclick={click} oncontextmenu={ctx} onkeydown={key} role="grid" tabindex="0" style="--cols:{cols};--w:{total}px">
    <div class="row hd">
      <div class="rn" data-all>#</div>
      {#each rs.columns as h, i}
        <div class="h" class:r={num[i]} class:s={!!b && i >= b.c0 && i <= b.c1} data-h={i}><span>{h}{sort?.c === i ? (sort.dir === 1 ? ' ▲' : ' ▼') : ''}</span><i onpointerdown={(e) => resize(e, i)} ondblclick={() => fit(i)} role="separator" aria-orientation="vertical" data-testid="col-resizer"></i></div>
      {/each}
    </div>
    <div class="body" style="height:{rs.rows.length * RH}px">
      <div class="win" style="top:{start * RH}px">
        {#each range as r (r)}
          {@const row = rs.rows[ord ? ord[r] : r]}
          <div class="row" class:z={r % 2 === 1} data-r={r}>
            <div class="rn" class:s={!!b && r >= b.r0 && r <= b.r1} data-rn>{r + 1}</div>
            {#each rs.columns as _, c}
              {@const v = row[c]}
              <div class="c" class:s={inSel(r, c)} class:f={sel?.r1 === r && sel.c1 === c} class:nl={v === null} class:r={num[c]} data-c={c}>{v ?? 'NULL'}</div>
            {/each}
          </div>
        {/each}
      </div>
    </div>
  </div>
  {#if b}<div class="cnt" data-testid="grid-selection-count">{b.r1 - b.r0 + 1} × {b.c1 - b.c0 + 1} selected</div>{/if}
</div>
{#if menu}<ContextMenu {...menu} onclose={() => (menu = null)} />{/if}

<style>
  .gw { position: relative; height: 100%; }
  .cnt { position: absolute; right: 16px; bottom: 8px; padding: 1px 8px; border-radius: 4px; font-size: 11.5px; background: var(--bg2); border: 1px solid var(--bd2); color: var(--dim); pointer-events: none; z-index: 5; }
  .g { height: 100%; overflow: auto; position: relative; outline: none; font-size: 12.5px; background: var(--bg); }
  .row { display: grid; grid-template-columns: var(--cols); width: var(--w); height: 24px; line-height: 23px; }
  .row.z { background: var(--zebra); }
  .hd { overflow: clip; position: sticky; top: 0; z-index: 3; background: var(--bg2); }
  .body { position: relative; width: var(--w); }
  .win { position: absolute; left: 0; }
  .rn { position: sticky; left: 0; z-index: 2; background: var(--bg2); color: var(--dim); font-size: 11px; text-align: right; padding: 0 8px; border-right: 1px solid var(--bd); border-bottom: 1px solid var(--bd); font-variant-numeric: tabular-nums; }
  .h { position: relative; padding: 0 8px; font-weight: 600; border-right: 1px solid var(--bd); border-bottom: 1px solid var(--bd2); }
  .h span { display: block; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .h i { position: absolute; right: -4px; top: 0; width: 8px; height: 100%; cursor: col-resize; z-index: 1; touch-action: none; }
  .h i::after { content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 2px; }
  .h i:hover::after { background: var(--accent); }
  .c { padding: 0 8px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; border-right: 1px solid var(--bd); border-bottom: 1px solid var(--bd); }
  .r { text-align: right; font-variant-numeric: tabular-nums; }
  .nl { color: var(--dim); font-style: italic; opacity: .7; }
  .c.s { background: var(--sel); }
  .c.f { outline: 2px solid var(--accent); outline-offset: -2px; }
  .h.s, .rn.s { background: var(--sel); }
</style>
