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
served only by blind backends, at `quick` depth, and the result says so —
unless `trifecta = "allow"`, which waives this narrowing along with the
interlock. **So
configure at least one blind backend**, or web search stops working in those
conversations. With none configured, the tool is treated as choosing its
destination, and its refusal names `kind = "searxng"` as the fix.

One thing blind does not change: the query still leaves your machine. Even a
self-hosted SearXNG forwards it to upstream engines. Blind removes the
attacker's choice of reader, not the fact that the query is sent.

## Opening a result

Each result comes back with a handle in brackets:

```text
1. [a3f-9c01de.1] Specific aims: a worked example
   https://example.org/aims
```

`web_open` takes that handle and reads the full page. Its only argument is
the handle, never a URL. The page it fetches is therefore one a search
already returned, and the model picks among results rather than writing an
address. That makes `web_open` blind by the same argument as `web_search`,
so it keeps working in a conversation that has read your mail, where
`http_fetch` is refused.

- **What it leaks** is which result was picked, to whoever serves that page.
  An injection can steer the pick, but it cannot pick a reader that wasn't in
  the results.
- **Redirects are followed**, because the page's server chooses the target,
  not the model. Every hop is vetted the way `http_fetch` vets its one:
  private and link-local addresses are refused, and blocked domains stay
  blocked.
- **Handles are random per search** and do not survive a restart, so an old
  handle is refused. The fix is to search again.
- **An approval prompt shows the URL** beside the handle, because a handle
  alone tells you nothing about the page.
- **A URL from anywhere else** still goes through `http_fetch`, and is refused
  once the conversation holds both private data and untrusted content. That
  includes a link in an email, one you pasted, and one the model made up.

`web_open` is registered with `web_search` and not without it.

## Related

- [Tools and MCP](/docs/features/tools): the other built-in tools, and how
  capabilities are declared.
- [Security model](/docs/features/security): the interlock and the send
  classes (`none`, `blind`, `chosen`).
