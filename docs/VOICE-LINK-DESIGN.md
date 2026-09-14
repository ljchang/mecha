# The reliable voice link — design

Designed 2026-09-14; built the same day on `voice/reliable-uplink` (§2 as
amended below — three of its mechanisms changed on contact with the code,
each for a reason recorded in place). The question it answers: **when the
phone's connection to the box is unstable, what happens to what the owner
said?**

Today the answer is "it is lost." Speech rides a WebRTC audio track, RTP does
not retransmit, and on the morning call of 2026-09-14 (12:18–12:23 UTC) the
uplink stalled eighteen times for 0.7–13 s: Parakeet received eleven seconds
of speech in a five-minute call, every fragment transcribed correctly and
every truncation lined up with a stall — *"Today is Monday the four"*,
*"When is the / Cameron"*, and forty-seven closing seconds of the owner
talking into a dead uplink with no transcription at all. #226 (the pause
sounds and the turn hold) keeps the *turn* across a stall; it cannot keep
bytes that never arrived. `VOICE-RESEARCH.md` §*A gap in the audio is not
silence* is the record of that; this file assumes it.

The owner's framing, 2026-09-14: *"this could work much better if voice is
still recorded, just buffered until connection is available and agent can
catch up."* That is the design. What follows is the contract, the decisions,
the two honest limits, and what is deliberately not in scope.

## 1. The principle

**Input is never paused. Output waits.**

ChatGPT's voice mode pauses input during an outage. That is a constraint of
its transport, not a choice: real-time audio cannot buffer, so the honest
thing is to stop the person talking. Once capture and buffering live on the
phone, the constraint is gone and the design inverts. The phone is the one
place that always hears the owner; the box's job is to catch up.

Three consequences, each a requirement:

- **Nothing said is thrown away** while the page is alive. A 13 s stall is
  13 s of audio arriving 13 s late, complete and in order.
- **The assistant's reply is late, whole, and in order.** Speech through a
  stall is answered once the box has caught up; a barge-in during a stall
  still cancels the reply it was aimed at, because the backlog replays in
  order.
- **The owner is told how far behind the assistant is**, never whether
  packets are flowing. A stall that costs two seconds of latency is not an
  event.

## 2. The uplink contract

### 2.1 Capture on the phone — the encoded tap, not a worklet

As designed this was an `AudioWorklet` on the microphone stream. As built it
is not, because the page's own record forbids it: *"nothing on this page
touches the mic through WebAudio any more"* — WebKit silently disables echo
cancellation once WebAudio attaches to a `getUserMedia` track, and on
2026-08-24 that produced the TTS transcribed as the owner. A worklet tap
would reopen exactly that.

So the tap is on the **encoded** stream instead: an `RTCRtpScriptTransform`
on the audio sender (`web/public/voice-uplink-transform.js`, a worker from
this origin because the CSP is `script-src 'self'`) copies every Opus frame
the browser is about to put on RTP — post-AEC, post-AGC, byte-identical to
what the RTP path carries — and passes the original on untouched. Chrome's
older main-thread `createEncodedStreams` is the fallback; a browser with
neither keeps the RTP path and says so in the offer. Frames go into a ring
(`UplinkRing`) stamped on the page's **media clock**, derived from RTP
timestamp deltas at the 48 kHz codec clock: milliseconds of captured audio
since the session object was created, continuing across reconnects. Five
minutes of Opus is about a megabyte; the design's 16 kHz PCM would have
been ten times that for the same words, and would have touched the track.

### 2.2 Delivery over a reliable channel — RTVI messages, not a second channel

As designed, a second binary data channel. As built, **base64 batches inside
RTVI client messages on the existing `rtvi` channel**, because pipecat
1.7.0's `SmallWebRTCConnection` claims *every* channel the page opens as the
RTVI channel (`on_datachannel` sets `self._data_channel = channel` with no
label check) and JSON-parses its messages: a binary channel would have
hijacked RTVI and logged an error ten times a second. The cost is a third
more bytes on ~4 kB/s of Opus, which is nothing.

