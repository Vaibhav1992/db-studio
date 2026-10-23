<script lang="ts">
  import { wask } from './store.svelte';
  import Icon from './Icon.svelte';

  let typed = $state('');
  const req = $derived(wask.req);
  const phrase = $derived(req?.phrase ?? (req?.card ? `execute prod ${req.card}` : 'execute prod'));
  const ok = $derived(typed.trim().toLowerCase() === phrase.toLowerCase());

  function done(run: boolean) {
    const r = wask.req;
    if (!r) return;
    // Read ok/phrase before clearing state: both are derived from `typed` and `wask.req`.
    const result = run && ok ? phrase : null; // send the canonical phrase once the typed text matched
    wask.req = null;
    typed = '';
    r.resolve(result);
  }
  const modal = (d: HTMLDialogElement) => d.showModal();
</script>

{#if req}
  <dialog use:modal onclose={() => done(false)} data-testid="confirm-dialog">
    <form onsubmit={(e) => { e.preventDefault(); if (ok) done(true); }}>
      <h3><Icon name="lock" size={18} />{req.title ?? 'Production write'}</h3>
      <p class="reason" data-testid="confirm-reason">{req.reason}</p>
      <div class="meta"><span class="dim">Connection</span><b>{req.conn}</b><span class="dim">Database</span><b>{req.db}</b></div>
      <pre class="mono" data-testid="confirm-sql">{req.sql}</pre>
      <label><span>Type <code>{phrase}</code> to run</span>
        <input bind:value={typed} autocomplete="off" spellcheck="false" data-testid="confirm-input" />
      </label>
      <div class="btns">
        <button type="button" onclick={() => done(false)} data-testid="confirm-cancel">Cancel</button>
        <button class="danger" disabled={!ok} data-testid="confirm-run">{req.action ?? 'Run on production'}</button>
      </div>
    </form>
  </dialog>
{/if}

<style>
  dialog { width: 560px; max-width: 92vw; border-top: 3px solid var(--prod); }
  form { display: flex; flex-direction: column; gap: 10px; }
  h3 { margin: 0; font-size: 15px; display: flex; align-items: center; gap: 8px; color: var(--prod); }
  .reason { margin: 0; user-select: text; -webkit-user-select: text; white-space: pre-wrap; }
  .meta { display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; }
  pre { margin: 0; padding: 8px 10px; background: var(--bg2); border: 1px solid var(--bd); border-radius: 5px; font-size: 12px; max-height: 220px; overflow: auto; user-select: text; -webkit-user-select: text; }
  label { display: flex; flex-direction: column; gap: 4px; }
  code { font-family: var(--mono); background: var(--bg3); padding: 0 4px; border-radius: 3px; }
  input { width: 100%; }
  .btns { display: flex; justify-content: flex-end; gap: 8px; }
</style>
