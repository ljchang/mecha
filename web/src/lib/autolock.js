// The page's half of the autolock (`serve/library.rs`): an unlock lapses
// after the owner's span with no one touching the page — the same span the
// server's token keeps, handed back by `POST /api/library/unlock` as
// `idle_secs`. The server's clock is the backstop for a page that was
// closed; this one is for a page left open on a phone, whose token a
// background poll could otherwise keep alive with nobody there.
//
// Measured from timestamps, never by counting timer ticks: a phone freezes
// a backgrounded tab's timers, so the check also runs the moment the page
// is shown again, before anything locked is on screen for a second look.

// What counts as someone using the page. Not the page's own traffic: a
// streamed reply arriving is the persona talking, not the owner. Not
// `scroll` either: the chat pins itself to the bottom with `scrollTop =` on
// every streamed event, and a scroll listener hears that as a touch — a
// reply streaming kept the unlock open (review of #469). A person's scroll
// arrives as a touch, a wheel or a key, which are all here.
export const ACTIVITY = ['pointerdown', 'keydown', 'wheel', 'touchstart', 'mousemove'];

// The span when an unlock answer names none: the server's default. Never
// "no autolock" — a guard whose point is that it cannot be skipped must not
// be skipped by a missing field (review of #469).
export const DEFAULT_IDLE_SECS = 15 * 60;

// The span to arm with, from an unlock answer's `idle_secs`.
export function idleSpan(secs) {
  return Number.isFinite(secs) && secs > 0 ? secs : DEFAULT_IDLE_SECS;
}

// Whether an unlock last used at `last` has lapsed by `now`.
export function lapsed(last, now, idleMs) {
  return idleMs > 0 && now - last >= idleMs;
}

// Watch for `idleMs` without activity, then call `onIdle` once and stop.
// `onReturn` runs when the page is shown again and has not lapsed — the
// moment to ask the server whether the token outlived a restart. Returns
// the function that stops watching.
export function watchIdle({
  idleMs,
  onIdle,
  onReturn = () => {},
  target = globalThis,
  doc = globalThis.document,
  now = () => Date.now(),
  every = 10_000,
}) {
  let last = now();
  let stopped = false;
  const touch = () => {
    last = now();
  };
  const check = () => {
    if (stopped || !lapsed(last, now(), idleMs)) return false;
    stop();
    onIdle();
    return true;
  };
  const shown = () => {
    if (doc?.visibilityState === 'hidden' || check()) return;
    onReturn();
  };
  const opts = { passive: true, capture: true };
  for (const ev of ACTIVITY) target.addEventListener(ev, touch, opts);
  doc?.addEventListener('visibilitychange', shown);
  target.addEventListener('pageshow', shown);
  const timer = every > 0 ? setInterval(check, every) : null;
  function stop() {
    if (stopped) return;
    stopped = true;
    if (timer) clearInterval(timer);
    for (const ev of ACTIVITY) target.removeEventListener(ev, touch, opts);
    doc?.removeEventListener('visibilitychange', shown);
    target.removeEventListener('pageshow', shown);
  }
  return stop;
}

// "15 minutes", for the settings row and the pane.
export function autolockLine(minutes) {
  if (minutes == null) return '';
  if (minutes % 60 === 0) {
    const h = minutes / 60;
    return `${h} hour${h === 1 ? '' : 's'}`;
  }
  return `${minutes} minute${minutes === 1 ? '' : 's'}`;
}
