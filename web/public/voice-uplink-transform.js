/* The uplink tap: every encoded audio frame the browser is about to send
   over RTP is also handed to the page, which buffers it and ships it over
   the data channel — reliable, ordered — so an unstable link delays the
   owner's words instead of losing them (docs/VOICE-LINK-DESIGN.md §2).

   A separate file rather than a blob URL because `mecha serve`'s CSP is
   `script-src 'self'`, and a worker must come from the origin. It runs as
   an `RTCRtpScriptTransform` worker (Safari 15.4+, Firefox, Chrome — a
   Baseline feature since late 2025): `onrtctransform` fires once per
   sender the page attaches it to.

   The frame is *copied* before it is passed on, never consumed: RTP still
   carries it, so the far end's receiver reports — the page's only witness
   for an uplink stall — keep flowing, and a browser without this API
   simply sends audio the old way. `timestamp` is the RTP timestamp at the
   codec clock (48 kHz for Opus); the page turns deltas into its media clock.
   Tapping the *encoded* stream is deliberate: attaching WebAudio to a
   microphone track silently disables echo cancellation in WebKit (the page
   records the incident), and this path touches no track at all. */
/* Said first, so the page can prove this file loaded before it declares
   the channel in the offer: a 404 or a parse error surfaces only as an
   asynchronous error, and a tap that attached but never delivers is a
   call nobody can hear (review of #231). */
self.postMessage({ ready: true });

onrtctransform = (event) => {
  const { readable, writable } = event.transformer;
  const tap = new TransformStream({
    transform(frame, controller) {
      try {
        const copy = frame.data.slice(0);
        self.postMessage({ ts: frame.timestamp, data: copy }, [copy]);
      } catch { /* a frame that cannot be copied is still sent */ }
      controller.enqueue(frame);
    },
  });
  readable.pipeThrough(tap).pipeTo(writable);
};
