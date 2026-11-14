// Exercise the shipping send function: a refused memory turn must preserve
// exact owner text, and a memory run must never become ordinary steering.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { parseGrantCommand } from '../src/lib/grants.js';
const source = fs.readFileSync(new URL('../src/lib/Chat.svelte', import.meta.url), 'utf8');
function read(marker) {
  const start = source.indexOf(marker);
  assert.ok(start >= 0, marker);
  return source.slice(start, source.indexOf('\n  }\n', start) + 4);
}
const send = read('  async function send({ text: fixed = null, regenerate = null, regions = null, onFail = null } = {}) {');
const memorySend = source.includes('  async function sendMemory(') ? read('  async function sendMemory(') : '';
function page({ phase = 'ready', ordinal = 3, status = 200, text = '  preserve my words\n', response = { started: true }, during = null, fastDone = false } = {}) {
  return new Function('parseGrantCommand', 'settings', `
    let key = 'memory-test', draft = settings.text, attachments = [], memoryIntent = true;
    let memory = {phase: settings.phase, next_turn_ordinal: settings.ordinal};
    let memorySending = false, running = settings.phase === 'running', memoryConnected = true;
    let memoryRevision = 0, memoryNeedsResume = false;
    const memoryMaxBytes = 2048, calls = [], notes = [];
    const withAttachments = text => text, closeIncognito = () => {}, receiveInput = () => { running = true; };
    const pushEntry = e => notes.push(e.text), grantCommand = async () => { throw Error('unexpected grant'); };
    const fetch = async (url, opts) => {
      calls.push({url, body:JSON.parse(opts.body)});
      if (settings.during) draft = settings.during;
      if (settings.fastDone) { memoryRevision++; memory = {phase:'ready',next_turn_ordinal:4}; running = false; }
      return {ok: settings.status === 200, status: settings.status, text: async () => 'refused', json: async () => settings.response};
    };
    ${memorySend}
    ${send}
    return {send: () => send(), state: () => ({draft,calls,notes,memory,memorySending,running,memoryConnected})};
  `)(parseGrantCommand, {phase, ordinal, status, text, response, during, fastDone});
}
let failures = 0;
async function check(name, fn) { try { await fn(); console.log('ok', name); } catch (e) { failures++; console.error('FAIL', name, e.message); } }
const newMemorySession = read('  async function newMemorySession() {');
for (const uuid of ['12345678-1234-4567-89ab-123456789abc', null]) {
  await check(`new memory key obeys the server key contract (${uuid ? 'crypto' : 'fallback'})`, async () => {
    const p = new Function('globalThis', `
      let memoryAvailable = true, memoryOpening = false, drawer = true, memory, inputEl;
      const calls = [], memorySessions = new Map();
      const switchTo = () => {}, pushEntry = () => {};
      const fetch = async (url, options) => {
        calls.push({url,body:JSON.parse(options.body)});
        return {ok:true,json:async()=>({session:'native-id',memory:{mode:'episodic'}})};
      };
      ${newMemorySession}
      return {open:newMemorySession,calls};
    `)({crypto:uuid ? {randomUUID:()=>uuid} : undefined});
    await p.open();
    assert.equal(p.calls.length,1);
    const key = p.calls[0].url.slice('/api/chat/'.length);
    assert.ok(key.length <= 32 && /^[a-z0-9][a-z0-9_-]*$/.test(key), `invalid server key: ${key}`);
    assert.deepEqual(p.calls[0].body,{memory:'episodic'});
  });
}
await check('409 keeps the exact draft', async () => {
  const p = page({status:409}); await p.send(); assert.equal(p.state().draft, '  preserve my words\n');
});
await check('accepted memory send has exact text and ordinal, no ordinary attachments', async () => {
  const p = page(); await p.send(); const body=p.state().calls[0].body;
  assert.equal(body.text, '  preserve my words\n'); assert.equal(body.memory_turn,3);
  assert.deepEqual(Object.keys(body).sort(), ['memory_turn','request_id','text']); assert.equal(p.state().draft,'');
});
for (const phase of ['starting','running','blocked']) await check(`${phase} cannot steer or queue`, async () => {
  const p=page({phase}); await p.send(); assert.equal(p.state().calls.length,0); assert.equal(p.state().draft,'  preserve my words\n');
});
for (const ordinal of [null,0,-1,1.5,Number.MAX_SAFE_INTEGER+1]) await check(`invalid ordinal ${ordinal} cannot send`, async () => {
  const p=page({ordinal}); await p.send(); assert.equal(p.state().calls.length,0);
});
await check('UTF8 byte bound refuses before request', async () => {
  const p=page({text:'🙂'.repeat(513)}); await p.send(); assert.equal(p.state().calls.length,0);
});
await check('typing during response is not erased', async () => {
  const p=page({during:'next thought'}); await p.send(); assert.equal(p.state().draft,'next thought');
});
await check('grant-shaped text stays exact owner input under fixed profile', async () => {
  const p=page({text:'/grant /tmp/example'}); await p.send(); assert.equal(p.state().calls[0].body.text,'/grant /tmp/example');
});
await check('fast completion before POST acknowledgment stays ready', async () => {
  const p=page({fastDone:true}); await p.send(); assert.equal(p.state().running,false); assert.equal(p.state().memory.next_turn_ordinal,4);
});
await check('uncertain/refused send requires refresh before retry', async () => {
  const p=page({status:409}); await p.send(); await p.send(); assert.equal(p.state().calls.length,1);
});


