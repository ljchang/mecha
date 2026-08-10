# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security vulnerability.

Report it through GitHub's private vulnerability reporting: go to the
[Security tab](https://github.com/ljchang/mecha/security/advisories/new) of this
repository and open a draft advisory. That keeps the report private to the
maintainers until a fix is available.

Please include what you can: the version or commit, a description of the
problem, and the smallest reproduction you can manage. If the issue is in the
security model rather than in a single function — a way around the path jail or
the trifecta interlock, for instance — a transcript demonstrating it is worth
more than a description.

Expect an acknowledgement within a week. This is a research project maintained
by one person, so please calibrate accordingly; there is no paid response
commitment behind it.

## Supported versions

The project is pre-1.0. Only the latest release on `main` receives fixes.

## What mecha assumes

mecha runs a language model in a loop with tools that read files, execute
commands and reach the network. That is inherently a large amount of authority,
and the design assumes the model may be adversarially steered by content it
reads. Three mitigations are enforced structurally rather than by prompting.

**The path jail.** Every model-supplied path is canonicalized and proven to be
inside the workspace before any filesystem call. A path escape is a
vulnerability; please report it.

**The trifecta interlock.** Tools declare capabilities, and the loop refuses any
tool that can send data outward once both private data and untrusted input have
entered the conversation. Taint is a property of the conversation, not of a
single run, and survives both compaction and session resume. A way to launder
taint, or to reach an `external_send` tool with the interlock armed, is a
vulnerability.

The interlock deliberately sits *ahead* of the human approver, because a person
clicking "yes" is precisely what a prompt injection is trying to engineer.

**The front-door quarantine.** Requests that arrive from strangers carry free
text, which is the one place someone outside controls the bytes. That text is
typed by an extractor issued a request with an empty tool list and a single user
message — not one instructed to avoid tools — and the privileged run receives
only the typed extraction, through a function with no argument that returns the
prose. The extractor's own paraphrase stays behind too, because a paraphrase of
an injection is the injection rearranged. An extraction failure parks the record
for a human; it never falls back to passing the prose through. A way to get a
stranger's free text in front of a privileged run is a vulnerability.

## Known limitations

These are documented rather than fixed, and are not vulnerabilities on their own.
A way to exploit one *past* its stated mitigation is.

**`shell` is not treated as an untrusted source.** Taint tracking cannot see
inside a command, so a command that fetches a hostile page does not arm the
interlock the way `http_fetch` does. The mitigation is the sandbox: a confined
shell has no network, and `[sandbox] kind` should be set to `bwrap` or `docker`
in any configuration where this matters. `kind = "none"` is the default and runs
commands as you.

**A configured sandbox that does not work stops the run.** This is intentional.
Falling back to unconfined execution would be worse than having no sandbox,
because `shell` declares narrower capabilities when confined and the interlock
believes it.

**MCP servers are third-party code running on your machine.** They receive a
cleared environment plus a named allowlist, never your inherited environment, so
a server cannot read your provider keys unless you pass them deliberately. A
server that cannot be confined while `sandbox = true` is a startup error rather
than a warning.

**The Slack remote control is a network ingress path.** When the connector is
running, messages from a linked workspace start agent runs. It dials out over
Socket Mode rather than accepting inbound connections, an owner is bound by a
one-time code, and `[slack]` is stripped from project config layers so a cloned
repository cannot name an owner. Each thread is its own conversation, so taint
does not cross between threads — but anything a workspace member can say to the
bot reaches a run. Treat workspace membership as the trust boundary it is. Note
also that MCP tools do not honour the per-thread jail; only the built-in tools
do, because servers are spawned once with the agent.

**Learned rules ride in every future prompt.** That is a longer-half-life
injection path than anything the interlock guards, so reflections carry a
provenance origin and `mecha learn` structurally excludes anything not
classified `clean`. Classification fails closed and there is deliberately no
override. A way to get untrusted text classified as clean is a vulnerability.

## Scope

Out of scope: the behaviour of any particular language model, including a model
choosing to do something unhelpful within the permissions it was granted; and
anything requiring a configuration the documentation warns against.