Each message is one 100 ms batch (`UPLINK_BATCH_MS`): `seq`, `ms` (media
clock of the first frame), `backlog_ms` (audio captured but not yet sent, at
send time), `frames` as `[duration_ms, base64]` pairs, and `dropped` spans
when the ring overflowed since the last batch. The page sends from the ring,
paced on `bufferedAmount` (`UPLINK_SCTP_HIGH_BYTES`), so the SCTP queue never
holds much and what waits, waits in the ring, where the page controls it. A
stall therefore looks like this from the page: the ring grows, `backlog_ms`
in each header grows, and nothing is dropped. `backlog_ms` counts the ring
*and* what the SCTP queue still holds (`wireBacklogMs`): the first cut sent
the ring alone, and with ~32 kB deliberately queued the last batches of a
drain reported near zero while seconds of the sentence were in flight, so
the worker lifted its hold mid-sentence (review of #231). A batch the
channel refuses goes back into the ring with its dropped spans
(`UplinkRing.unsend`). The channel is SCTP — ordered and reliable — and the
worker checks `seq` and warns on a gap.

### 2.3 Injection on the worker

`UplinkAudio` (worker.py) decodes each batch with PyAV's Opus decoder,
resamples to 16 kHz mono, and pushes 20 ms `InputAudioRawFrame`s into the
transport's **own audio queue** (`push_audio_frame`) — the place RTP audio
entered — so Silero, the segmenter, smart-turn and the aggregator see exactly
what they saw before and learn nothing about the route. The remainder of a
batch that is not a whole frame carries into the next; dropping it would
have lost a frame per batch across a drain. The RTP reader is parked by a
transport subclass (`UplinkTransport` → `UplinkInput._receive_audio` returns
at once) rather than by `audio_in_enabled=False`, because that flag also
gates `push_audio_frame` and the VAD that runs on the queue.

The page's RTP track keeps sending, **unmuted**: its receiver reports are
`linkVerdict`'s only uplink witness, and a muted track thins them. The
worker reads none of it; two copies of one voice would be a stream at twice
speed. The page declares the mode in the offer's `request_data`
(`uplink: "channel"`), and `bot()` picks the transport class from it before
a single RTP frame is read — a switch mid-call would deliver the first words
twice. Whether iOS keeps capture alive with the screen off is unchanged by
this build, because the track is exactly what it was.

**A tap that attaches and never delivers would be a deaf call**, silently
— RTP carrying the voice past a parked reader — and the first deploy that
forgets the worker file at `dist` root is exactly that (review of #231).
Both ends watch for it. The page declares the channel only after the
worker script's first message proves it loaded (`UPLINK_READY_MS`), and
falls back to RTP for the call if no frame reaches the ring within
`UPLINK_FIRST_FRAME_MS` of `connected`, saying so in the transcript. The
worker, which can act without a round trip, starts reading RTP after all
if no batch has arrived `UPLINK_DEAF_SECS` after the transport came up,
at `error` level, and ignores batches that arrive afterwards rather than
doubling the voice.

On the worker every delivery — live frames and late turns alike — goes
through **one ordered queue with one consumer, and nothing is cancelled**:
pipecat runs each client message's handler as its own task, so `on_audio`
decides and enqueues before its first `await`; the settle timer only
enqueues a flush. The first cut's timer *was* the flush after its sleep,
and a late batch arriving mid-transcription cancelled the turn with the
audio already moved into locals — minutes of speech gone without a log
(review of #231). `NothingIsCancelled` in `test_uplink.py` pins both the
loss and the ordering.

### 2.4 Turn-taking runs on media time — and mostly already does

The load-bearing property: when a backlog drains, frames arrive faster than
real time, and no decision may be taken on the wire's clock. Read against
pipecat 1.7.0 as installed:

- **Silero's stop** is frame-counted (`_vad_stop_frames`) — media time.
- **smart-turn's 3 s of silence** is `_silence_ms += chunk_duration_ms` —
  media time.
- **Three timers are wall-clock**, and they are exactly the ones `LinkWatch`
  already holds for: the VAD controller's `audio_idle_timeout` (1.0 s,
  `time.time()`), the aggregator's `user_turn_stop_timeout` (15 s, an
  `asyncio.sleep`), and `TranscriptStartedTurnStop`'s p99 timer
  (`STT_TTFS_P99`, 2.0 s).

So the rule is *preserve*, not *build*: **no wall-clock timer may end a
turn**, `LinkWatch`'s hold remains the guard for the three pipecat has, and
any new timer this design adds counts audio. One change to `LinkWatch`
follows: **"resumed" means caught up, not "the first packet arrived".**
`LinkWatch.note_uplink` records each batch's `backlog_ms`, and
`LinkResumedFrame` fires only when that is under `LINK_CAUGHT_UP_MS` (500 ms)
*and* frames have flowed for `LINK_RESUME_SETTLE_SECS`, so the hold outlives
the drain and the p99 timer cannot fire against a half-delivered sentence.
An RTP-only call has no backlog and resumes on flow alone, as before
(`test_uplink.py` pins both). This is the seventh #226 review's "resumed too
early" finding, generalised.

The first end-to-end call (`test_call.py --uplink --stall-at 2 --stall-secs
6`, 2026-09-14) found the one wall-clock timer the hold does not reach: the
VAD controller's `audio_idle_timeout` closes the *segment*, not the turn,
so the word straddling the stall reached Parakeet in two halves —
*"Wednesday foot."* / *"from twelve…"* — with every word present and the
turn intact. On the buffered uplink that timeout's job is the hold's, and a
real end of speech is still found by Silero on the silence that keeps
flowing, so the path sets it to `UPLINK_VAD_IDLE_SECS` (30 s); the RTP path
keeps pipecat's 1.0 s.

### 2.5 Two lanes: live and late

Every batch carries its capture time, so the worker knows how old audio is
on arrival. Age decides which lane it takes:

- **Live** (age under `BACKLOG_TALK_SECS`, start at 120 s): the pipeline as
  above. Hold, drain, decide on media time, answer.
- **Late** (older than that, or arriving into a *new* pipeline after a
  reconnect — §4.2): turn-taking is bypassed. The span is transcribed as a
  whole (in pieces of `LATE_CHUNK_SECS` to the same Parakeet server), waits
  for the bot to stop speaking so it never interrupts, and lands in the
  conversation as **one user turn the harness prefixes** — a
  `TranscriptionFrame` pushed at the head of the pipeline, so the page's
  transcript shows it too: *"(said 08:22–08:23, delivered late)"*, in the
  phone's own clock. A span is closed by the first live batch after it,
  or by `LATE_SETTLE_SECS` without one, and is put *before* the live audio
  that closed it. The model answers it as a turn. This is the outbox and questions shape
  applied to speech — a run's input surviving the run — and it is what
  keeps "the outage outlasted the conversation" from meaning "what was
  said is gone".

The boundary is a constant because it is a judgement: past two minutes the
assistant is not *in conversation* with what it is hearing, and pretending
otherwise produces barge-ins against replies nobody remembers.

## 3. What the owner hears, and when

The #226 sounds mean *packets stopped / packets resumed*. Under this design
that is the wrong event to sound: the morning call had twenty-two stalls, and
sounding each is noise, while "paused" would say *stop talking* — the one
thing the owner need not do. The cue keys on **how far behind the assistant
is**, measured on the page as ring depth plus SCTP queue:

- under `BEHIND_TONE_MS` (start at 3,000): nothing;
- past it: one soft "behind" cue and the label *listening · 8 s behind*,
  updated live from the page's own numbers — no round trip needed;
- the "caught up" cue when the ring has **drained**, not when the first
  packet flows — the same moment §2.4's `LinkResumedFrame` fires.

`Pauses` (the page's reason set) and the `link`/`mic` protocol stay: a `mic`
pause is still a real pause (§4.1). What changes, when the uplink is
buffered: the page's own `link` reason and the worker's `server`
announcement no longer pause, sound or relabel — the `link` verdict is
still sent so the worker holds the turn, and both are covered by the
behind count, which is the page's own ring depth plus what the SCTP queue
holds (`behindVerdict`, `withBehind`). Under the RTP path everything is as
#226 left it.

## 4. The two honest limits — where we do pause, and say so

### 4.1 The page dies

Safari suspending the tab, a screen lock without the wake lock, a crash:
capture itself stops, and "I can't hear you" is true. The page already
detects this as the mic track's mute edge (`t.onmute → pause("mic")`), and
it stays a pause with its own sound, distinct from *behind*. Nothing in this
design can buffer audio that was never captured, and the design says so
rather than implying otherwise.

### 4.2 The outage outlasts a conversation

The ring is bounded — `RING_BUFFER_SECS`, start at 300, about 9.6 MB — and
three things happen in order as an outage lengthens:

1. Past `BACKLOG_TALK_SECS` (120 s) the conversation is paused as such: the
   pause sound, the label *lost you — I'll pick up when you're back*. Capture
   continues into the ring.
2. A `disconnected` ICE state opens the existing 15 s grace
   (`DROP_GRACE_MS`) and ends the call if it never recovers. **The ring
   survives the end of the call**, in page memory, and is delivered at the
   head of the *next* connection in the same page session, on the late lane
   (§2.5), with its capture times. No ICE restart is attempted — pipecat's
   `restart_pc` fires the server-side `disconnected` the worker cancels the
   pipeline on, as `voice-core.js` records — so the reconnect is a new call
   whose first input is what was said during the old one.
3. Past `RING_BUFFER_SECS` the oldest audio is dropped — and **the drop is
   recorded**: the page keeps the dropped span's capture times and sends
   them with the backlog, so the late-lane turn says *"I lost about a
   minute around 12:22"*. Loss is named, never silent; "nothing arrived"
   and "nothing was said" stay opposite findings.

## 5. The downlink is the mirror, and comes second

The same call showed the other direction: the page reported the downlink
paused four times, and the worker currently believes it spoke every sentence
it generated. #226's own diagnosis found a clarifying question in a
transcript the owner never heard. The mirror design — TTS PCM over an
`audio-down` channel, played from the page's `AudioContext`, with the page
reporting play boundaries per sentence — gives three things: no reply is
lost to a stall, the echo window (`Pending::asked`, `_bot_audible_until`)
is grounded in what was *played* rather than what was generated, and the
playback constant the timing layer has waited on since 2026-09-03 becomes a
measurement. It is sequenced after the uplink because the uplink is where
the words are being lost, and because it reuses this design's channel and
reporting plumbing rather than inventing its own.

## 6. What is deliberately not in scope

- **Recognition on the phone** — Safari's `webkitSpeechRecognition` or a
  WASM model. The loss is in delivery, not recognition; moving recognition
  trades Parakeet for a worse model to solve a transport problem. Recorded
  as options B and C in the 2026-09-14 diagnosis, with their Safari caveats.
- **Tap-to-talk.** Declined by the owner on 2026-09-12; not re-pitched.
- **Compression.** PCM first. The failure is stalls, not throughput, and
  32 kB/s is comfortable on any link that is up at all. Opus over the same
  channel is the follow-up if a cellular uplink turns out to be
  throughput-bound between stalls.
- **ICE restart**, for the reason above.
- **Changing the RMS gate or the fragment timers.** Neither dropped anything
  on the morning call (RMS 0.013–0.045 against a 0.010 floor; no gate line
  logged). Calibration was the wrong diagnosis and is not touched.

## 7. How it is measured

`scripts/voice/test_call.py --uplink [--stall-at S --stall-secs D]` is the
repeatable stand-in: a headless aiortc client that sends the RTP track as
before *and* speaks the uplink protocol from the same wav, with a stall of
the caller's choosing, against any worker (`--offer`). First run,
2026-09-14, against the branch's worker on a second port: a 6 s stall two
seconds into a seven-second utterance, every word delivered, one turn, the
model answering both requests in it.

The acceptance shape is the 2026-09-14 morning call. On a stall-heavy call:

- **transcript coverage ≈ speech**: the worker logs `behind_ms` at every
  resume and the total late-delivered seconds per call; the page logs every
  dropped span. *Words lost* must be zero unless the page died (§4.1) or the
  ring overflowed (§4.2), and both of those are logged as what they are.
- **no turn ends on the wire's clock**: a probe like `test_turn_stop.py`
  drives a burst — 15 s of audio delivered in 2 s after a 15 s gap — and
  asserts one turn, ending on the audio's own silence.
- **behind is what is sounded**: the page's unit tests (`test_echo_filter.py`
  is the pattern) drive the ring through a stall and assert the cue fires at
  the threshold and the caught-up cue at drain, never at packet events.

The next real drive is the measurement. Serve logs at `warn`, so the worker's
journal remains the instrument; the numbers above go there.

## 8. Open, for the owner

- **`BACKLOG_TALK_SECS` = 120 s** — the point where live becomes late. Two
  minutes is a guess at where a conversation stops being one.
- **`RING_BUFFER_SECS` = 300 s** — how much of an outage is kept. Memory is
  not the constraint; whether five-minute-old speech is worth answering is.
- ~~Keep the muted RTP track~~ — resolved by the build: the track is kept
  and unmuted, because the receiver reports on it are the page's uplink
  witness. Nothing about iOS background capture changed.
- **The late-lane prefix wording** — *"(said 12:22–12:23, delivered late)"*
  — is what the model reads and what the transcript records; a listener
  hears only the answer.
