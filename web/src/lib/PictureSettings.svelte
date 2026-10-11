<script>
  import { imageSettingsBody, presetBody } from './persona.js';

  // How pictures are sampled: the chat's on Settings → Images, a persona's on
  // its page. Each field offers the server's own lists, so nothing the graph
  // cannot take is offered, and an unset field says what it falls to — the
  // chat's settings on a persona's page (the owner, 2026-10-11), the model's
  // own on the chat's. Applies to every pass, a layered picture's included.
  // `save` takes the route's body; `token` rides for a locked persona.
  let { img, opts = null, busy = false, save, token = null } = $props();

  const under = $derived(img.inherits === 'chat' ? 'chat' : 'default');
  const modelName = (m) => (m === 'turbo' ? 'Turbo' : 'Standard');
  const onOff = (b) => (b ? 'on' : 'off');

  function set(patch) {
    return save(imageSettingsBody(img.set, patch, token));
  }

  function cfgChoices(o) {
    const [min, max, step] = o?.cfg ?? [1, 5, 0.5];
    const out = [];
    for (let v = min; v <= max + 1e-9; v += step) out.push(Math.round(v * 10) / 10);
    return out;
  }
  function stepChoices(o) {
    const [min, max] = o?.steps ?? [4, 60];
    return Array.from({ length: max - min + 1 }, (_, i) => min + i);
  }
</script>

<div class="imgset">
  <div class="locktext">
    Picture settings
    <span class="hint" id="imghint">about {img.estimate_secs} s a picture in one pass{img.model === 'turbo' ? ' with the turbo model: quick, weaker on two people touching' : ''}{img.model === 'turbo' && img.cfg > 1 ? '; turbo is made for guidance 1, and above it each step costs twice' : ''}; changing model loads it on the next picture and unloads the other</span>
    {#if under === 'chat'}
      <span class="hint">A setting left at "chat" follows Settings → Images; a different model from the chat's takes that model's own numbers.</span>
    {/if}
  </div>
  <div class="presets" role="group" aria-label="Picture presets">
    <button type="button" class="linkbtn" disabled={busy} onclick={() => save(presetBody('default', token))}>{under === 'chat' ? "Chat's" : 'Default'}</button>
    <button type="button" class="linkbtn" disabled={busy} onclick={() => save(presetBody('guided', token))}>Guided</button>
    <button type="button" class="linkbtn" disabled={busy} onclick={() => save(presetBody('turbo', token))}>Turbo</button>
  </div>
  <div class="imggrid">
    <label>Model
      <select value={img.set?.model ?? ''} aria-describedby="imghint" disabled={busy} onchange={(e) => set({ model: e.currentTarget.value || null })}>
        <option value="">{under} ({modelName(img.defaults.model)})</option>
        <option value="standard">Standard</option>
        <option value="turbo">Turbo</option>
      </select>
    </label>
    <label>Steps
      <select value={img.set?.steps ?? ''} disabled={busy} onchange={(e) => set({ steps: e.currentTarget.value || null })}>
        <option value="">{under} ({img.defaults.steps})</option>
        {#each stepChoices(opts) as n (n)}<option value={n}>{n}</option>{/each}
      </select>
    </label>
    <label>Guidance (cfg)
      <select value={img.set?.cfg ?? ''} disabled={busy} onchange={(e) => set({ cfg: e.currentTarget.value || null })}>
        <option value="">{under} ({img.defaults.cfg})</option>
        {#each cfgChoices(opts) as v (v)}<option value={v}>{v}</option>{/each}
      </select>
    </label>
    <label>Sampler
      <select value={img.set?.sampler ?? ''} disabled={busy} onchange={(e) => set({ sampler: e.currentTarget.value || null })}>
        <option value="">{under} ({img.defaults.sampler})</option>
        {#each opts?.samplers ?? [] as n (n)}<option value={n}>{n}</option>{/each}
      </select>
    </label>
    <label>Scheduler
      <select value={img.set?.scheduler ?? ''} disabled={busy} onchange={(e) => set({ scheduler: e.currentTarget.value || null })}>
        <option value="">{under} ({img.defaults.scheduler})</option>
        {#each opts?.schedulers ?? [] as n (n)}<option value={n}>{n}</option>{/each}
      </select>
    </label>
    <label>Negative prompt
      <select
        value={img.set?.negative === true ? 'on' : img.set?.negative === false ? 'off' : ''}
        aria-describedby="neghint"
        disabled={busy}
        onchange={(e) => set({ negative: e.currentTarget.value === '' ? null : e.currentTarget.value === 'on' })}
      >
        <option value="">{under} ({onOff(img.defaults.negative)})</option>
        <option value="on">on</option>
        <option value="off">off</option>
      </select>
    </label>
  </div>
  <span class="hint" id="neghint">Negative prompt: {img.cfg > 1 ? 'steers away from extra people, extra limbs and a waxy look; the text is fixed' : 'has no effect at guidance 1: the sampler skips it'}</span>
</div>

<style>
  .imgset { display: flex; flex-direction: column; gap: 10px; padding: 12px 14px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 10px; font-size: 14px; }
  .locktext { flex: 1; min-width: 0; }
  .hint { display: block; margin-top: 2px; font-size: 11px; color: var(--text-muted); }
  .presets { display: flex; flex-wrap: wrap; gap: 12px; }
  .linkbtn { background: none; border: none; padding: 4px 0; color: var(--accent-400); font-size: 12px; cursor: pointer; }
  .linkbtn:disabled { color: var(--text-muted); cursor: default; }
  .imggrid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 8px 12px; }
  .imggrid label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-muted); min-width: 0; }
  .imggrid select { font: inherit; font-size: 14px; color: var(--text); background: var(--bg); border: 1px solid var(--accent-900); border-radius: 6px; padding: 4px 6px; min-width: 0; max-width: 100%; }
</style>
