---
title: Images
sidebar_position: 4
description: How a screenshot reaches the model, why a multimodal model is two files, and what the caps are for.
---

# Images

Send the agent a screenshot and ask what is wrong with it. Four doors reach
that, and which ones are open to you depends on where you are sitting and on
whether the model you are serving has had its eyes plugged in — which, if you
run a local model, is very likely the part that is missing.

## A vision model is two files

This is the whole trap, so it comes first.

The weights you downloaded hold the **language model**. The vision tower ships
as a **separate `mmproj-*.gguf`** in the same repository, and `llama-server`
loads it only if `--mmproj` names it. Without it the server starts, answers
well, reports `modalities.vision: false`, and the model tells anyone who sends
it a picture that it cannot see images — which reads as a limitation of the
weights rather than a flag nobody passed.

Three things conspire to keep it hidden:

- **`--mmproj-auto` is enabled by default**, so the flag list looks handled. It
  only fires for `-hf` downloads. If you start the server with `-m <path>` —
  which every example here does — the default does nothing.
- **`/props` answers a different question than the one you are asking.**
  `modalities.vision` reports what is *loaded*, never what the architecture
  supports, so a multimodal model with no projector is indistinguishable from
  a text-only one.
- **Nothing fails.** There is no error to find.

### Is my model multimodal?

Three tells in the GGUF metadata, none of which needs the model loaded:

| Key | Multimodal |
|---|---|
| `general.tags` | contains `image-text-to-text` |
| `<arch>.rope.dimension_sections` | present — mRoPE, e.g. `[11, 11, 10, 0]`. A text-only model has none. |
| `tokenizer.chat_template` | handles `image_url` items and emits `<\|vision_start\|>` |

If those say yes and `/props` says `vision: false`, the projector is what you
are missing. Fetch it from the same repository as the weights:

```bash
curl -L -o "$SNAPSHOT/mmproj-BF16.gguf" \
  "https://huggingface.co/<org>/<repo>/resolve/main/mmproj-BF16.gguf"
```

then start the server with it:

```bash
llama-server -m "$MODEL" --mmproj "$SNAPSHOT/mmproj-BF16.gguf" ...
```

`BF16` and `F16` are the same size and either is fine; `F32` doubles the
memory for a tower whose precision is not the bottleneck.

## Telling mecha the model can see

A loaded projector changes what the *server* can do. It does not change what
mecha *sends*:

```toml
[providers.local]
vision = true
```

Unset means `true` for `kind = "anthropic"` — every Claude model in the family
sees — and `false` everywhere else, which is the safe direction for a local
server: an `image_url` part sent to a text-only endpoint is a failed request,
where an image rendered as text is merely a model that cannot see, which is
what it already was.

Do not type this by hand. `mecha setup` compares the config against `/props`
and warns in **both** directions, because they fail differently — declared but
not served silently degrades every image to a line of text, and served but not
declared means a projector is loaded, paid for in memory, and never used:

```bash
mecha setup            # what disagrees
mecha setup --write    # rewrite model, context_window and vision from /props
```

The same check runs at startup on every command, so a mismatch tells you the
next time you use mecha at all.

## The four doors

| Where you are | How |
|---|---|
| A local terminal | **Drop the file on the TUI prompt** |
| Anywhere, scripted | `mecha run --image shot.png "what is wrong here?"` |
| Away from the machine | Send it to the Slack DM, or into a `/remote-control` thread |

### Dropping a file on the prompt

A terminal turns a drop into a *paste of the path*, so this is bracketed paste
rather than any drag-and-drop protocol. When every token of a paste resolves to
an existing image, mecha replaces it with a chip:

```
› what error is on this screen? [image: shot.png]
  ⇄ 1 image attached · 179 KB
```

The chip is the handle: **delete it and the image is not sent.** That is the
only undo there is, because the bytes are held beside the input where backspace
cannot reach them.

Requiring the *whole* paste to be paths is a safety property, not a
convenience — a paste is also a paragraph you copied off a web page, and a rule
that attached any file whose path appeared somewhere in pasted prose would let
copied text pull bytes off your disk into a request. A non-image file inserts
its path unchanged, which is what a dropped `.csv` wants: `fs_read` can read it.

:::warning This cannot work over SSH
The path your terminal pastes is the path on **your laptop**, and mecha
resolves it on the machine at the other end, where it does not exist. The bytes
never left the laptop. Nothing can fix that, and it is why the Slack conduit
exists — send the screenshot to the DM instead.
:::

### From Slack

The connector downloads the file into the run's workspace under `inbox/` and
puts the image on the turn. You get both: pixels the model can look at, and a
path it can `shell` or `fs_read`. In a `/remote-control` thread the same
happens, except the TUI that owns the session is what moves the file into its
workspace.

## What it costs

Images are capped **at the door** rather than per turn, because the transcript
is append-only and every turn resends the whole history — so a resize is paid
once and collected on every turn afterwards. An image already within the caps
passes through **byte for byte**: re-encoding a crisp screenshot of text is a
real loss, and that is the case this is most often used for.

Measured on a 2222x1548 photo of a laptop screen:

| | raw | after the caps |
|---|---|---|
| file | 5.7 MB PNG | 179 KB JPEG, 1568px long edge |
| `prompt_tokens` | **294** | **294** |

**The token cost is identical**, because the server tiles the image to a fixed
count before the model sees it. So the resize buys nothing in context and 32x
on the wire and in the session file — which matters more than it sounds: one
un-resized screenshot was **99% of a session transcript**, re-sent whole on
every subsequent turn.

The caps are 1568px on the long edge and 5 MB encoded. Five is Anthropic's hard
per-image limit and is applied to local servers too, because a conversation is
one object and a `/model` switch must not turn a working transcript into a
rejected request.

:::tip `max_tokens` bites sooner with images
Vision prompts reason longer. A local server with `--reasoning-budget` set will
happily spend the whole `max_tokens` allowance on thinking and return HTTP 200
with empty `content` — measured at `max_tokens: 300` on an image that answered
fine at 1200. Keep `max_tokens` comfortably above the budget.
:::

## What an image does to the interlock

An attached image arms the **private** leg of the taint, exactly as reading a
file with `fs_read` would. Typed text does not, and the difference is
deliberate: **a screenshot is captured, not composed.** You choose every word
you type; you choose the window, not everything in it — which is most of why
people screenshot instead of retyping.

So a session holding a screenshot *and* third-party content — a fetched page, a
mail body, a graph read — will refuse an outbound tool, which is
[the interlock](/docs/features/security) doing its job.

## Limits

- **Tool results cannot carry images.** Anthropic accepts one inside a
  `tool_result`; the OpenAI dialect's `role: "tool"` messages carry a string and
  nothing else. A tool returning pixels would work on one backend and silently
  lose them on the other, so images ride the user turn only — which means "look
  at the chart you just made" is not available.
- **An image cannot join a run already in flight.** Steering carries text. A
  file attached mid-run still lands on disk and still has its path named, but
  the pixels wait for the next turn.
