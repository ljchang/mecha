// The demo transport: `fetch` and `EventSource`, answered from fixtures.
//
// This exists so the documentation site can embed the *real* web app rather
// than a screenshot of it. What a reader clicks on docs.mecha-factory.ai is
// this bundle, built from `web/src/` by the same Vite config, with the same
// components and the same stylesheet — only the bytes coming back over `/api`
// are invented (`./fixtures.js` says why they have to be).
//
// **Why a transport shim and not a demo mode inside the components.** The app
// reaches the server through exactly two primitives: 57 bare `fetch('/api/…')`
// calls and one `EventSource`. Replacing those two is one file that no
// component knows about, so a page cannot drift into rendering differently
// under the demo — there is no `if (demo)` anywhere for it to drift through.
// A flag threaded through twelve components would have twelve chances to lie.
//
// **Why it is not in the shipped build.** `installDemo` is called behind
// `import.meta.env.VITE_MECHA_DEMO` in `main.js`, which Vite replaces with a
// literal at build time, so Rollup drops both the branch and this module from
// a normal `npm run build`. `npm run check-demo` in the docs site greps the
// production bundle for a fixture string, because "tree-shaking should have
// handled it" is exactly the class of belief this project makes a test out of.
//
// **Fail loudly, not blankly.** An unmatched route returns a 501 naming the
// path it could not answer. The alternative — an empty 200 — renders as a page
// with nothing on it, which reads to a docs reader as "this feature does
// nothing". `website/scripts/check-demo.mjs` is what stops one shipping: it
// asks the ROUTES table below whether every endpoint the app reaches is
// answered, and fails the docs build when one is not.

import * as fx from './fixtures.js';

const json = (value) =>
  new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });

const text = (value) => new Response(value, { status: 200 });

// The live event stream, when the page has one open. A scripted run pushes
// through here; there is at most one because the app opens one per session
// key and the demo only ever shows one key.
let stream = null;

function emit(event) {
  stream?.push(event);
}

/** Replay `fixtures.script` into the open stream, on its own clock. */
function replay(userText) {
  emit({ type: 'queued', text: userText });
  let at = 0;
  for (const [delay, event] of fx.script) {
    at += delay;
    setTimeout(() => emit(event), at);
  }
}

