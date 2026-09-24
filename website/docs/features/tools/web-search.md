---
title: Web search
sidebar_position: 4
description: The web_search tool — a chain of search backends tried in order, why its results count as untrusted, and why it keeps working in a conversation that holds your mail.
---

# Web search

`web_search` lets a run look something up on the web. It is registered only
when at least one `[[search]]` backend is configured; with none, the tool does
not exist and the model cannot call it.

## Configure a chain of backends

```toml
[[search]]
kind = "searxng"                   # self-hosted: no key, no quota
base_url = "http://127.0.0.1:8888"

[[search]]
kind = "exa"
api_key_env = "EXA_API_KEY"
prefer_deep = true                 # try this one first for deep searches

[[search]]
kind = "tavily"
api_key_env = "TAVILY_API_KEY"
```

Three kinds are supported: `searxng` (your own instance), `exa` and `tavily`.
Backends form a **chain tried in order, and the first to answer wins**. If
one is rate-limited or down, the next answers, which is what makes stacking
two free tiers a working strategy.

The model asks for a `quick` or a `deep` search. `prefer_deep` moves a backend
to the front of the chain for deep searches only. It reorders and never
filters: a preferred backend that fails still falls through, and a quick
search still reaches it as a fallback. Every key is in the
[configuration reference](/docs/reference/configuration#search).

Two outcomes are kept apart. Backends that answered and found nothing return
*no results*, which is an answer. A chain where nothing answered is an error.
That includes a SearXNG instance whose upstream engines are all suspended,
which returns an empty page with HTTP 200.

## Results are untrusted

Search results are the largest source of third-party text an agent reads, and
anyone can put text on a page that ranks. So `web_search` declares
`untrusted_input`, and every result marks the conversation as holding
untrusted content. A backend that writes a summary gets no more trust than
its snippets, because the summary was written from the same pages.

## Why it keeps working next to your mail

The [security interlock](/docs/features/security) refuses a tool that can
send data to a destination **the model chooses** once a conversation holds
both private data and untrusted content. That is the combination an
injected instruction needs to send your data somewhere.

`web_search` is different in one respect that matters: its input is `query`,
`limit` and `depth`, with no destination field. The query goes to the
backends *you* configured and nowhere else. mecha calls this a **blind**
sender. An injection can shape the query, but it cannot choose who reads it.
So the interlock leaves `web_search` alone, and reading your mail does not end
web search for the rest of the conversation. The stricter
`block_sends_after_private` guard still refuses it, if you turn that on.

Blind is decided per backend **and per depth**, in code, never in config:

| Backend | `quick` | `deep` |
|---|---|---|
| `searxng` | blind | blind |
| `tavily` | blind | blind |
| `exa` | blind | not blind: deep search is agentic research that fetches pages the query can steer it to |
| any backend added later | not blind until someone reads its API and says so in code | same |

In a conversation holding both private data and untrusted content, a search is
served only by blind backends, at `quick` depth, and the result says so. **So
configure at least one blind backend**, or web search stops working in those
conversations. With none configured, the tool is treated as choosing its
destination, and its refusal names `kind = "searxng"` as the fix.

One thing blind does not change: the query still leaves your machine. Even a
self-hosted SearXNG forwards it to upstream engines. Blind removes the
attacker's choice of reader, not the fact that the query is sent.

## Related

- [Tools and MCP](/docs/features/tools): the other built-in tools, and how
  capabilities are declared.
- [Security model](/docs/features/security): the interlock and the send
  classes (`none`, `blind`, `chosen`).
