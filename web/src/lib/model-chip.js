// What the chat's model chip shows and offers (REMOTE-SURFACE-DESIGN §14,
// step 5), from `GET /api/model` — which is `mecha model list --json`.
//
// Framework-free so `test/model-chip.mjs` pins it: the chip is where a wrong
// reading is silent. An unreadable router shown as "nothing loaded", or a
// model offered that runs would not follow, looks fine on the page and is
// wrong on the machine.

/// The router the chip speaks for: the first one that answered. There is one
/// in practice (`:8080`); several would be a config the chip does not try to
/// arbitrate, and `mecha model list` shows them all.
export function routerOf(data) {
  const all = data?.routers ?? [];
  return all.find((r) => r.reachable) ?? null;
}

/// Why no router can be offered, in the owner's words — or null when one can.
/// "No router" and "router down" need different answers (the CLI's rule).
export function unavailable(data) {
  const all = data?.routers ?? [];
  if (!all.length) return 'no provider follows a router, so there is nothing to switch';
  if (!all.some((r) => r.reachable)) return `the model server at ${all[0].base_url} is not answering`;
  return null;
}

const STATUS_WORD = {
  loaded: 'loaded',
  sleeping: 'loaded, idle',
  loading: 'loading',
  unloaded: '',
  downloading: 'downloading',
};

/// One row per model the router serves. `name` is what `mecha model use`
/// is handed: the single provider entry naming it, so a model id shared
/// across routers cannot pick the wrong one. A row is refused (with its
/// reason) when runs would not follow it, or when R4 would refuse the load —
/// the page does not offer what the CLI would turn down.
/// In an incognito chat a model is also refused when the entry a run binds
/// for it would make the chat refuse its turns (`incognito_refusal`: cloud
/// `fallbacks`, or not on this machine) — offering it would strand the chat
/// after a menu promising it stays local (found on review).
export function rows(router, incognito = false) {
  if (!router) return [];
  return (router.models ?? []).map((m) => {
    const why =
      m.would_not_follow ??
      (m.sampling_mismatches?.length ? m.sampling_mismatches.join('; ') : null) ??
      (incognito && m.incognito_refusal ? `an incognito chat refuses it: ${m.incognito_refusal}` : null);
    return {
      id: m.id,
      name: m.providers?.length === 1 ? m.providers[0] : m.id,
      current: router.readable && router.resident === m.id,
      status: m.status,
      word: m.status in STATUS_WORD ? STATUS_WORD[m.status] : m.status,
      disabled: !!why,
      why: why ?? null,
    };
  });
}

/// What the chip is doing right now. A pending switch outranks a loading
/// model: the switch file lives until the load finishes, so "switching"
/// covers the wait for runs *and* the load, and the router's own `loading`
/// status is what distinguishes the two.
export function phase(router) {
  if (!router) return { kind: 'unknown' };
  const p = router.pending_switch;
  if (p) {
    // A switch file nobody can read: pending (the fail-closed way), with no
    // switcher behind it — so nothing is loading and nothing can hurry it.
    // `readable` absent is an older build's list, whose files were readable.
    if (p.readable === false) {
      return { kind: 'switching', stuck: true, past: false, to: null, loading: false, waitingOn: [] };
    }
    const loading = (router.models ?? []).some((m) => m.id === p.to && m.status === 'loading');
    // Past its wait, the switch is unloading or about to load: no run is
    // holding it any more, and "switch now" or cancel would do nothing. The
    // router reports `loading` only once the load has begun, so this is the
    // switcher's own word for the seconds before (found on review of #364).
    const past = p.past_the_wait === true;
    return {
      kind: 'switching',
      stuck: false,
      past,
      to: p.to,
      loading,
      waitingOn: loading || past ? [] : p.waiting_on ?? [],
    };
  }
  const loading = (router.models ?? []).find((m) => m.status === 'loading');
  if (loading) return { kind: 'loading', id: loading.id };
  if (!router.readable) return { kind: 'unknown' };
  return { kind: 'idle', resident: router.resident ?? null };
}

/// One read on the wire at a time, and a way to ask for one that *starts*
/// after the asking. `refresh` shares a read in flight — polls need no more.
/// `fresh` waits one out and then reads again, because a read that began
/// before the tap may predate a switch that has ended since: sharing it let
/// `pick` take a stale baseline and report that switch's outcome as this
/// tap's (found on review).
export function reader(read) {
  let inflight = null;
  const refresh = () =>
    (inflight ??= (async () => {
      try {
        return await read();
      } finally {
        inflight = null;
      }
    })());
  const fresh = async () => {
    if (inflight) await inflight;
    return refresh();
  };
  return { refresh, fresh };
}

/// Poll while something is moving; stop when the router is at rest.
export function busy(ph) {
  return ph.kind === 'switching' || ph.kind === 'loading';
}

/// How often to re-read, in ms — or null for not at all. Every read is a
/// child process on the server, and D13's wait for runs has no time limit,
/// so a closed chip over a switch that is only *waiting* reads slowly; an
/// open menu, or a load (seconds from done), reads fast (found on review).
export function pollEvery(ph, open) {
  if (open || (ph.kind === 'switching' && ph.loading) || ph.kind === 'loading') return 2000;
  if (ph.kind === 'switching') return 15000;
  return null;
}

