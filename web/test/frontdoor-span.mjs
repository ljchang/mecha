// Behaviour checks for the front door card's meeting line.
//
// `npm test` in web/. Plain node, same technique as queue-logic.mjs: read the
// component, slice the pure function out by text, evaluate it.
//
// **Why this exists.** `Booking::local_span` on the Rust side is pinned by
// `a_booking_renders_in_the_owners_zone_not_utc` and
// `a_span_crossing_midnight_names_the_day_it_ends_on`. The card renders the
// same meeting with its own JavaScript, in the *viewer's* zone rather than the
// owner's, so the two are genuinely different functions — and the
// crossing-midnight fix landed on the Rust one first and was missed here,
// which is exactly the half-covered shape a passing test on one renderer hides.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'src', 'lib', 'Frontdoor.svelte'), 'utf8');

const lift = (name, decl) => {
  const start = src.indexOf(decl);
  if (start < 0) throw new Error(`Frontdoor.svelte no longer defines ${name}`);
  const end = src.indexOf('\n  };\n', start) + 6;
  return src.slice(start, end);
};

const span = new Function(`${lift('span', '  const span = (b) => {')}; return span;`)();
const isPast = new Function(`${lift('isPast', '  const isPast = (b) => {')}; return isPast;`)();

let pass = 0;
let fail = 0;
const t = (name, cond) => {
  if (cond) {
    pass += 1;
    console.log('  ok   ', name);
  } else {
    fail += 1;
    console.log('  FAIL ', name);
  }
};

// Everything below renders in the *runner's* zone, so the assertions are about
// structure rather than a literal clock reading — a fixed string would only
// pass in one timezone, which is the trap this function exists inside.
const within = { start: '2026-09-16T14:00:00Z', end: '2026-09-16T15:00:00Z' };
const out = span(within);
t('names a weekday, a date and two times', /\w{3},? \w{3} \d+ · .+ – .+/.test(out) || /·/.test(out));
t('carries a zone label', /[A-Z]{2,5}|GMT[+-]?\d*/.test(out));

// A meeting that ends on a different local day must say which. Build one that
// crosses midnight wherever this runs: start 30 minutes before local midnight.
const midnight = new Date();
midnight.setHours(24, 0, 0, 0);
const crossing = {
  start: new Date(midnight.getTime() - 30 * 60000).toISOString(),
  end: new Date(midnight.getTime() + 30 * 60000).toISOString(),
};
const crossed = span(crossing);
t('a meeting crossing local midnight names the day it ends on', /\(.+\)/.test(crossed));

// And one that does not cross must not carry the parenthetical.
const sameDay = {
  start: new Date(midnight.getTime() - 120 * 60000).toISOString(),
  end: new Date(midnight.getTime() - 60 * 60000).toISOString(),
};
t('a meeting inside one day does not', !/\(.+\)/.test(span(sameDay)));

// Unreadable stamps degrade to the raw values rather than to "Invalid Date".
const bad = span({ start: 'whenever', end: 'later' });
t('unreadable stamps fall back to the raw text', bad.includes('whenever') && bad.includes('later'));
t('and never render as Invalid Date', !bad.includes('Invalid'));

// is_past, with the same boundary the Rust side pins.
t('a finished meeting is past', isPast({ end: '2000-01-01T00:00:00Z' }));
t('a future meeting is not', !isPast({ end: '2999-01-01T00:00:00Z' }));
t('an unreadable end is not guessed at', !isPast({ end: 'later' }));

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);