// Routes are matched in order, first match wins. A function may return a
// value (sent as JSON) or a `Response`.
// Exported so `website/scripts/check-demo.mjs` can ask this table directly
// rather than scraping it: a guard that reads the real object cannot drift
// from it, and a regex over source can.
export const ROUTES = [
  ['GET', /^\/api\/ping$/, () => ({ ok: true })],
  ['GET', /^\/api\/summary$/, () => fx.summary],

  ['GET', /^\/api\/mail$/, () => fx.mail],
  ['GET', /^\/api\/mail\/inbox$/, () => fx.mailInbox],
  [
    'GET',
    /^\/api\/mail\/read$/,
    (url) => fx.mailRead[url.searchParams.get('thread_id')] ?? fx.mailRead['thr-8812'],
  ],

  ['GET', /^\/api\/outbox$/, () => fx.outbox],
  ['GET', /^\/api\/outbox\/([^/]+)$/, (_url, [id]) => fx.outboxDetail[id] ?? fx.outboxDetail['ob-4417']],

  ['GET', /^\/api\/queue$/, () => fx.queue],
  ['GET', /^\/api\/queue\/classes$/, () => fx.queueClasses],
  // Review-on-use: facts the graph served to a run that nobody has ruled on.
  ['GET', /^\/api\/queue\/shadow$/, () => fx.shadow],

  ['GET', /^\/api\/frontdoor$/, () => fx.frontdoor],
  // Plain text, not JSON — `mecha frontdoor show <seq>`, which is what the
  // reader pane renders. Answering this with an object would have been a
  // silent `[object Object]` in the pane.
  ['GET', /^\/api\/frontdoor\/read$/, () => text(fx.frontdoorShow)],

  ['GET', /^\/api\/notes$/, () => fx.notes],
  ['GET', /^\/api\/find$/, () => fx.find],

  // One entity, everything the graph knows about it. The page always asks by
  // name, and the demo has one person to answer with — a lookup for anyone
  // else honestly reports not-found rather than pretending the graph is full.
  [
    'GET',
    /^\/api\/entity$/,
    (url) => {
      const asked = (url.searchParams.get('name') ?? '').trim().toLowerCase();
      if (!asked || fx.entity.query.toLowerCase().includes(asked) || asked.includes('priya')) {
        return fx.entity;
      }
      return { found: false, query: url.searchParams.get('name') };
    },
  ],

  ['GET', /^\/api\/tasks$/, () => fx.tasks],
  ['GET', /^\/api\/questions$/, () => fx.questions],

  ['GET', /^\/api\/settings\/charter$/, () => fx.charter],
  ['GET', /^\/api\/settings\/rules$/, () => fx.rules],
  ['GET', /^\/api\/settings\/reflections$/, () => fx.reflections],
  [
    'GET',
    /^\/api\/settings\/learning-report$/,
    () => fx.learningReport,
  ],
  [
    'GET',
    /^\/api\/settings\/reflections\/show$/,
    (url) =>
      fx.reflectionDetail[url.searchParams.get('id')] ??
      fx.reflectionDetail['20260826T143000-7f21a9c4'],
  ],
  ['GET', /^\/api\/settings\/voice$/, () => fx.voice],

  ['GET', /^\/api\/sessions$/, () => fx.sessions],
  ['GET', /^\/api\/history$/, () => fx.history],
  ['GET', /^\/api\/chat\/[^/]+$/, () => fx.transcript],
  // Never reached: `EventSource` is replaced wholesale below, so the stream
  // does not go through `fetch` at all. Listed anyway, because `check-demo`
  // asks this table whether every endpoint the app reaches is accounted for,
  // and "it is handled somewhere else" is exactly the answer a guard should
  // be told explicitly rather than left to infer.
  ['GET', /^\/api\/chat\/[^/]+\/events$/, () => text('')],

  // The one interactive path. Whatever is typed, the scripted run replays:
  // the demo has no model behind it and should not pretend to, which is what
  // the frame's caption on the docs site says out loud.
  [
    'POST',
    /^\/api\/chat\/[^/]+\/send$/,
    async (_url, _params, init) => {
      let typed = '';
      try {
        typed = JSON.parse(init?.body ?? '{}').text ?? '';
      } catch {
        // A body that will not parse is still a send; the script does not
        // depend on it.
      }
      replay(typed);
      return text('');
    },
  ],
  ['POST', /^\/api\/chat\/[^/]+\/cancel$/, () => text('')],
  [
    'POST',
    /^\/api\/chat\/[^/]+\/mode$/,
    async (_url, _params, init) => {
      // The mode chip tracks the *server*, not the tap — the real one echoes
      // the change back over the stream, so the demo does too. Getting this
      // wrong would make the chip look local, which is the opposite of the
      // property the docs describe.
      let mode = 'read_only';
      try {
        mode = JSON.parse(init?.body ?? '{}').mode ?? mode;
      } catch {
        /* keep the default */
      }
      setTimeout(() => emit({ type: 'mode', mode }), 60);
      return text('');
    },
  ],

  // The demo boundary: everything that would change something outside the
  // browser, plus the queue panes deep enough to need a real store behind
  // them. Each answers with a sentence naming itself as the demo, because
  // every one of these paths surfaces its error text in the pane that called
  // it — so the alternative is a page that reads as a broken feature.
  //
  // Deliberately enumerated rather than a trailing catch-all. A catch-all
  // would also swallow the next endpoint somebody adds, and `check-demo`
  // would go green on a page the demo cannot actually draw.
  //
  // The five `settings/(reflections|rules)/…` verbs are the one group here
  // that `check-demo` cannot see: `SettingsLearning.svelte` reaches them
  // through its one `act(path, body, fallback)` helper, so the path is a
  // variable at the `fetch` call and the guard's regex — which reads
  // literals — never learns of them. They are listed anyway, and this
  // paragraph is why: without an entry each one falls through to "demo: no
  // fixture for POST /api/…", which the learning pane renders in its own
  // error card, and a docs reader sees a broken feature rather than a
  // declined one.
  [
    '*',
    new RegExp(
      '^/api/(' +
        [
          'chat/[^/]+/(answer|upload)',
          'outbox/[^/]+/[^/]+',
          'mail/(act|compose)',
          'notes(/edit)?',
          'tasks/[a-z]+',
          'questions/[a-z]+',
          'frontdoor/act',
          'queue/(groups|items|verdict|bind|sample)',
          'queue/shadow/verdict',
          'settings/charter',
          'settings/reflections/(edit|drop|restore)',
          'settings/rules/(retire|restore)',
          'settings/voice/clone(/delete)?',
          'resume',
          'dictate',
          'offer',
        ].join('|') +
        ')$',
    ),
    () =>
      new Response(
        'This is the documentation demo — it has fixtures behind it, not a mecha. ' +
          'Run `mecha serve` on your own machine to do this for real.',
        { status: 501 },
      ),
  ],
];

function route(method, url, init) {
  for (const [verb, pattern, handler] of ROUTES) {
    if (verb !== '*' && verb !== method) continue;
    const match = pattern.exec(url.pathname);
    if (match) return handler(url, match.slice(1), init);
  }
  return null;
}

export function installDemo() {
  const realFetch = globalThis.fetch.bind(globalThis);

  globalThis.fetch = async (input, init) => {
    const raw = typeof input === 'string' ? input : input.url;
    const url = new URL(raw, location.href);
    if (!url.pathname.startsWith('/api/')) return realFetch(input, init);

    const method = (init?.method ?? 'GET').toUpperCase();
    const answer = await route(method, url, init);
    if (answer === null) {
      return new Response(`demo: no fixture for ${method} ${url.pathname}`, { status: 501 });
    }
    return answer instanceof Response ? answer : json(answer);
  };

  // EventSource, minus the network. Only `onmessage` is implemented, because
  // that is the only handler `Chat.svelte` sets — an unused `addEventListener`
  // stub would be a claim this object is a polyfill, which it is not.
  class DemoEventSource {
    constructor(url) {
      this.url = url;
      this.onmessage = null;
      this.readyState = 1;
      stream = this;
    }

    push(event) {
      if (this.readyState !== 1) return;
      this.onmessage?.({ data: JSON.stringify(event) });
    }

    close() {
      this.readyState = 2;
      if (stream === this) stream = null;
    }
  }

  globalThis.EventSource = DemoEventSource;
}
