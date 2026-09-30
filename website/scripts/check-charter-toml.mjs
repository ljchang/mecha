// What the web settings page shares with `mecha_core::charter`, pinned.
//
// The page no longer writes `charter.toml`: it sends its rows, and the server
// sets them in place with `tomlform` over `charter::form`, so one language
// describes the file (the JavaScript serialiser this script used to pin
// against `charter.rs`, byte for byte, is gone — `charter.rs`'s
// `the_list_editors_rows_are_what_this_reader_loads` is that half now, in
// Rust). Two things still cross the boundary and are checked here: how the
// page derives a new line's id, and the sensor kinds the docs demo carries a
// copy of.

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { slugify, toRows } from '../../web/src/lib/charter-toml.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const fail = (msg) => {
  console.error(`check-charter-toml: ${msg}`);
  process.exit(1);
};
let checks = 0;
const eq = (got, want, what) => {
  checks++;
  if (got !== want) fail(`${what}\n  got:  ${JSON.stringify(got)}\n  want: ${JSON.stringify(want)}`);
};
const rs = readFileSync(join(root, 'mecha-core/src/charter.rs'), 'utf8');

// --- ids ------------------------------------------------------------------
eq(slugify('Say no early'), 'say-no-early', 'a slug from typed text');
eq(slugify('  Punctuation!! and — dashes  '), 'punctuation-and-dashes', 'punctuation collapses');
eq(slugify('one two three four five six seven'), 'one-two-three-four-five', 'capped at five words');

// --- the sensor kinds the demo offers -----------------------------------
// `sensor_kinds_json` is served so the real page never carries a copy; the
// docs demo has to, and a copy nothing pins drifts the moment a kind joins
// or a hint is reworded, with every test still green. The literal between
// the markers is asserted equal to the function in Rust; here the demo
// fixture is asserted equal to the literal, so the chain reaches the demo.
const kindsMarked = /\/\/ sensor-kinds:begin[\s\S]*?r#"([\s\S]*?)"#;[\s\S]*?\/\/ sensor-kinds:end/.exec(rs);
if (!kindsMarked) fail('could not find the sensor-kinds markers in mecha-core/src/charter.rs');
const kindsPinned = JSON.parse(kindsMarked[1]);
const { charter: demoCharter } = await import('../../web/src/demo/fixtures.js');
eq(
  JSON.stringify(demoCharter.sensor_kinds),
  JSON.stringify(kindsPinned),
  'the demo fixture offers a sensor-kind list that mecha-core no longer serves'
);

// --- the fields a list save sends ----------------------------------------
// `toRows` is what the page sends and `charter::form` what the server's
// fence accepts; a field renamed on one side refuses every list save while
// both sides' own tests pass (review of #439). The literal between the
// markers is asserted equal to `form()`'s fields in Rust; here `toRows` is
// asserted to send exactly those, sensor and all.
const fieldsMarked = /\/\/ charter-form-fields:begin[\s\S]*?r#"([\s\S]*?)"#;[\s\S]*?\/\/ charter-form-fields:end/.exec(rs);
if (!fieldsMarked) fail('could not find the charter-form-fields markers in mecha-core/src/charter.rs');
const pinnedFields = JSON.parse(fieldsMarked[1]);
const [sent] = toRows([{ id: 'a', text: 't', sensor: { kind: 'outbox_age', setpoint: '24h' }, reading: {}, uid: 1 }]);
const sentFields = Object.entries(sent).flatMap(([k, v]) =>
  v && typeof v === 'object' ? Object.keys(v).map((sub) => `${k}.${sub}`) : [k]
);
eq(JSON.stringify(sentFields), JSON.stringify(pinnedFields), 'toRows sends fields the charter form does not declare, or misses one it does');

console.log(`check-charter-toml: ${checks} checks, the page and mecha-core agree on ids, row fields and sensor kinds`);
