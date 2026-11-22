// Settings → GitHub, the pure half (web/test/github-picker.mjs): which rows
// the list shows, what a click asks the server to write, and the words for
// what the collector will do. `SettingsGithub.svelte` holds the fetches.
//
// **The rule is the server's.** Whether a repository is selected is the
// collector's reader's answer (`included`, from `Selection::selects`), and
// what its owner's line alone gives it is `by_owner` beside it. Nothing here
// restates how a `[[repo]]` line and an `[[owner]]` line combine: a click
// compares the wanted state with `by_owner` and asks for a line only where
// they differ, and the page draws whatever the server answers back.

const YEAR_MS = 365 * 24 * 3600 * 1000;

/**
 * The repositories of one owner the list shows. Forks and archived ones are
 * hidden unless asked for (G4) — but never one that is selected or that the
 * last collect watched, so a watched repository cannot drop out of sight
 * behind a filter, and neither can its "dropped next collect" (review of
 * #913). `recent` keeps those pushed in the last year; a repository with no
 * push date is not called recent.
 */
export function visibleRepos(repos, { forks = false, archived = false, query = '', recent = false, now = Date.now() } = {}) {
  const q = query.trim().toLowerCase();
  return repos.filter((r) => {
    if (q && !r.full_name.toLowerCase().includes(q)) return false;
    if (r.included === true || r.collected) return true;
    if (r.fork && !forks) return false;
    if (r.archived && !archived) return false;
    if (recent) {
      const at = Date.parse(r.pushed_at ?? '');
      if (!Number.isFinite(at) || now - at > YEAR_MS) return false;
    }
    return true;
  });
}

/**
 * The change a repository's checkbox asks for: its own line only where the
 * owner's line would give the other answer, and no line where it agrees —
 * so the file holds exceptions, never restatements. `null` while the file
 * cannot be read (`by_owner` unknown): the page offers no edit then.
 */
export function repoChange(repo, want) {
  if (repo.by_owner === null || repo.by_owner === undefined) return null;
  return { op: 'repo', name: repo.full_name, include: want === repo.by_owner ? null : want };
}

/**
 * The owner's all switch, as the change it asks for: `all`, or back to
 * `listed` with every `[[repo]]` line kept — so ticking it and unticking it
 * is where the owner started, never their hand-made picks deleted (review
 * of #913). Clearing the owner's lines is `noneStep`'s, a separate action.
 */
export function ownerChange(login, all) {
  return { op: all ? 'select_all' : 'select_listed', login };
}

/**
 * "Select none" for an owner removes the owner's `[[repo]]` lines,
 * inclusions too, so where it would remove one it asks first, in the page:
 * `{ confirm }` with the sentence to show until `confirmed`, else
 * `{ change }` to send.
 */
export function noneStep(owner, confirmed = false) {
  const picked = [
    ...owner.repos.filter((r) => r.line === true).map((r) => r.full_name),
    ...(owner.unseen ?? []).filter((l) => l.include).map((l) => l.name),
  ];
  if (picked.length && !confirmed) {
    const n = picked.length;
    return {
      confirm: `This removes ${n === 1 ? 'the repository' : `the ${n} repositories`} you picked by name for ${owner.login} (${picked.join(', ')}), as well as anything else of it. Nothing of ${owner.login} is watched after.`,
    };
  }
  return { change: { op: 'select_none', login: owner.login } };
}

/** What an owner's line means, in words — unknown said as unknown. */
export function ownerLine(owner, selectionState) {
  if (selectionState === 'invalid') return 'repos.toml cannot be used, so what this owner includes is unknown';
  const l = owner.line;
  if (!l) return 'not named in repos.toml: only those ticked below';
  if (l.include === 'all')
    return `every repository, and any added later${l.forks ? ', forks included' : ''}${l.archived ? ', archived included' : ''}`;
  return 'only those ticked below';
}

/** The owner's filters (G4), as the change they ask for. */
export function filterChange(login, forks, archived) {
  return { op: 'filters', login, forks, archived };
}

/** A package correction: `name` a string, `'none'`, or `'detected'`. */
export function packageChange(repo, registry, to) {
  if (to === 'none' || to === 'detected') return { repo, registry, to };
  return { repo, registry, to: { name: String(to).trim() } };
}

