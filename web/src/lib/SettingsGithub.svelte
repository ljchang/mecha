<script>
  import { apiFetch as fetch } from './api.js';
  import {
    visibleRepos,
    repoChange,
    ownerChange,
    noneStep,
    ownerLine,
    filterChange,
    packageChange,
    ownerCount,
    pending,
    packageLine,
    hasPackages,
    when,
    storeLine,
    fileNotice,
  } from './github-picker.js';

  // The picker (GITHUB-DASHBOARD-DESIGN.md step 6, ruling G3): which
  // repositories `mecha github collect` watches, and the package names it
  // counts. It lists the inventory the collector last wrote — the page never
  // asks GitHub, so a slow API is an old list with its age shown — and
  // writes ~/.mecha/github/repos.toml and packages.toml on a click, each save
  // carrying the digest of the file this page read, so a hand edit made
  // meanwhile is refused rather than written over. What a row shows as
  // selected is the server's answer, after every save.

  let view = $state(undefined);
  let error = $state(null);
  let busy = $state(false);
  let query = $state('');
  let forks = $state(false);
  let archived = $state(false);
  let recent = $state(false);
  // Which repository's package editor is open, and its draft per registry.
  let editing = $state(null);
  let draft = $state({});
  // An owner's "select none" waiting on its in-page confirm: `{ login, text }`.
  let confirming = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/github/picker');
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      view = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  // One save at a time: each carries the digest the last answer gave, so a
  // second click sent before the first's answer would be refused as stale.
  async function post(path, body) {
    if (busy) return;
    busy = true;
    try {
      const res =
        path === 'packages'
          ? await fetch('/api/github/picker/packages', {
              method: 'POST',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify(body),
            })
          : await fetch('/api/github/picker/selection', {
              method: 'POST',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify(body),
            });
      if (!res.ok) {
        const why = (await res.text()).trim() || `HTTP ${res.status}`;
        // Re-read whatever the refusal: a stale page then clicks against the
        // file as it is now, and a box the click ticked goes back to what
        // the file says. The refusal stays on screen to say why.
        await load();
        error = why;
        return false;
      }
      view = await res.json();
      error = null;
      return true;
    } catch (e) {
      await load();
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  const selection = (change) => post('selection', { base: view.selection.digest, changes: [change] });

  // "Select none" removes the owner's lines, hand-picked inclusions too:
  // where it would remove one, the first press asks and the second sends.
  async function selectNone(owner, confirmed = false) {
    const step = noneStep(owner, confirmed);
    if (step.confirm) {
      confirming = { login: owner.login, text: step.confirm };
      return;
    }
    confirming = null;
    await selection(step.change);
  }

  function toggleRepo(repo, want) {
    const change = repoChange(repo, want);
    if (change) selection(change);
  }

  function openEditor(repo) {
    editing = repo.full_name;
    draft = Object.fromEntries(
      repo.packages.map((p) => [
        p.registry,
        {
          to: p.state === 'owner' ? 'name' : p.state === 'owner_none' ? 'none' : 'detected',
          name: p.state === 'owner' ? p.name : (p.detected ?? ''),
        },
      ]),
    );
  }

  async function savePackages(repo) {
    const changes = repo.packages
      .map((p) => {
        const d = draft[p.registry];
        const was = p.state === 'owner' ? 'name' : p.state === 'owner_none' ? 'none' : 'detected';
        if (d.to === was && (d.to !== 'name' || d.name.trim() === p.name)) return null;
        return packageChange(repo.full_name, p.registry, d.to === 'name' ? d.name : d.to);
      })
      .filter(Boolean);
    if (!changes.length) {
      editing = null;
      return;
    }
    if (await post('packages', { base: view.packages.digest, changes })) editing = null;
  }

  const canSelect = $derived(view?.selection?.state !== 'invalid' && view?.store?.state === 'read');
  const canMap = $derived(view?.packages?.state !== 'invalid' && view?.store?.state === 'read');
  const store = $derived(storeLine(view?.store));
  const notices = $derived(
    [fileNotice(view?.selection, 'repos.toml'), fileNotice(view?.packages, 'packages.toml')].filter(Boolean),
  );
</script>

<p class="hint">
  Which repositories the GitHub board watches, from your account and your organizations, and which package names its
  download counts are read under. The list is what the collector last saw; nothing here asks GitHub.
</p>

{#if error}
  <div class="card notice">{error}</div>
{/if}

{#if view === undefined && !error}
  <p class="hint">reading…</p>
{:else if view}
  <div class="card status">
    <div class:bad={store.bad}>{store.text}</div>
    <div class="sub">
      Changes take effect on the next collect{#if view.schedule}, {view.schedule}{/if} (<code
        >mecha-github-collect.timer</code
      >, as shipped), or when <code>mecha github collect</code> is run by hand.
    </div>
  </div>
  {#each notices as n}
    <div class="card notice">{n}</div>
  {/each}

  <div class="filters">
    <input class="search" type="search" placeholder="search repositories" bind:value={query} />
    <label><input type="checkbox" bind:checked={forks} /> forks</label>
    <label><input type="checkbox" bind:checked={archived} /> archived</label>
    <label><input type="checkbox" bind:checked={recent} /> pushed in the last year</label>
  </div>

  {#if !view.owners.length && !store.bad}
    <p class="hint">No owners yet: the collector has listed nothing, and repos.toml names no one.</p>
  {/if}

  {#each view.owners as owner (owner.login)}
    {@const rows = visibleRepos(owner.repos, { forks, archived, query, recent })}
    <section class="card owner">
      <div class="ohead">
        <label class="all">
          <input
            type="checkbox"
            disabled={busy || !canSelect}
            checked={owner.line?.include === 'all'}
            onchange={(e) => selection(ownerChange(owner.login, e.currentTarget.checked))}
          />
          <span class="login">{owner.login}</span>
        </label>
        <span class="count">{ownerCount(owner)}</span>
      </div>
      <div class="osub">
        {ownerLine(owner, view.selection.state)}
        {#if canSelect && (owner.repos.some((r) => r.included) || owner.unseen.some((l) => l.include))}
          <button class="link" disabled={busy} onclick={() => selectNone(owner)}>select none</button>
        {/if}
        {#if owner.line}
          <span class="ofilters">
            <label
              ><input
                type="checkbox"
                disabled={busy || !canSelect}
                checked={owner.line.forks}
                onchange={(e) => selection(filterChange(owner.login, e.currentTarget.checked, owner.line.archived))}
              /> take forks</label
            >
            <label
              ><input
                type="checkbox"
                disabled={busy || !canSelect}
                checked={owner.line.archived}
                onchange={(e) => selection(filterChange(owner.login, owner.line.forks, e.currentTarget.checked))}
              /> take archived</label
            >
          </span>
        {/if}
      </div>
      {#if confirming?.login === owner.login}
        <div class="confirm" role="alert">
          <span>{confirming.text}</span>
          <span class="eact">
            <button disabled={busy} onclick={() => selectNone(owner, true)}>Remove them</button>
            <button class="link" disabled={busy} onclick={() => (confirming = null)}>keep them</button>
          </span>
        </div>
      {/if}

      {#each rows as repo (repo.full_name)}
        <div class="repo">
          <label class="pick">
            <input
              type="checkbox"
              disabled={busy || !canSelect || repo.included === null}
              checked={repo.included === true}
              onchange={(e) => toggleRepo(repo, e.currentTarget.checked)}
            />
            <span class="rname">{repo.name}</span>
          </label>
          <span class="tags">
            {#if repo.fork}<span class="tag">fork</span>{/if}
            {#if repo.archived}<span class="tag">archived</span>{/if}
            {#if repo.private}<span class="tag">private</span>{/if}
            {#if !repo.listed}<span class="tag">named</span>{/if}
            {#if pending(repo)}<span class="tag next">{repo.included ? 'added' : 'dropped'} next collect</span>{/if}
          </span>
          <div class="rmeta">
            pushed {repo.pushed_at ? when(repo.pushed_at).slice(0, 10) : '—'}
            {#if hasPackages(repo) || editing === repo.full_name}
              ·
              {#each repo.packages.filter((p) => p.state !== 'none_found' && p.state !== 'not_scanned') as p, i}
                {i ? ', ' : ''}<span class:mine={packageLine(p).owner}>{p.registry} {packageLine(p).text}</span>
              {/each}
            {:else if repo.packages.every((p) => p.state === 'not_scanned')}
              · packages not scanned yet
            {:else}
              · no package found
            {/if}
            {#if canMap && editing !== repo.full_name}
              <button class="link" disabled={busy} onclick={() => openEditor(repo)}>edit</button>
            {/if}
          </div>
          {#if repo.private && repo.packages.some((p) => p.state === 'detected')}
            <div class="rnote">A name detected in a private repository is never sent to a registry; set it here to count it.</div>
          {/if}
          {#if editing === repo.full_name}
            <div class="editor">
              {#each repo.packages as p (p.registry)}
                <div class="reg">
                  <span class="rlabel">{p.registry}</span>
                  <select class="sel" bind:value={draft[p.registry].to}>
                    <option value="detected"
                      >as detected ({p.detected ?? (p.state === 'not_scanned' ? 'not scanned yet' : 'none found')})</option
                    >
                    <option value="name">this name</option>
                    <option value="none">publishes nothing</option>
                  </select>
                  {#if draft[p.registry].to === 'name'}
                    <input class="pname" bind:value={draft[p.registry].name} placeholder="package name" />
                  {/if}
                </div>
              {/each}
              <div class="eact">
                <button disabled={busy} onclick={() => savePackages(repo)}>Save</button>
                <button class="link" disabled={busy} onclick={() => (editing = null)}>cancel</button>
              </div>
            </div>
          {/if}
        </div>
      {/each}
      {#if rows.length < owner.repos.length}
        <div class="hidden">{owner.repos.length - rows.length} hidden by the filters</div>
      {/if}
      {#each owner.unseen as line (line.name)}
        <div class="repo unseen">
          <span class="rname">{line.name}</span>
          <span class="tags"><span class="tag">{line.include ? 'included' : 'excluded'} by name · not seen yet</span></span>
          {#if canSelect}
            <button class="link" disabled={busy} onclick={() => selection({ op: 'repo', name: line.name, include: null })}
              >remove the line</button
            >
          {/if}
        </div>
      {/each}
    </section>
  {/each}
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
  .status {
    padding: 10px 12px;
    margin-bottom: 12px;
    font-size: 12.5px;
    line-height: 1.45;
  }
  .bad {
    color: var(--hazard);
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 4px;
  }
  code {
    font-family: var(--mono);
    font-size: 11px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    margin-bottom: 12px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .search {
    flex: 1 1 100%;
    min-height: 36px;
    padding: 0 10px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
  }
  .owner {
    padding: 10px 12px;
    margin-bottom: 12px;
  }
  .ohead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .all {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
  }
  .login {
    font-size: 14px;
    font-weight: 500;
  }
  .count {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
    flex-shrink: 0;
  }
  .osub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.45;
    padding-bottom: 6px;
  }
  .confirm {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 4px 0 8px;
    padding: 8px 10px;
    border-left: 2px solid var(--hazard);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .ofilters {
    display: inline-flex;
    gap: 12px;
    margin-left: 8px;
  }
  .repo {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 8px;
    padding: 8px 0;
    border-top: 1px solid var(--accent-900);
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    min-width: 0;
  }
  .rname {
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .tags {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tag {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    padding: 0 5px;
  }
  .tag.next {
    color: var(--accent-400);
    border-color: var(--accent-700);
  }
  .rmeta {
    flex: 1 1 100%;
    padding-left: 26px;
    font-size: 11.5px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .rnote {
    flex: 1 1 100%;
    padding-left: 26px;
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .mine {
    color: var(--accent-400);
  }
  .link {
    background: none;
    border: none;
    padding: 0 0 0 6px;
    color: var(--accent-400);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
  }
  .editor {
    flex: 1 1 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 0 4px 26px;
  }
  .reg {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .rlabel {
    width: 44px;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .sel,
  .pname {
    min-height: 32px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 12.5px;
    padding: 0 6px;
    min-width: 0;
  }
  .pname {
    flex: 1 1 140px;
  }
  .eact {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .eact button:first-child {
    min-height: 32px;
    padding: 0 14px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
  }
  .hidden {
    padding-top: 6px;
    border-top: 1px solid var(--accent-900);
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .unseen .rname {
    color: var(--text-muted);
  }
</style>
