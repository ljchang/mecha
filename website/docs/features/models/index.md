---
title: Models and context
sidebar_position: 0
description: The model behind the loop, hosted or local, and how a long conversation is made to fit its window.
---

# Models and context

mecha runs the same agent loop over any model: a hosted one through
Anthropic's API, or an open-weight one on your own machine through an
OpenAI-compatible server. The loop never learns which provider is behind it.
Whichever you use, the conversation has to fit the model's context window.

| Page | Covers |
|---|---|
| [Providers](/docs/features/models/providers) | Configuring a provider, prompt caching, and which failures are retried. |
| [Serving a local model](/docs/features/models/serving) | Running llama-server: slots, what `-c` divides, and the numbers that must agree with your config. |
| [Compaction](/docs/features/models/compaction) | What happens when a conversation outgrows the window, and what survives it. |
