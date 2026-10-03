"""Pitch-preserving time stretch shared by the TTS servers.

Chatterbox has no speed parameter and Breeze's server parses one and ignores
it, so both apply speed here, after synthesis. Numpy only, so it runs in the
voice-worker venv as well as the Chatterbox image.
"""
import numpy as np


def stretch(samples: np.ndarray, speed: float, sr: int = 24000,
            frame_ms: float = 30.0, search_ms: float = 10.0) -> np.ndarray:
    """Pitch-preserving time stretch by WSOLA. speed > 1 is faster.

    Was `librosa.effects.time_stretch`, which is an STFT phase vocoder -
    the classic source of the smeared, phasey quality on speech, and
    audible enough on this path that the speed control was unusable. WSOLA
    (waveform-similarity overlap-add) instead searches for the splice point
    whose waveform best continues the previous frame, so periodicity
    survives the cut. Judged by ear against ffmpeg's `atempo` (a WSOLA
    implementation) and librosa on identical source audio.

    Hand-rolled rather than shelling out: the serving container has no
    ffmpeg and no `torchaudio.sox_effects`, and the image is built ad-hoc
    with no Dockerfile in the repo, so a dependency here is a build step
    nothing tracks. Forty lines of numpy is the cheaper thing to own.

    Note this is a *repair*, applied after synthesis. Pace is really a
    property of the reference clip - Chatterbox clones speaking style - so
    a voice meant to talk faster is better built as its own reference
    (`vctk_p276_brisk`) than stretched on every utterance.
    """
    if abs(speed - 1.0) < 0.01 or samples.size == 0:
        return samples
    x = np.asarray(samples, dtype=np.float32)
    N = int(sr * frame_ms / 1000.0) & ~1          # frame, even
    Hs = N // 2                                    # synthesis hop, 50% overlap
    Ha = max(1, int(round(Hs * speed)))            # analysis hop
    delta = int(sr * search_ms / 1000.0)           # similarity search radius
    if x.size < N + 2 * delta:
        return x
    win = np.hanning(N).astype(np.float32)
    frames = max(1, int((x.size - N) / Ha))
    out = np.zeros(frames * Hs + N, dtype=np.float32)
    norm = np.zeros_like(out)

    prev = 0          # start of the frame actually used last time
    for k in range(frames + 1):
        # The nominal analysis position advances by exactly Ha and is
        # computed from k rather than accumulated. That is the whole
        # correctness of the tempo: chaining it off each *adjusted*
        # position lets the local search drift, and since the perfect
        # continuation sits at prev+Hs - inside the search window whenever
        # delta exceeds Ha-Hs, which at 1.2x is 10ms against 3ms - the
        # drift is not random. It walks at the synthesis hop, scales
        # nothing, and the result is the input played at normal speed and
        # cut off where the shorter output buffer ends.
        nominal = k * Ha
        lo = max(0, nominal - delta)
        hi = min(x.size - N, nominal + delta)
        if hi < lo:
            break
        if k == 0:
            ana = 0
        else:
            # What the ear expects next if nothing were cut, found in the
            # neighbourhood of where the tempo says to look. Choosing the
            # best-correlating offset keeps pitch periods aligned across
            # the splice, which is what a phase vocoder cannot do.
            target = x[prev + Hs:prev + Hs + N]
            if target.size < N:
                break
            cand = np.lib.stride_tricks.sliding_window_view(x[lo:hi + N], N)
            energy = np.sqrt(np.einsum("ij,ij->i", cand, cand)) + 1e-9
            ana = lo + int(np.argmax((cand @ target) / energy))
        syn = k * Hs
        out[syn:syn + N] += x[ana:ana + N] * win
        norm[syn:syn + N] += win
        prev = ana

    return out / np.maximum(norm, 1e-6)
