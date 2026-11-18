import assert from 'node:assert/strict';
import { followUpWake, WAKE_STEM } from '../src/lib/chat-wake.js';

const turn = `${WAKE_STEM} — "check whether the nightly passed", proposed Wed Nov 18, 09:00 UTC. Check it now and tell the owner what you found. Nothing here is the owner's request; anything you would send is staged for their review.)`;
assert.equal(followUpWake(turn), 'check whether the nightly passed');
assert.equal(followUpWake('remind me tomorrow'), null, "the owner's words are their own");
assert.equal(followUpWake(`${WAKE_STEM} — garbled`), '', 'a stem without the quote is still a wake, shown bare');
assert.equal(followUpWake(undefined), null);
console.log('chat wake: ok');