/// The chip's text. `fallback` is the model this chat's agent is bound to,
/// shown until the router has been read.
export function chipLabel(ph, fallback) {
  switch (ph.kind) {
    case 'switching':
      return ph.stuck ? 'switch stuck' : `→ ${ph.to}`;
    case 'loading':
      return `loading ${ph.id}`;
    case 'idle':
      return ph.resident ?? 'no model loaded';
    default:
      return fallback || '…';
  }
}

/// Whether "switch now" and cancel can still do anything: only while a
/// readable switch is waiting for runs — not once it is loading or past its
/// wait, and not for a stuck one (which offers cancel alone).
export function canHurry(ph) {
  return ph.kind === 'switching' && !ph.stuck && !ph.past && !ph.loading;
}

/// The line under the menu while a switch is pending: what it waits for, in
/// the words each run gave its hold (D13's "waiting for: …").
export function waitingLine(ph) {
  if (ph.kind !== 'switching') return null;
  if (ph.stuck) {
    return 'a switch file here cannot be read, and every run on this router waits for it — cancel withdraws it';
  }
  if (ph.loading) return `loading ${ph.to}…`;
  if (ph.past) return `switching to ${ph.to}…`;
  // No runs left, but the router has not taken the load: the loaded model is
  // still answering a request (a client that takes no hold). Not "switching"
  // — "switch now" and cancel both still act here (found on review of #371).
  if (!ph.waitingOn.length) return `waiting for the loaded model to finish a request — switching to ${ph.to} next`;
  return `waiting for: ${ph.waitingOn.join(', ')}`;
}

/// How the switch this page started ended — or null. `asked` is what the
/// server's own record said just before the tap (`{ before: <last_switch.at
/// or null> }`), so "newer than the tap" compares the server's clock with
/// itself: a browser clock ahead of the server's dropped the very failure
/// this exists to report, and one behind showed an older switch's as this
/// one's (found on review). Only a failure or a warning is worth a line;
/// success shows as the chip's new label.
export function outcomeNote(data, asked) {
  const last = data?.last_switch;
  if (!last || !asked || last.at === asked.before) return null;
  if (!last.ok) return { tone: 'bad', text: last.message || `the switch to ${last.to} failed` };
  if (last.message) return { tone: 'warn', text: last.message };
  return null;
}

// ---- the catalog (MODEL-CATALOG-DESIGN §1) ----
//
// `model list --json` carries `catalog`: every model mecha can install here,
// whether this machine's router can take one at all (`refusal`), and an
// install under way. The chip offers the ones the router does not serve yet,
// each with what tapping it would cost — the download and the memory — so the
// tap that installs is an approval of numbers it has seen.

/// GiB, one decimal: what the CLI says too.
export function gib(bytes) {
  return `${(bytes / 1073741824).toFixed(1)} GiB`;
}

/// Why nothing in the catalog can be installed here — this router is not
/// mecha's, or the catalog could not be read — as a line the menu shows.
/// A title on each greyed row is hover-only, so unreachable on a phone, and
/// an empty section with no word reads as a build with no catalog (review
/// of #727).
export function catalogTrouble(data) {
  return data?.catalog?.refusal ?? null;
}

/// The catalog models not served by `router`, as rows. A model this machine
/// is not offered stays listed, refused with why — "offered at the 64, 128 GB tiers" is
/// an answer, a missing row is not. With no router read yet nothing is
/// offered: what is served is unknown, and an install over a model already
/// there is a restart for nothing.
export function available(data, router) {
  const cat = data?.catalog;
  if (!cat || !Array.isArray(cat.offers)) return [];
  const all = data?.routers ?? [];
  // Served is known from a reachable router; no followed router at all (a
  // fresh install) serves none of the catalog by construction.
  if (all.length && !router) return [];
  const served = new Set((router?.models ?? []).map((m) => m.id));
  return cat.offers
    .filter((o) => !served.has(o.id))
    .map((o) => {
      const why = o.unfit ?? cat.refusal ?? null;
      return {
        id: o.id,
        label: o.label,
        note: o.note,
        download: o.download_bytes ?? 0,
        memoryMb: o.memory_mb ?? null,
        provider: o.provider ?? null,
        unfit: !!o.unfit,
        disabled: !!why,
        why,
      };
    });
}

/// What installing `row` costs, in one line.
export function planLine(row) {
  const parts = [`${gib(row.download)} to download`];
  if (row.memoryMb != null) parts.push(`~${gib(row.memoryMb * 1048576)} in memory, in place of the loaded model`);
  // The card is the approval `model add` asks for, so it names the config it
  // writes, as the CLI's plan does (review of #727).
  if (row.provider) parts.push(`writes [providers.${row.provider}] to your config`);
  return parts.join(' · ');
}

/// The install under way, or null — the child's own progress record, which
/// `model list` reports only while its process is alive.
export function installing(data) {
  return data?.catalog?.installing ?? null;
}

/// The line the menu shows for an install under way.
export function installLine(p) {
  if (!p) return null;
  switch (p.phase) {
    case 'fetching':
      return p.total_bytes > 0
        ? `downloading ${p.id} — ${gib(p.done_bytes)} of ${gib(p.total_bytes)}`
        : `checking ${p.id}'s files`;
    case 'restarting':
      return `restarting the router with ${p.id} — once no run is using it`;
    default:
      return `installing ${p.id}`;
  }
}

/// How often to re-read while an install runs: a download moves slowly, so
/// a closed chip reads every 15 s and an open menu every 2 s.
export function pollWhileInstalling(p, open) {
  if (!p) return null;
  return open ? 2000 : 15000;
}
