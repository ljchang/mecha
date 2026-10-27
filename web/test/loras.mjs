// Behaviour checks for Settings → LoRAs' pure half (src/lib/loras.js): the
// form an entry is written through, and what a row says about it.
import { SECTIONS, listUrl, entryUrl, standing, filesOf, rowDetail, others, suggestName, formFor, formFrom, entryOf, unusedLine } from '../src/lib/loras.js';

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
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

console.log('routes');
t('each section has its own feature', eq(SECTIONS.map((s) => s.feature), ['image', 'video']));
t('the unlock rides along only when there is one', listUrl('image', null) === '/api/loras/image' && listUrl('video', 'a b') === '/api/loras/video?unlock=a%20b');
t('a name is escaped in its route', entryUrl('video', 'a/b', 't') === '/api/loras/video/a%2Fb?unlock=t');

console.log('what a row says');
t('a checked picture LoRA is not applied yet', standing({ status: 'checked' }, 'qwen-image').text.includes('not applied'));
t('a checked clip LoRA is checked', standing({ status: 'checked' }, 'h3').text === 'checked');
t('a left-out entry gives the server\'s reason', eq(standing({ status: 'missing', reason: 'left out: x' }, 'h3'), { text: 'left out: x', bad: true }));
t('one file for both modes is listed once', eq(filesOf({ frames: 'a.safetensors', references: 'a.safetensors' }), ['a.safetensors']));
t('a file per mode is labelled', eq(filesOf({ frames: 'f.safetensors', references: null }), ['frames: f.safetensors']));

t('a row spells strength and trigger apart', rowDetail({ file: 'a.safetensors', strength: 0.7, trigger: 'x y' }) === 'a.safetensors · strength 0.7 · trigger "x y"');
t('one other file is one', others([{ family: 'h3' }], 'qwen-image') === '1 other file on this server');

console.log('the form');
t('a name starts from the file\'s stem', suggestName('sub/Wave Hello_v2.safetensors') === 'wave-hello_v2');
const pic = formFor('qwen-image', 'wc.safetensors');
t('a picture form takes one file', pic.file === 'wc.safetensors' && pic.frames === '');
const clip = formFor('h3', 'w.safetensors');
t('a clip form starts with the file in both modes', clip.frames === 'w.safetensors' && clip.references === 'w.safetensors');
t('the same file in both modes is sent as one', eq(entryOf({ ...clip, about: ' a wave ' }, 'h3'), { name: 'w', about: 'a wave', file: 'w.safetensors' }));
t('a file per mode is sent per mode', eq(entryOf({ ...clip, about: 'x', references: '' }, 'h3'), { name: 'w', about: 'x', frames: 'w.safetensors' }));
t('an empty strength is the default', !('strength' in entryOf({ ...pic, about: 'x' }, 'qwen-image')));
t('a strength is a number', entryOf({ ...pic, about: 'x', strength: '0.7' }, 'qwen-image').strength === 0.7);
t('a strength that is not a number goes as text, for the server to refuse', entryOf({ ...pic, about: 'x', strength: '0.8x' }, 'qwen-image').strength === '0.8x');
t('always is a clip field only', !('always' in entryOf({ ...pic, about: 'x', always: true }, 'qwen-image')) && entryOf({ ...clip, about: 'x', always: true }, 'h3').always === true);
const back = formFrom({ name: 'w', about: 'x', frames: 'f.safetensors', references: 'r.safetensors', strength: 1, trigger: null, always: false });
t('an edit starts from the entry, a default strength left blank', back.editing === 'w' && back.strength === '' && back.frames === 'f.safetensors');

console.log('the add list');
t('a file of this model is addable', unusedLine({ family: 'h3', note: null }, 'h3').ok);
t('one of the other model says so', !unusedLine({ family: 'qwen-image' }, 'h3').ok && unusedLine({ family: 'qwen-image' }, 'h3').text.includes('picture'));
t('one whose model is unknown gives the reason', unusedLine({ family: null, note: 'it is in none of the folders' }, 'h3').text === 'it is in none of the folders');

console.log(`\n${pass} passed, ${fail} failed`);
if (fail) process.exit(1);