// Hold real transcript reads across actual sends, and run the connection effect
// under Svelte's reactive runtime. These are authored responses, not HTTP/model
// calls. Compiling just the lifted logic keeps DOM unrelated to these races out.
const { compileModule } = await import('svelte/compiler');
const client = await import('svelte/internal/client');
const memoryDeclarations = source.slice(source.indexOf('  const memorySessions = new Map();'), source.indexOf('  $effect(() => {', source.indexOf('  const memorySessions = new Map();')));
const connectionStart = source.indexOf('  $effect(() => {', source.indexOf('  // Re-subscribe whenever the key changes;'));
assert.ok(connectionStart >= 0, 'shipping connection effect exists');
const connection = source.slice(connectionStart, source.indexOf('\n  });', connectionStart) + 6);
const harness = `
import { untrack } from 'svelte';
export function racePage(fetch, EventSource, connectInitially) {
  let key = $state('memory-test'), mode = 'read_only';
  let running = $state(false), gone = $state(null), draft = 'preserved draft';
  let entries = [], partialRun = false, liveFrom = 0, doneSeq = 0;
  let task, incognito = false, todo, taint, model, usage, error = null, grants = [];
  let viewSignal = null, loadGen = 0, todoGen = 0, queue = [], queueSeq = 0;
  let attachments = [], drawer = false;
  const receivedInputs = new Set(), inputDelivery = new Map();
  ${memoryDeclarations}
  memoryIntent = true; memoryConnected = !connectInitially;
  memory = {mode:'episodic', phase:'ready', next_turn_ordinal:3};
  memorySession = 'authored-native'; memorySessions.set(key, memorySession);
  const timers = [];
  const setTimeout = fn => (timers.push(fn), timers.length);
  const clearTimeout = () => {};
  const pageOnly = () => false, scrollDown = () => {}, loadRail = () => {}, refreshLists = () => {};
  const pushEntry = e => entries.push(e), closeIncognito = why => { gone = why; };
  ${read('  function catchUp(sessionKey) {')}
  ${read('  async function load(')}
  ${read('  function receiveInput(ev) {')}
  ${memorySend}
  ${send}
  ${read('  function switchTo(k) {')}
  ${read('  async function resumeSession(')}
  ${read('  function subscribe(sessionKey) {')}
  if (connectInitially) { ${connection} }
  return {
    load: () => load(key), send: () => send(), resume: () => resumeSession(memorySession, true),
    retry: () => { const fn=timers.shift(); if (fn) fn(); },
    state: () => ({memory, running, memoryConnected, memorySending, draft, error})
  };
}`;
const compiled = compileModule(harness, {filename:'memory-race.svelte.js', generate:'client'}).js.code
  .replace(/from (["'])(svelte(?:\/[^"']+)?)\1/g, (_, quote, specifier) => `from ${quote}${import.meta.resolve(specifier === 'svelte' ? 'svelte/internal/client' : specifier)}${quote}`);
const {racePage} = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const settle = async () => { for(let i=0;i<8;i++) { await Promise.resolve(); client.flush(); } };
const response = (status, data={}) => ({ok:status>=200&&status<300,status,json:async()=>data,text:async()=>'authored refusal'});
const readyTranscript = () => ({session:'authored-native',entries:[],running:false,held_by_run:false,memory:{mode:'episodic',phase:'ready',next_turn_ordinal:3}});
function race({connect=false}={}) {
  const wire=[], streams=[];
  const fetch=(url,opts={})=>new Promise(resolve=>wire.push({url,opts,resolve}));
  class Stream { constructor(url) {this.url=url;this.closed=false;streams.push(this);} close(){this.closed=true;} }
  let p;
  const dispose=client.effect_root(()=>{p=racePage(fetch,Stream,connect);});
  client.flush();
  return {p,wire,streams,dispose};
}
await check('a pre-send GET cannot undo accepted running state', async () => {
  const {p,wire,dispose}=race();
  try {
    const pendingRead=p.load(); const read=wire.shift();
    const sending=p.send(); const post=wire.shift();
    post.resolve(response(200,{started:true})); await sending;
    assert.equal(p.state().memory.phase,'running','ACK established running before old GET');
    read.resolve(response(200,readyTranscript())); await pendingRead;
    assert.equal(p.state().memory.phase,'running','old GET must not restore ready phase');
    assert.equal(p.state().running,true);
    await p.send(); assert.equal(wire.length,0,'no stale-ordinal second send');
  } finally {dispose();}
});
await check('a pre-send GET cannot undo refusal and enable retry', async () => {
  const {p,wire,dispose}=race();
  try {
    const pendingRead=p.load(); const read=wire.shift();
    const sending=p.send(); const post=wire.shift();
    post.resolve(response(409)); await sending;
    assert.equal(p.state().memoryConnected,false,'refusal requires explicit recovery');
    read.resolve(response(200,readyTranscript())); await pendingRead;
    assert.equal(p.state().memoryConnected,false,'old GET cannot authenticate recovery after refusal');
    assert.equal(p.state().draft,'preserved draft');
    await p.send(); assert.equal(wire.length,0,'no retry from stale GET');
  } finally {dispose();}
});
await check('same-key explicit resume restores a stopped event stream', async () => {
  const {p,wire,streams,dispose}=race({connect:true});
  try {
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    assert.equal(streams.length,1,'initial authenticated connection subscribed');
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    streams[0].onerror(); p.retry(); await settle();
    const restart=wire.shift(); assert.equal(restart.opts.method,'GET','reconnect never creates');
    restart.resolve(response(404)); await settle();
    assert.equal(streams[0].closed,true);
    const resuming=p.resume(); const post=wire.shift();
    assert.equal(post.url,'/api/resume'); assert.equal(JSON.parse(post.opts.body).memory,'episodic');
    post.resolve(response(200,{key:'memory-test',session:'authored-native',memory:readyTranscript().memory}));
    // The fixed effect may issue both its existence GET and the explicit
    // catch-up; answer every read, then observe the actual EventSource count.
    for(let i=0;i<8;i++) { await settle(); for(const req of wire.splice(0)) { assert.notEqual(req.opts.method,'POST','resume recovery cannot recreate'); req.resolve(response(200,readyTranscript())); } }
    await resuming; await settle();
    assert.equal(streams.filter(s=>!s.closed).length,1,'same-key recovery must have a live SSE stream');
  } finally {dispose();}
});
await check('an SSE-superseded initial transcript is read again before enabling send', async () => {
  const {p,wire,streams,dispose}=race({connect:true});
  try {
    assert.equal(p.state().memoryConnected,false,'new/resumed connection starts disabled');
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    assert.equal(streams.length,1);
    const stale=wire.shift();
    const fresh=readyTranscript(); fresh.memory.next_turn_ordinal=4;
    streams[0].onmessage({data:JSON.stringify({type:'memory',memory:fresh.memory})});
    stale.resolve(response(200,readyTranscript())); await settle();
    assert.equal(p.state().memory.next_turn_ordinal,4,'older read cannot replace the SSE snapshot');
    // Permit either immediate catch-up or the connection's bounded retry.
    // Every resulting request must remain read-only.
    let rereads=0;
    for(let i=0;i<4;i++) {
      p.retry(); await settle();
      for(const req of wire.splice(0)) {
        assert.equal(req.opts.method ?? 'GET','GET','catch-up cannot recreate or admit');
        rereads++; req.resolve(response(200,fresh));
      }
      await settle();
    }
    assert.ok(rereads>0,'superseded initial read requires another authenticated transcript read');
    assert.equal(p.state().memoryConnected,true,'healthy stream and fresh transcript restore readiness');
    assert.equal(p.state().memory.next_turn_ordinal,4);
    assert.equal(p.state().draft,'preserved draft');
    assert.equal(streams.filter(s=>!s.closed).length,1,'only one active subscription');
  } finally {dispose();}
});
await check('reconnect catch-up cannot undo a newer send refusal', async () => {
  const {p,wire,streams,dispose}=race({connect:true});
  try {
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    const sending=p.send(); const post=wire.shift();
    streams[0].onerror(); p.retry(); await settle();
    wire.shift().resolve(response(200,readyTranscript())); await settle();
    const stale=wire.shift();
    post.resolve(response(409)); await sending;
    stale.resolve(response(200,readyTranscript())); await settle();
    // A network refresh begun before refusal cannot silently act as the
    // owner's subsequent explicit resume, even if its replacement GET is OK.
    for(let i=0;i<4;i++) {
      p.retry(); await settle();
      for(const req of wire.splice(0)) {
        assert.equal(req.opts.method ?? 'GET','GET');
        req.resolve(response(200,readyTranscript()));
      }
      await settle();
    }
    assert.equal(p.state().memoryConnected,false,'newer refusal still requires explicit recovery');
    assert.equal(p.state().draft,'preserved draft');
    await p.send(); assert.equal(wire.length,0,'no automatic retry of the refused turn');
  } finally {dispose();}
});
if (failures) process.exit(1);
