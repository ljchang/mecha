<script>
  import { apiFetch as fetch } from './api.js';
  import { autolockLine } from './autolock.js';

  // The library lock's one setting the page may change: how long an unlock
  // lasts with no one touching the page (`imagelib::autolock_minutes`). The
  // Personas and Library tabs lock themselves after it, and the server's
  // token lapses on the same span. The password stays a terminal act —
  // typed here, it would ride a request and could land in a log.
  //
  // Saved as `mecha imagelib set-autolock` (the server runs that child), and
  // it applies from the next unlock: an unlock already open keeps the span
  // it was granted.
  const CHOICES = [1, 5, 15, 30, 60, 120, 240];

  let lock = $state(undefined);
  let error = $state(null);
  let saving = $state(false);
  let saved = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/lock');
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      lock = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  // A value set from a terminal that is not one of the choices is still
  // offered, as itself, rather than shown as a choice it is not.
  const options = $derived(
    lock?.idle_minutes != null && !CHOICES.includes(lock.idle_minutes)
      ? [...CHOICES, lock.idle_minutes].sort((a, b) => a - b)
      : CHOICES,
  );

  async function save(minutes) {
    saving = true;
    saved = null;
    try {
      const res = await fetch('/api/settings/lock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ idle_minutes: minutes }),
      });
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      await load();
      saved = minutes;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      saving = false;
    }
  }
</script>

<p class="hint">
  Locked personas and library entries show only after an unlock. An unlock ends when you lock again, when the page is
  reloaded, or after this long with no one using the page.
</p>

{#if error}
  <div class="card notice">{error}</div>
{/if}

{#if lock === undefined && !error}
  <p class="hint">reading…</p>
{:else if lock}
  {#if lock.error}
    <!-- Damaged: no unlock is granted until it is set again. -->
    <div class="card notice">
      The autolock setting could not be read, so nothing unlocks until it is set again: {lock.error}
    </div>
  {/if}
  <div class="card fields">
    <label class="field">
      <span class="flabel">lock after</span>
      <select
        class="pick"
        disabled={saving}
        value={lock.idle_minutes ?? ''}
        onchange={(e) => save(Number(e.currentTarget.value))}
      >
        {#if lock.idle_minutes == null}<option value="" disabled>choose</option>{/if}
        {#each options as m}
          <option value={m}>{autolockLine(m)}{m === lock.default_minutes ? ' (default)' : ''}</option>
        {/each}
      </select>
    </label>
    <div class="sub">
      {#if saved !== null}
        Saved. Applies from the next unlock.
      {:else}
        with no one using the page. Applies from the next unlock.
      {/if}
    </div>
  </div>
  <div class="card fields">
    <div class="field">
      <span class="flabel">password</span>
      <span class="val">{lock.has_password ? 'set' : 'not set: the lock is a show/hide toggle'}</span>
    </div>
    <div class="sub">
      Set or change it in a terminal: <code>mecha imagelib set-lock-password</code>
    </div>
  </div>
{/if}

<style>
  .hint {
    margin: 0 0 14px;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .notice {
    padding: 10px 12px;
    margin-bottom: 12px;
    font-size: 12.5px;
    line-height: 1.45;
    border-left: 2px solid var(--hazard);
  }
  .fields {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    margin-bottom: 12px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .flabel {
    width: 78px;
    flex-shrink: 0;
    color: var(--text-muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .pick {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
    min-height: 36px;
    padding: 0 8px;
  }
  .val {
    font-size: 13px;
  }
  .sub {
    padding-left: 88px;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.45;
  }
  code {
    font-family: var(--mono);
    font-size: 11px;
  }
</style>
