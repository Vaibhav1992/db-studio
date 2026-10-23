<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Conn, type Env } from './api';
  import { ENVS, loadConns, clearDatabases } from './store.svelte';

  let { conn, onclose }: { conn: Conn | null; onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  const init = conn; // dialog is re-created per open
  let f = $state<Conn>(init ? { ...init } : { id: '', name: '', server: '', port: 1433, database: null, auth: 'sql', user: null, tenant: null, env: 'dev', read_only: false, trust_cert: false });
  let pw = $state('');
  let out = $state('');
  let bad = $state(false);
  let busy = $state(false);
  let d: HTMLDialogElement;

  onMount(() => d.showModal());

  function envChange() {
    if (init?.id) return;
    f.trust_cert = f.env === 'local';
    f.read_only = f.env === 'prod';
  }

  const payload = () => ({ ...f, port: Number(f.port) || 1433, user: f.auth === 'sql' ? f.user || null : null, tenant: f.auth === 'azure_cli' ? f.tenant || null : null, database: f.database || null });
  const password = () => (f.auth === 'sql' && pw ? pw : null);

  async function test() {
    busy = true; out = 'Testing...'; bad = false;
    try { out = await api.testConnection(payload(), password()); } catch (e) { out = String(e); bad = true; }
    busy = false;
  }

  async function save() {
    if (!f.name.trim() || !f.server.trim()) { out = 'Name and server are required.'; bad = true; return; }
    busy = true;
    try {
      await api.saveConnection(payload(), password());
      clearDatabases();
      await loadConns();
      onclose();
    } catch (e) { out = String(e); bad = true; busy = false; }
  }
</script>

<dialog bind:this={d} onclose={onclose}>
  <form class="f" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <b>{init?.id ? 'Edit connection' : 'New connection'}</b>
    <label>Name<input bind:value={f.name} /></label>
    <label>Environment
      <select bind:value={f.env} onchange={envChange}>{#each ENVS as e}<option value={e as Env}>{e}</option>{/each}</select>
    </label>
    <label>Server<input bind:value={f.server} placeholder="host.database.windows.net" /></label>
    <label>Port<input type="number" bind:value={f.port} /></label>
    <label>Authentication
      <select bind:value={f.auth}>
        <option value="sql">SQL Login</option>
        <option value="azure_cli">Microsoft Entra ID (Azure CLI)</option>
      </select>
    </label>
    {#if f.auth === 'sql'}
      <label>User<input bind:value={f.user} autocomplete="off" /></label>
      <label>Password<input type="password" bind:value={pw} autocomplete="off" placeholder={init?.id ? 'leave empty to keep saved password' : ''} /></label>
    {:else}
      <span></span><span class="dim">Uses your `az login` session</span>
      <label>Tenant id<input bind:value={f.tenant} placeholder="optional" /></label>
    {/if}
    <label>Database<input bind:value={f.database} placeholder="optional" /></label>
    <span></span><label class="chk"><input type="checkbox" bind:checked={f.trust_cert} />Trust server certificate</label>
    <span></span><label class="chk"><input type="checkbox" bind:checked={f.read_only} />Read-only</label>
    {#if out}<div class="out" class:err={bad}>{out}</div>{/if}
    <div class="btns">
      <button type="button" onclick={test} disabled={busy}>Test</button>
      <span class="grow"></span>
      <button class="primary" disabled={busy}>Save</button>
      <button type="button" onclick={() => d.close()}>Cancel</button>
    </div>
  </form>
</dialog>

<style>
  dialog { width: 460px; }
  .f { display: grid; grid-template-columns: 130px 1fr; gap: 6px 8px; align-items: center; }
  .f > b, .out, .btns { grid-column: 1 / -1; }
  .f > label:not(.chk) { display: contents; }
  .chk { display: flex; align-items: center; }
  .out { user-select: text; -webkit-user-select: text; word-break: break-word; }
  .btns { display: flex; gap: 6px; margin-top: 4px; }
  .grow { flex: 1; }
</style>