/**
 * "3 of 12 selected", counted over every repository of the owner (not only
 * those the filters show), with the unknown case said as itself.
 */
export function ownerCount(owner) {
  const total = owner.repos.length;
  if (owner.repos.some((r) => r.included === null || r.included === undefined)) {
    return `${total} ${total === 1 ? 'repository' : 'repositories'} · selection unknown`;
  }
  const n = owner.repos.filter((r) => r.included).length;
  return `${n} of ${total} selected`;
}

/** Whether the next collect will treat this repository differently. */
export function pending(repo) {
  return typeof repo.included === 'boolean' && repo.included !== repo.collected;
}

/** One registry's mapping, in words. */
export function packageLine(p) {
  switch (p.state) {
    case 'owner':
      return { text: `${p.name} · yours`, owner: true };
    case 'owner_none':
      return { text: 'none · yours', owner: true };
    case 'detected':
      return { text: p.manifest ? `${p.name} · from ${p.manifest}` : p.name, owner: false };
    case 'none_found':
      return { text: 'none found', owner: false };
    case 'not_scanned':
      return { text: 'not scanned yet', owner: false };
    default:
      return { text: '—', owner: false };
  }
}

/** Whether a repository's mapping says anything beyond "nothing here". */
export function hasPackages(repo) {
  return (repo.packages ?? []).some((p) => p.state === 'owner' || p.state === 'owner_none' || p.state === 'detected');
}

/** "2026-11-20 11:41 UTC" from an RFC 3339 time, or the text as given. */
export function when(at) {
  const t = Date.parse(at ?? '');
  if (!Number.isFinite(t)) return at ?? '—';
  return new Date(t).toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
}

/**
 * The collector's last run, in words. Unknown is said: no store, an
 * unreadable one, and a store with no finished run are three findings, none
 * of them "nothing is watched".
 */
export function storeLine(store) {
  if (!store) return { text: '—', bad: false };
  switch (store.state) {
    case 'absent':
      return { text: 'No collection has run on this machine yet, so there is no list to choose from.', bad: false };
    case 'unreadable':
      return { text: `The collector's store could not be read: ${store.error}`, bad: true };
    case 'read': {
      if (!store.last_collect) return { text: 'The store holds no finished collection yet.', bad: false };
      // A run that stopped before listing (no `gh` login, no `gh`) still
      // writes its finished row: that is a finding, not a clean run with
      // nothing in it (review of #913).
      if (!store.listed_at)
        return {
          text: `Last collect finished ${when(store.last_collect.finished)}, and it never listed your repositories, so there is no list to choose from. \`mecha doctor\` says why.`,
          bad: true,
        };
      const listed =
        store.listed_at < store.last_collect.started
          ? ` The repository list is from ${when(store.listed_at)}: the last run could not list them.`
          : '';
      return { text: `Last collect finished ${when(store.last_collect.finished)}.${listed}`, bad: !!listed };
    }
    default:
      return { text: '—', bad: false };
  }
}

/**
 * Settings' index row for the picker, from its read (`null` before or
 * without one). No list is never "0 of 0 watched": a rate over nothing is
 * not a count.
 */
export function indexLine(github) {
  if (github === null || github === undefined) return { text: 'which repositories the GitHub board watches' };
  if (github.selection?.state === 'invalid') return { text: 'repos.toml cannot be used — nothing is watched', bad: true };
  const store = github.store;
  if (store?.state === 'unreadable') return { text: "the collector's store could not be read", bad: true };
  if (store?.state === 'absent') return { text: 'no collection yet' };
  if (store?.state === 'read' && store.last_collect && !store.listed_at)
    return { text: 'the last collect never listed your repositories', bad: true };
  const repos = (github.owners ?? []).flatMap((o) => o.repos);
  if (!repos.length) return { text: 'nothing listed yet' };
  const n = repos.filter((r) => r.included).length;
  return { text: `${n} of ${repos.length} repositories watched` };
}

/** A file that cannot be used, as the sentence the page shows; else null. */
export function fileNotice(file, what) {
  if (!file || file.state !== 'invalid') return null;
  return `${what} cannot be used, so the page edits nothing in it until it is fixed by hand: ${file.error ?? 'unreadable'}`;
}
