<script lang="ts" module>
  export type MenuItem = { label: string; icon?: string; run: () => void; danger?: boolean; disabled?: boolean; sep?: boolean; hint?: string };
  export const SEP: MenuItem = { label: '', run() {}, sep: true };
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();
  let el: HTMLDivElement;
  const ok = (n: number) => { const x = items[n]; return !!x && !x.sep && !x.disabled; };
  let w = $state(0), h = $state(0), i = $state(-1); // keyboard cursor; set to the first enabled item on mount

  // Grab the item first: onclose() unmounts us and the items prop goes away with the parent state.
  const pick = (n: number) => { if (!ok(n)) return; const it = items[n]; onclose(); it.run(); };
  const step = (d: number) => { let n = i; for (let k = 0; k < items.length; k++) { n = (n + d + items.length) % items.length; if (ok(n)) { i = n; return; } } };

  // Capture phase + stopPropagation: the menu may open over the editor, which must not also see Enter/Esc/arrows.
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
    else if (e.key === 'ArrowDown') step(1);
    else if (e.key === 'ArrowUp') step(-1);
    else if (e.key === 'Enter') pick(i);
    else return;
    e.preventDefault(); e.stopPropagation();
  }

  onMount(() => {
    i = items.findIndex((_, n) => ok(n));
    addEventListener('scroll', onclose, true);
    addEventListener('wheel', onclose, true);
    return () => { removeEventListener('scroll', onclose, true); removeEventListener('wheel', onclose, true); };
  });
</script>

<svelte:window onpointerdown={(e) => el.contains(e.target as Node) || onclose()} onkeydowncapture={key} onblur={onclose} onresize={onclose} />

<div class="m" role="menu" tabindex="-1" bind:this={el} bind:clientWidth={w} bind:clientHeight={h} style="left:{Math.max(4, Math.min(x, innerWidth - w - 4))}px;top:{Math.max(4, Math.min(y, innerHeight - h - 4))}px" data-testid="ctx-menu">
  {#each items as it, n}
    {#if it.sep}<div class="sp" role="separator"></div>
    {:else}
      <button role="menuitem" class:on={i === n} class:danger-i={it.danger} disabled={it.disabled} onmouseenter={() => ok(n) && (i = n)} onclick={() => pick(n)} data-testid="ctx-item" data-label={it.label}>
        <span class="ic">{#if it.icon}<Icon name={it.icon} size={14} />{/if}</span>{it.label}{#if it.hint}<span class="hint">{it.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .m { position: fixed; z-index: 100; min-width: 190px; padding: 4px; background: var(--bg); border: 1px solid var(--bd2); border-radius: 6px; box-shadow: 0 6px 24px rgba(0, 0, 0, .3); display: flex; flex-direction: column; }
  button { width: 100%; height: 26px; border: none; background: none; border-radius: 4px; padding: 0 8px; justify-content: flex-start; transition: none; }
  button.on { background: var(--accent); color: #fff; }
  button.danger-i:not(.on) { color: var(--err); }
  button:disabled { opacity: .4; }
  .sp { height: 1px; margin: 4px 2px; background: var(--bd2); }
  .hint { margin-left: auto; padding-left: 18px; opacity: .6; font-size: 11.5px; }
  .ic { width: 16px; display: inline-flex; flex: none; }
</style>
