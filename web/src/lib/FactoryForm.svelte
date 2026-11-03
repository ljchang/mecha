<script>
  import { apiFetch as fetch } from './api.js';
  import { nameFrom } from './factory.js';
  // A form's editor: what strangers fill in, drafted beside what is served
  // and published by `type push` (`mecha_core::form_draft`). Saving writes
  // the draft only; Publish is the one act that changes the live form, and
  // it asks first. A booking page's hours are not edited here — they stay
  // the hours editor's, and publishing carries the live ones over.
  let { id, onpublished = () => {} } = $props();

  let st = $state(null); // the GET /api/factory/form body
  let edit = $state(null); // the structured draft being edited
  let raw = $state('');
  let mode = $state('fields'); // 'fields' | 'toml'
  let error = $state(null);
  let busy = $state(false);
  let preview = $state(null); // { html }
  let confirming = $state(false);
  let toast = $state(null);
  let toastHazard = $state(false);

  function adopt(body) {
    st = body;
    edit = body.form ? structuredClone(body.form) : null;
    raw = body.text;
    preview = null;
    error = null;
  }

  async function load() {
    try {
      const res = await fetch(`/api/factory/form?id=${encodeURIComponent(id)}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      adopt(await res.json());
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  $effect(() => {
    if (id) load();
  });

  // The TOML box is what is edited whenever it is the one shown — in TOML
  // mode, and also in fields mode when the field editor cannot show this
  // form — so its typing always gets a Save.
  const tomlShown = $derived(mode === 'toml' || !edit);
  const dirty = $derived(
    st ? (tomlShown ? raw !== st.text : JSON.stringify(edit) !== JSON.stringify(st.form)) : false,
  );

  function say(text, hazard = false) {
    toast = text;
    toastHazard = hazard;
    setTimeout(() => (toast = null), hazard ? 9000 : 5000);
  }

  async function post(path, body) {
    const res = await fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    const text = await res.text();
    if (!res.ok) throw new Error(text.trim());
    return JSON.parse(text);
  }

  async function saveDraft() {
    busy = true;
    try {
      const body = tomlShown ? { id, base: st.digest, raw } : { id, base: st.digest, form: $state.snapshot(edit) };
      adopt(await post('/api/factory/form/save', body));
      say('Draft saved. Nothing changes for strangers until you publish.');
    } catch (e) {
      say(String(e?.message ?? e), true);
    } finally {
      busy = false;
    }
  }

  async function showPreview() {
    busy = true;
    try {
      preview = await post('/api/factory/form/render', { id });
    } catch (e) {
      say(String(e?.message ?? e), true);
    } finally {
      busy = false;
    }
  }

  async function publish() {
    busy = true;
    try {
      await post('/api/factory/form/publish', { id, base: st.digest });
      confirming = false;
      say('Published. Strangers see this version now.');
      await load();
      onpublished();
    } catch (e) {
      confirming = false;
      say(String(e?.message ?? e), true);
    } finally {
      busy = false;
    }
  }

  async function discard() {
    busy = true;
    try {
      await post('/api/factory/form/discard', { id, base: st.digest });
      say('Draft discarded.');
      await load();
    } catch (e) {
      say(String(e?.message ?? e), true);
    } finally {
      busy = false;
    }
  }

  function revert() {
    edit = st.form ? structuredClone(st.form) : null;
    raw = st.text;
  }

  // ── the structured editor ──────────────────────────────────────────────
  const KIND_LABELS = {
    text: 'Short text',
    long_text: 'Long text',
    email: 'Email',
    url: 'Link',
    date: 'Date',
    integer: 'Number',
    select: 'One choice',
    multi_select: 'Several choices',
    bool: 'Yes / no',
    file: 'File',
  };
  const isChoice = (k) => k === 'select' || k === 'multi_select';

  function addField() {
    const taken = edit.fields.map((f) => f.name);
    edit.fields.push({ name: nameFrom('New question', taken), label: 'New question', help: null, kind: 'text', required: false, options: null, max_length: null });
  }
  function move(i, by) {
    const j = i + by;
    if (j < 0 || j >= edit.fields.length) return;
    const [f] = edit.fields.splice(i, 1);
    edit.fields.splice(j, 0, f);
  }
  function setKind(f, kind) {
    f.kind = kind;
    if (isChoice(kind) && !f.options?.length) f.options = [{ value: 'option_1', label: 'Option 1' }];
    if (!isChoice(kind)) f.options = null;
  }
  function addOption(f) {
    const taken = (f.options ?? []).map((o) => o.value);
    const label = `Option ${(f.options?.length ?? 0) + 1}`;
    f.options = [...(f.options ?? []), { value: nameFrom(label, taken), label }];
  }
  // An option's value is what every request records and Review shows, so
  // it follows the label until the field is saved with it; after that it is
  // a value requests already carry, and stays.
  const savedValues = $derived(
    new Map((st?.form?.fields ?? []).map((f) => [f.name, new Set((f.options ?? []).map((o) => o.value))])),
  );
  function relabelOption(f, o, label) {
    o.label = label;
    if (!savedValues.get(f.name)?.has(o.value)) {
      o.value = nameFrom(label, (f.options ?? []).filter((x) => x !== o).map((x) => x.value));
    }
  }
  // A new field's name follows its label until it is saved; a saved one's
  // name is its key in every request already received, so it stays.
  const savedNames = $derived(new Set((st?.form?.fields ?? []).map((f) => f.name)));
  function relabel(f, label) {
    f.label = label;
    if (!savedNames.has(f.name)) {
      f.name = nameFrom(label, edit.fields.filter((x) => x !== f).map((x) => x.name));
    }
  }
</script>

<div class="formed">
  <div class="section">
    The form
    {#if st && !st.published}<span class="chip warn">not published yet</span>
    {:else if st?.draft}<span class="chip warn">unpublished changes</span>{/if}
  </div>
  {#if error}<div class="warnline">{error}</div>{/if}
  {#if !st && !error}
    <div class="note">reading the form…</div>
  {:else if st}
    <div class="modes">
      <button class="pill" class:on={mode === 'fields'} onclick={() => (mode = 'fields')} disabled={dirty && mode === 'toml'}>Fields</button>
      <button class="pill" class:on={mode === 'toml'} onclick={() => (mode = 'toml')} disabled={dirty && mode === 'fields'}>Edit as TOML</button>
      {#if dirty}<span class="note">save or revert to switch</span>{/if}
    </div>

    {#if mode === 'toml' || !edit}
      {#if st.form_error && mode === 'fields' && !edit}<div class="note">This form uses a shape the field editor does not show ({st.form_error}); edit it as TOML.</div>{/if}
      <textarea class="toml" spellcheck="false" bind:value={raw} rows="24"></textarea>
    {:else}
      <div class="card fields">
        <label class="field col">
          <span class="flabel">Title</span>
          <input class="in" bind:value={edit.title} />
        </label>
        <label class="field col">
          <span class="flabel">Description <span class="sub">shown above the questions</span></span>
          <textarea class="in" rows="2" value={edit.description ?? ''} oninput={(e) => (edit.description = e.currentTarget.value || null)}></textarea>
        </label>
      </div>

      <div class="section">Questions</div>
      {#each edit.fields as f, i (i)}
        <div class="card q">
          <div class="qtop">
            <input class="in grow" value={f.label} oninput={(e) => relabel(f, e.currentTarget.value)} aria-label="question" />
            <button class="x" aria-label="move up" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
            <button class="x" aria-label="move down" disabled={i === edit.fields.length - 1} onclick={() => move(i, 1)}>↓</button>
            <button class="x" aria-label="remove question" onclick={() => edit.fields.splice(i, 1)}>×</button>
          </div>
          <div class="qrow">
            <select class="in" value={f.kind} onchange={(e) => setKind(f, e.currentTarget.value)} disabled={f.kind === 'file'}>
              {#each [...st.kinds, ...(f.kind === 'file' ? ['file'] : [])] as k}<option value={k}>{KIND_LABELS[k] ?? k}</option>{/each}
            </select>
            <label class="check"><input type="checkbox" bind:checked={f.required} /> required</label>
            <span class="name">{f.name}</span>
          </div>
          <input class="in" placeholder="help text (optional)" value={f.help ?? ''} oninput={(e) => (f.help = e.currentTarget.value || null)} />
          {#if isChoice(f.kind)}
            <div class="opts">
              {#each f.options ?? [] as o, oi (oi)}
                <div class="opt">
                  <input class="in grow" value={o.label} oninput={(e) => relabelOption(f, o, e.currentTarget.value)} aria-label="option" />
                  <button class="x" aria-label="remove option" disabled={f.options.length === 1} onclick={() => f.options.splice(oi, 1)}>×</button>
                </div>
              {/each}
              <button class="link" onclick={() => addOption(f)}>+ add an option</button>
            </div>
          {/if}
        </div>
      {/each}
      <button class="abtn" onclick={addField}>+ Add a question</button>

      <div class="section">After they submit</div>
      <div class="card fields">
        <label class="field col">
          <span class="flabel">Heading</span>
          <input class="in" value={edit.confirmation_heading ?? ''} oninput={(e) => (edit.confirmation_heading = e.currentTarget.value || null)} />
        </label>
        <label class="field col">
          <span class="flabel">Message</span>
          <textarea class="in" rows="3" value={edit.confirmation_body ?? ''} oninput={(e) => (edit.confirmation_body = e.currentTarget.value || null)}></textarea>
        </label>
      </div>
    {/if}

    {#if st.kind === 'booking'}
      <div class="note">This is a booking page: its hours are set above and are not edited here. Publishing keeps the live hours.</div>
    {/if}

    <div class="bar">
      {#if dirty}
        <button class="abtn" disabled={busy} onclick={revert}>Revert</button>
        <button class="abtn primary" disabled={busy} onclick={saveDraft}>Save draft</button>
      {:else}
        <button class="abtn" disabled={busy} onclick={showPreview}>Preview</button>
        {#if st.draft}
          <button class="abtn" disabled={busy} onclick={discard}>Discard draft</button>
          <button class="abtn primary" disabled={busy} onclick={() => (confirming = true)}>Publish…</button>
        {/if}
      {/if}
    </div>

    {#if preview}
      <div class="section">Preview <span class="sub">as strangers will see it{st.draft ? ', with your draft' : ''}</span></div>
      <!-- No scripts, no network, no forms: the page itself, looked at. -->
      <div class="note">The page as it looks, not as it behaves: questions that appear only after an earlier answer are all shown here.</div>
      <iframe class="preview" title="form preview" sandbox="" srcdoc={preview.html}></iframe>
    {/if}
  {/if}

  {#if confirming}
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">{st.published ? 'Replace the live form with this draft?' : 'Publish this form?'}</div>
      <div class="note">Strangers see it as soon as it is published{st.published ? ', and requests already received keep their answers' : ', at a public link'}. Preview it first if you have not.</div>
      <div class="btnrow">
        <button class="abtn" disabled={busy} onclick={() => (confirming = false)}>Back</button>
        <button class="abtn primary" disabled={busy} onclick={publish}>{busy ? 'Publishing…' : 'Publish'}</button>
      </div>
    </div>
  {/if}

  {#if toast}<div class="toast" class:hazardtoast={toastHazard}>{toast}</div>{/if}
</div>

<style>
  .formed { display: flex; flex-direction: column; gap: 10px; }
  .section { font-family: var(--mono); font-size: 11px; letter-spacing: 0.04em; text-transform: uppercase; color: var(--accent-500); margin-top: 8px; display: flex; gap: 8px; align-items: center; }
  .sub { text-transform: none; letter-spacing: 0; color: var(--text-muted); font-family: var(--mono); font-size: 11px; }
  .chip.warn { color: var(--hazard); text-transform: none; letter-spacing: 0; }
  .note { font-size: 11px; line-height: 1.45; color: var(--text-muted); }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; overflow-wrap: anywhere; }
  .modes { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .pill { min-height: 36px; padding: 0 12px; border-radius: var(--radius-chip); border: 1px solid var(--accent-900); background: none; color: var(--text-muted); font-family: var(--mono); font-size: 12px; cursor: pointer; }
  .pill.on { background: var(--accent-900); color: var(--text); border-color: var(--accent-500); }
  .pill:disabled { opacity: 0.5; }
  .toml { width: 100%; box-sizing: border-box; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 12px; line-height: 1.5; padding: 12px; resize: vertical; }
  .fields { padding: 6px 14px; display: flex; flex-direction: column; }
  .field.col { display: flex; flex-direction: column; gap: 6px; padding: 10px 0; border-bottom: 1px solid var(--accent-900); }
  .field.col:last-child { border-bottom: none; }
  .flabel { font-size: 13px; }
  .in { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font: inherit; font-size: 13px; padding: 8px 10px; box-sizing: border-box; color-scheme: dark; }
  textarea.in { resize: vertical; }
  .grow { flex: 1; min-width: 0; }
  .q { padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
  .qtop, .qrow, .opt { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .check { font-size: 12px; color: var(--text-muted); display: inline-flex; gap: 6px; align-items: center; }
  .name { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .opts { display: flex; flex-direction: column; gap: 6px; padding-left: 12px; border-left: 2px solid var(--accent-900); }
  .x { background: none; border: none; color: var(--text-muted); font: inherit; font-size: 14px; min-width: 32px; min-height: 32px; cursor: pointer; }
  .x:disabled { opacity: 0.3; }
  .link { background: none; border: none; padding: 0; color: var(--accent-400); font: inherit; font-size: 12px; cursor: pointer; text-align: left; }
  .bar { display: flex; gap: 8px; justify-content: flex-end; flex-wrap: wrap; }
  .abtn { min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .preview { width: 100%; height: 640px; border: 1px solid var(--accent-900); border-radius: var(--radius); background: #fff; }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  .sheet { position: fixed; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; line-height: 1.4; }
  .toast { position: fixed; bottom: 90px; left: 50%; transform: translateX(-50%); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 10px 16px; font-size: 13px; max-width: 90%; z-index: 7; text-align: center; }
  .toast.hazardtoast { border-color: var(--hazard); color: var(--hazard); }
</style>
