# Canvas — how an agent gets a seat in the course

*2026-08-21. The survey behind "wire mecha into Canvas so it can post
announcements, maintain course pages, and help with grading". Six web-research
passes over Instructure's API docs, Dartmouth's own service catalogue, three
third-party MCP servers, and the empirical literature on grading injection.
Claims carry their sources. The conclusion changed twice during the pass — once
on reading what Dartmouth did in June 2025, and once on reading the Wharton
grading-injection numbers, which are the reason §6.2 exists.*

## 0. The short answer

Canvas has a large, mature REST API and everything asked for is reachable
through it. The integration is not hard. Two things about it are.

**Build it as a fifth binary on `mecha-mail`, authenticated by OAuth2 over
`urn:ietf:wg:oauth:2.0:oob`, with a hand-picked surface of about fourteen
tools.** Not a built-in core tool, not a third-party MCP server, and not the
manual access token — even though the manual token is the path of least
resistance and the one every existing integration takes.

The two hard things:

- **A Canvas user token is not a scope, it is a login.** `drive.file` made the
  documents boundary provable by reading a string (`DOCS-RESEARCH.md` §0). There
  is no equivalent here: a manual token carries *every permission its user has*,
  which for a professor means rewriting any grade, deleting any assignment, and
  emailing every enrolled student. The boundary can only be the tool surface,
  the way `mecha-docs` has no sharing verb and no permanent-delete verb — and
  the developer-key path is the only one that can back that up with an
  enforcement Canvas itself applies.
- **Student submissions are the most adversarial text this project has ever
  planned to read.** §6.2 has the numbers. The decision recorded there —
  **assistive, never evaluative, with completion checking as the one
  exception** — is what makes the rest of this affordable, and it dissolves the
  local-versus-frontier tension rather than resolving it.

## 1. What is being asked for

Three verbs, and they are not equally priced:

- **Post** — announcements, pages, assignment shells, files, calendar events.
  Cheap, well-covered, and the write that reaches students the instant it lands.
- **Update** — course settings, module ordering, due dates, page bodies. Cheap
  and mostly consequence-free while a thing is unpublished.
- **Grade** — `posted_grade`, submission comments, rubric assessments. The
  expensive one, and not because the endpoint is hard. It is one POST.

The asymmetry is worth stating early: the API cost of grading is trivial and the
*correctness* cost is the whole design. Everything in §6 is downstream of that.

## 2. The API

### 2.1 REST

`/api/v1/`, bearer token, JSON. Instructure documents 190+ resource groups; the
ones that matter here:

| Resource | Endpoint root | What it is |
|---|---|---|
| Courses | `/courses` | list, settings, `?include[]=` for term/teachers |
| Assignments | `/courses/:id/assignments` | full CRUD, due dates, overrides |
| Submissions | `/courses/:id/assignments/:aid/submissions` | the student's work, `?include[]=submission_comments,rubric_assessment` |
| Grading | `PUT .../submissions/:user_id` | `submission[posted_grade]`, `comment[text_comment]`, `rubric_assessment[...]` |
| Announcements | `/courses/:id/discussion_topics` with `is_announcement` | reaches every student immediately |
| Discussions | `/courses/:id/discussion_topics` | third-party text |
| Pages | `/courses/:id/pages` | wiki bodies, revision history |
| Modules | `/courses/:id/modules` | structure and ordering |
| Files | `/courses/:id/files` | three-step upload, §2.3 |
| Enrollments | `/courses/:id/enrollments` | the roster, i.e. the FERPA payload |
| Conversations | `/conversations` | Canvas inbox; **mails students** |
| Calendar | `/calendar_events` | overlaps what `mecha-mail` already does |

The grading write is the one to look at closely, because it is a single call
that does three separable things:

```
PUT /api/v1/courses/:course_id/assignments/:id/submissions/:user_id
    submission[posted_grade]=92
    comment[text_comment]=...
    rubric_assessment[criterion_id][points]=...
```

A tool surface that exposes this verbatim is one tool that grades, comments and
assesses. §7 splits it, on the `TriageAction` reasoning from
`SLACK-ACTIONS-DESIGN.md` §1: a free-form argument that changes what kind of
thing an action *is* hides the dangerous case inside a verb that reads as
harmless.

### 2.2 GraphQL

`/api/graphql`, same OAuth2 bearer tokens, same permissions, GraphiQL at
`/graphiql`. Instructure says new features are developed in GraphQL first, and
that it does not yet cover everything REST does. SpeedGrader itself is built on
it — one query pulls submissions with comments, rubric assessments and media
objects, where REST needs several round trips.

**Use REST anyway, for now.** Partial coverage means a GraphQL-first client
carries a REST client too, which is two clients; and the round-trip saving is
irrelevant at this volume — a course has forty students, not forty thousand.

And the choice may be made for us, in a way that turns out to be load-bearing:
**a developer key with scopes enforced cannot use GraphQL at all.** Instructure
does not document why, and it is a reported and persistent behaviour rather than
a stated policy. §6.2 depends on it, so it is the first thing to verify once a
key exists — see §8.

### 2.3 The mechanical facts

Three, all of which `mecha-mail`'s `http.rs` already has the shape for:

- **Throttling is a leaky bucket.** Every response carries `X-Request-Cost` and
  `X-Rate-Limit-Remaining`; exhaustion is a 403 (documented, confusingly, in
  places as 429). Instructure's own guidance: *a client that makes no more than
  one simultaneous request is unlikely to be throttled*, and parallel requests
  take a pre-flight penalty that is refunded on completion. So: serial, and
  back off on the remaining-quota header rather than waiting for the refusal.
- **Pagination is `Link` headers, and they are opaque.** Absolute URLs carrying
  every parameter needed. Two rules from the docs: never construct the next page
  yourself, and **never parse the header case-sensitively** — HTTP header names
  are case-insensitive and Canvas is not consistent.
- **File upload is three steps.** POST metadata to get an `upload_url` plus
  `upload_params`; multipart POST to that URL with the file *last* and **no
  access token**; then follow a 3xx with an authenticated GET, or take a 201 as
  done. Endpoint choice determines permissions — only files posted to the
  submission-comments endpoint can be attached to a submission comment.

## 3. Auth — the whole problem

### 3.1 Three tiers

| | How obtained | Scoped? | Expiry | Multi-user? |
|---|---|---|---|---|
| Manual user token | User profile settings | **No** — full user permissions | Long-lived | Policy violation |
| OAuth2 developer key | Root account admin issues client id/secret | **Yes**, per endpoint | 1h access + refresh | Yes |
| LTI | Tool installed at account level | LTI-defined | Per launch | Yes |

Instructure's policy on the first is explicit: manual tokens are *for testing*,
and "asking any other user to manually generate a token and enter it into your
application is a violation of Canvas' API Policy."

### 3.2 What Dartmouth did

**On 2025-06-18 Dartmouth decommissioned self-service manual token generation.**
Tokens are now issued by an approved Canvas administrator through a service
request form, roughly five business days, and:

> At this time we will not be granting user access tokens to students.

Their stated reasoning is FERPA-covered educational records and IP, and they say
plainly that they would rather vendors use LTI than collect user tokens.

**Probe, 2026-08-21.** Checked against the live account rather than taken from
the announcement: on `canvas.dartmouth.edu/profile/settings`, under *Approved
Integrations*, the **`+ New Access Token` button is greyed out** for a faculty
account. So the decommission was not students-only — there is no self-service
path for anyone, and the service request form is the sole door. Worth having
measured: the announcement said the service was decommissioned, which is
compatible with faculty being exempted, and that would have changed the plan.

Two consequences, and the second is the useful one:

- The frictionless path is closed, so *some* institutional conversation is
  happening either way. Given that, it should be the conversation that produces
  the better credential.
- Dartmouth says it wants to support "internal development and customization"
  and will "partner with Dartmouth to do that securely." A single professor
  asking for a scoped developer key for a tool that touches only his own courses
  is the case they describe wanting. Asking for the *narrower* thing is also
  the easier ask.

### 3.3 The scope problem, stated precisely

This is where the `mecha-docs` precedent inverts and it is worth being exact
about why.

`drive.file` is a boundary that survives every future diff: a document nobody
handed mecha is unreachable, no instruction inside a run can widen that, and the
proof is a scope string. The reviewer's job is to read one line of config.

A Canvas **manual token** offers nothing of the kind. It is the user, over HTTP.
Everything a professor can do in the web UI, the token can do — including the
things there is no tool for, because the boundary is the surface *this* client
happens to expose, and a surface is only a boundary while nobody adds to it.
That is exactly the "remember not to include the free text" shape the front door
rejects (`frontdoor.rs`, `Record::for_privileged_run`): it holds until the first
person in a hurry.

A **developer key** restores the property. Root account admins scope keys to a
subset of endpoints, written as `url:POST|/api/v1/conversations` and the like,
and Canvas enforces them server-side. That means "mecha cannot delete an
assignment" becomes true the way `drive.file` is true — provable by reading a
string, checked by somebody else's code — rather than true because our tool list
is currently short.

**That is the whole argument for OAuth here, and it is not about token
hygiene.** It is the same argument that made `drive.file` worth the awkward
picker flow.

### 3.4 There is no OAuth without a developer key

Worth stating flatly, because it is the first thing anyone asks and the answer
is counter-intuitive: **the developer key *is* the OAuth credential.**

> Developer keys are OAuth2 client ID and secret pairs stored in Canvas that
> allow third-party applications to request access to Canvas API endpoints via
> the OAuth2 flow.

`GET /login/oauth2/auth` resolves the developer key behind the `client_id` it is
handed, so there is no flow to start without one. And a key cannot be
self-issued:

> Developer keys created in a root account, by root account administrators or
> Instructure employees, are only functional for the account they are created in
> and its sub-accounts.

Root account admin or Instructure — being teacher of record on every affected
course does not reach it. Keys are bound to the issuing institution too, so a key
from a free `canvas.instructure.com` teacher account cannot authorize against
`canvas.dartmouth.edu`.

So "just use OAuth instead of asking Dartmouth" is not a door. There are exactly
two, and both go through the same request:

| | The ask | Enforcement | Cost to them |
|---|---|---|---|
| Manual token | a bearer string | none — full user permissions | the form already exists |
| Developer key | client id + secret + scope list | server-side, per endpoint | bespoke; admin configures scopes |

**Ask for both in one submission**, leading with the scoped key and naming the
manual token as an acceptable fallback. The key is the better credential but a
bespoke request against a process Dartmouth may not have, and a five-day wait
that returns "no" leaves nothing. One form, one wait, no empty-handed outcome.

**Ask for "Allow Include Parameters" to be enabled on the key.** A scoped token
does not *reject* `include[]` parameters — it **silently ignores them** unless
that option is on. So `?include[]=submission_comments` returns a submission with
no comments and a 200, which is this project's recurring failure shape: a
component that stopped working reading as one that found nothing. Cheap to ask
for up front, invisible and confusing to diagnose later.

Note for that conversation: Instructure's "manual tokens are for testing only"
line targets **multi-user applications collecting other people's tokens** — that
is the stated API-policy violation. One person holding their own token for their
own courses is not that, and Dartmouth's own page describes wanting to support
exactly it. The fallback is legitimate rather than a workaround; the developer
key's advantage is narrower and specific — Canvas enforces the scopes, so §7's
surface restrictions stop being the *only* boundary.

### 3.5 The flow: OOB, and it is better positioned than Google's

Canvas's native-app answer is the one Google took away.

> For native applications, currently `urn:ietf:wg:oauth:2.0:oob` is the only
> supported value, which signifies that the credentials will be retrieved
> out-of-band using an embedded browser or other functionality.

Canvas redirects to a page whose query string carries `code=<code>`; the user
copies it back. That is **exactly `mecha-docs --paste`** (`DOCS-RESEARCH.md`),
arrived at from the opposite direction: there it was a workaround for a
device-code flow that refuses Desktop clients, here it is the documented and
only native path. Same properties, and they are the ones that matter on this
box — no loopback listener, no forwarded port, no browser on the machine holding
the grant, works over SSH.

The rest is machinery `token.rs` already has, in the same shape:

| | Google (`mecha-docs`) | Canvas |
|---|---|---|
| Grant surface | loopback + picker, or `--paste` | `GET /login/oauth2/auth` → OOB paste |
| Access token life | 1h | 1h |
| Refresh | refresh token, reusable | refresh token, reusable, no documented expiry |
| Secret at refresh | yes | **yes** — `client_secret` required on both grant types |
| Revoke | Google endpoint | `DELETE /login/oauth2/token` |

Two Canvas-specific notes worth writing down before they cost something:

- **`client_secret` is required on the refresh call.** Microsoft taught the
  opposite lesson — a device-code public client that 400s with `AADSTS7000215`
  if you send one (`ARCHITECTURE.md`, mecha-mail). Canvas is a confidential client and
  wants it on both `authorization_code` and `refresh_token`. The two providers
  disagree, so this cannot be shared code without a flag.
- **`purpose` is a real parameter** and shows in the user's token list. Set it
  to `mecha`, so a professor auditing their own authorizations can see what this
  is. Cheap, and it is the kind of thing nobody adds later.

**Fallback if Dartmouth will only issue a manual token:** take it, and store it
through the same `StoredCredentials` path with `expires_at: None`, so the client
is written once against both. But treat the surface restrictions in §7 as
load-bearing rather than tidy, because in that configuration they are the only
boundary there is.

### 3.6 Where credentials live

`~/.mecha/canvas/<account>/oauth.json`, mode 0600. **Its own root**, on the rule
`mecha-docs` established the hard way: doctor globs `~/.mecha/mail/*/` and reads
each `oauth.json` *as a mail grant*, so a Canvas credential parked there gets
reported as a broken mail account. Share the type, never the namespace.

`accounts.toml` maps a short name to a Canvas host, because a professor with a
Dartmouth course site and a workshop on `*.instructure.com` is one person with
two institutions, and a developer key is scoped to the institution that issued
it. Same design as mail: **the model names an account, never a host**, and the
account names are baked into the tool schemas as an enum at startup.

## 4. Prior art: how others structure the tool surface

Three mature MCP servers, surveyed for surface shape rather than as candidates.

| Server | Tools | Grouping | Notable |
|---|---|---|---|
| `DMontgomery40/mcp-canvas-lms` | 54 | student (33) / instructor (13) / account (7) | stdio + streamable-http; names prompt injection in its README |
| `r-huijts/canvas-mcp` | 60 | 9 domains: courses, assignments, submissions, rubrics, modules, pages, quizzes, sections, eportfolios | **MCP annotations used properly**; pseudonymizes student names by default |
| `plyght/canvas-mcp` | 31 | flat | smallest surface |

### 4.1 What they get right

- **`r-huijts` annotates.** `readOnlyHint` on queries, `destructiveHint` on
  deletes. That is the interface mecha's interlock actually reads, and it being
  populated at all is better than most MCP servers manage.
- **`r-huijts` pseudonymizes student names and emails by default**, with real
  data opt-in. Independently arrived at, and it is the right instinct: the model
  usually needs *which submission*, not *whose*. Worth stealing — see §7.
- **Everyone splits `list_`/`get_` from `create_`/`update_`.** Naming carries
  the read/write distinction even where nothing enforces it.

### 4.2 What they get wrong, for this harness

- **Nobody declares `openWorldHint` on grading or announcements.** They are
  read/write-annotated at best. In mecha's model a posted grade and a class
  announcement reach third parties and are exfiltration-shaped, which is what
  routes them to the outbox; a surface that does not say so lands them in the
  wrong quadrant silently. This is the `DOCS-RESEARCH.md` observation verbatim —
  a third-party server decides how much the interlock distrusts it, and gets it
  wrong in the dangerous direction.
- **The surfaces are 31–60 tools.** The tool list is the front of the cached
  prefix (`CLAUDE.md`, Conventions). Sixty Canvas tools is a permanent per-turn
  tax for a surface where a working set is about fourteen, and it is paid on
  every run including the ones that never mention a course.
- **Student tools and instructor tools in one server.** `canvas_submit_assignment`
  next to `canvas_submit_grade` is a surface where the model can turn in
  coursework. Irrelevant to a professor, and not harmless: it widens what a
  compromised run can do for no benefit.
- **Account-level tools** (`canvas_create_user`, account reports) have no
  business in a personal assistant.

## 5. Native, MCP, or ours?

Three options, and the question in the CLAUDE.md sense is *which invariant does
each make checkable*.

**(a) Built into `mecha-core` as a built-in tool.** No. The invariant that
forbids it is already written down: "no mecha-core or mecha-cli code knows
Google or Microsoft exists, and neither does the model." Core must not learn
what a course is, for the same reason `agent.rs` must not learn which provider
is behind a request. The one thing built-ins buy that MCP does not — the per-run
path jail, `carried_state`, `narrows_surface_to` — is irrelevant to a surface
that touches the filesystem only to upload an attachment.

**(b) A third-party MCP server, config-only.** This is what I would have
recommended as a first step, and reading `config.rs` killed it. The override
that lets config distrust a server further is declared **per server, not per
tool**:

```rust
/// Capabilities forced onto every tool this server exposes, on top of
/// whatever it declares for itself.
pub capabilities: CapabilityOverride,
```

So to make `grade_submission` an `external_send` sink — which it must be — you
set `external_send = true` on the server, and **every read becomes a send sink
too**. `list_courses` then trips the interlock the moment anything untrusted is
in the conversation, which for this integration is *the moment it reads a single
student submission*. The feature disables itself on first contact with its own
data.

There is no per-tool escape: `[outbox] tools` routes by name and would stage the
right calls, but staging is downstream of the capability declaration, not a
substitute for it. The server would have to annotate correctly itself — which
returns the trust decision to third-party code holding a credential that can
rewrite grades.

**(c) A fifth binary on `mecha-mail`. This is the answer.**

- Capabilities are declared per tool, in our code, where they can be tested —
  the `assert_tool_surface` pattern that already exists once per mail surface.
- It reuses `http.rs` (retry/backoff on 429/5xx), `token.rs` (refresh under a
  lock, one forced retry on 401), and `accounts.rs` (named accounts baked into
  schemas as an enum). The OOB flow is a variant of a flow already written.
- The MCP boundary still buys the sandbox and the env allowlist, and it is
  *our* code inside them.
- The surface is ours to keep at fourteen tools.

**Not a fifth crate**, on the rule that a crate exists to make an invariant
checkable in `Cargo.toml` (the `mecha-slack` rule). This one would enforce
nothing and would need `token.rs` anyway. The cost is that `mecha-mail` now
means "personal-context providers", which is naming debt the crate list already
acknowledges for `mecha-docs`.

## 6. Where it lands in the security model

### 6.1 Three quadrants, again

The pattern from `mecha-docs` and `mail_triage` transfers cleanly.

**Reads → `untrusted_input` forced, `readOnlyHint`, never `openWorldHint`.**
A query travels only to `canvas.dartmouth.edu`, which already custodies every
byte it returns, so it is not an exfiltration channel — the same distinction
that separates mail search from `http_fetch`. But the content is other people's
words in the strongest sense available, so config forces `untrusted_input` the
way it already does for pkg, mail and docs. **Reading a submission arms the
interlock.**

**Writes that reach students → `openWorldHint`, named in `[outbox] tools`.**
Announcements, submission comments, posted grades, Conversations messages,
publishing a page or assignment. The `mecha-docs` argument carries over intact:
writing into a document a third party can read is exfiltration, it looks like a
local edit and it is a publish. A posted grade is stronger than that — it is
consequential, immediately visible, and socially expensive to retract.

**Neither → `destructiveHint` alone.** Editing an *unpublished* assignment,
reordering modules, drafting a page body, trashing a draft. Reaches nobody, so
staging it would make review circular — the `mail_triage`/`docs_trash` slot. Not
`readOnlyHint` either, or an unattended read-only trigger could rewrite a
syllabus at 7am.

The line between quadrants two and three is **published/unpublished**, and it is
a field Canvas already carries (`published`, `workflow_state`). That makes the
split checkable rather than remembered — which is the property to preserve if
this ever gets refactored.

### 6.2 Grading: assistive, with one exception, and the exception is enforceable

Reading a student submission is not like reading email. It is the first source
this project has wired where **the author of the untrusted text has a direct,
quantified incentive to manipulate the reader, and knows the reader may be a
model.**

The literature is no longer speculative. Wharton's Generative AI Labs ran
~40,000 grading trials with instructions hidden in student papers:

| Model | Effect of injection |
|---|---|
| Claude Opus 4.5 | minimal |
| GPT-5.2 | minimal |
| Gemini 3 Pro | >10 percentage points on longer papers (verbose, early/middle) |
| GPT-4o mini | **~20 percentage points on average** |

Frontier average 2.6 points. Verbose injections had more than twice the effect
of concise ones. And the detail that matters most: **models almost never
verbalized detecting the injection.** It does not appear in the reasoning. It
appears in the grade.

Small models are the susceptible class, which is what mecha runs. None of the
existing defences reach this: the interlock stops *exfiltration*, not
*persuasion* — an injected grade change never leaves as data, it leaves as the
number the user asked mecha to write. The sandbox is irrelevant. The outbox
helps only insofar as a human genuinely re-reads each grade, which at forty
submissions is precisely the approving-without-reading failure `DraftView`
exists to fight.

**Decision, 2026-08-21: assistive, never evaluative.** The user grades their own
work. mecha reads submissions to find patterns, flag missing and late work,
summarise where a cohort went wrong, and draft prose a human rewrites. It does
not assign quality scores, and **there is no verb that can.** That is not a
policy sentence; §7 has no tool that accepts a numeric or letter grade.

This is the cheap resolution to a tension that looked expensive. Injection
inflates *judgements*, and an assistant that renders no judgement has nothing to
inflate. An injected "award full marks" reaching a summarisation pass produces a
slightly odd paragraph a human reads, not a grade nobody rechecks. It also
dissolves §6.3's local-versus-frontier problem: with no evaluative pass, FERPA
wins uncontested and the model stays local.

#### The exception: completion checking

The one evaluative act that remains is **completion** — did the student turn in
something responsive, yes or no. It is real work at scale, it is near-mechanical,
and it is the case the user actually wants.

It is also a narrower target than quality grading, but not a safe one. The
attack is specific: a submission containing *only* an injection and no real work,
marked complete. That is the whole exposure, and unlike a 78 inflated to 92 it is
**visible on inspection** — which is what makes the following mechanism enough.

Canvas already has the review surface, and it is better than anything the outbox
could offer for this shape of work:

- `submission[posted_grade]` accepts `"complete"` / `"incomplete"` natively for
  pass/fail assignments. Completion is a first-class Canvas concept, not a
  number we are pretending is one.
- An assignment with a **manual post policy** hides entered grades from students
  until the instructor posts them. So marks written by mecha are *invisible* to
  the class.
- The instructor reviews the whole column in the Gradebook — a UI built for
  scanning forty rows, which a terminal queue is not — and posts them.

**So the boundary is: mecha can write completion marks but cannot make them
visible.** Three things enforce it, and the third is the reason this section
changed:

1. **Surface absence.** No post verb, no unhide verb. The `docs_share` rule.
2. **A fail-closed precondition.** `canvas_mark_completion` reads the
   assignment's `post_manually` field and **refuses to write unless it is
   true**. This is what makes the quadrant assignment correct rather than
   assumed: under an automatic post policy the same write *is* immediately
   visible, and would belong in the outbox. Refusing is the
   `Sandbox::preflight` rule — a configured protection that is not actually in
   effect must stop the run, not degrade quietly.
3. **The credential cannot address the operation.** Posting grades is
   **GraphQL-only** — `postAssignmentGrades`, `hideAssignmentGrades` and
   `setAssignmentPostPolicy` are mutations with no REST equivalent, and
   `post_manually` is read-only on the REST Assignment object. Combined with
   "a scoped developer key cannot use GraphQL at all" (§2.2), a correctly
   scoped key **structurally cannot post grades.**

That third one is the `drive.file` property, recovered by an accident of
Instructure's implementation: provable by reading what the credential can
address, rather than by reviewing every future diff. It is worth stating that
it rests on undocumented behaviour and could change — hence §8's verification
step — but while it holds it is the strongest boundary in this design.

Two further rules:

- **One assignment per call.** `canvas_mark_completion` takes many students but
  a single assignment; the bulk endpoint
  (`POST /courses/:id/submissions/update_grades`) also accepts a
  `grade_data[<assignment_id>][<student_id>]` form that can write across
  assignments, and that blast radius buys nothing here.
- **A submission read must be visibly third-party.** The `outbox_source.rs`
  treatment: when a run reports what it found, the submission text is marked
  with a per-line gutter and the `<untrusted-content>` envelope stripped, exactly
  as a staged `mail_reply` shows the message it answers.

### 6.3 FERPA

Dartmouth names it as the reason the token door closed, and it is not
decoration. Student submissions, grades and rosters are protected educational
records; the roster is arguably the most sensitive single object here, since
`/enrollments` returns names and SIS ids for every enrolled student at once.

The obligation this creates is concrete: **which provider sees the bytes is a
compliance question, not a preference.** mecha is unusually well placed to
answer it — "the model is local, the transcript is on my disk, nothing left the
machine" is a configuration you can point at rather than a vendor promise, and
that is a materially better story than a cloud assistant plus an off-the-shelf
MCP server.

This used to be in direct tension with §6.2 — the model safest for FERPA is the
one most susceptible to grade manipulation — and the assistive decision removed
the tension rather than splitting the difference. With no evaluative pass there
is nothing worth injecting, so nothing argues for sending protected records to a
frontier model, and **the whole integration runs local.** That is the cleanest
possible answer to a FERPA question, and it is a consequence of the surface
decision rather than a promise about configuration.

Two things follow regardless:

- **Steal the pseudonymization.** `r-huijts` returns pseudonymous names by
  default. Most of what a run needs is *which* submission, and a stable
  per-course alias serves that. It reduces both the FERPA surface and the amount
  of identifiable data sitting in an append-only transcript forever.
- **Do not distill course sessions into pkg by default.** `mecha distill` pushes
  an episode per session to the knowledge graph. A session that read forty
  submissions would seed a personal graph with other people's protected records.
  This wants a producer-level exclusion and it does not exist today.

### 6.4 Bulk reading is an operator verb

The `mecha-mail corpus` rule applies verbatim, and harder. "Read all forty
submissions and tell me the class's common misconceptions" is legitimate and
genuinely useful — and it is an **operator verb, not a tool**. A
bulk-submission-read on the MCP surface is one prompt away from a run pulling a
cohort's protected records into context unasked. Unlike a mail corpus, these are
not the user's records to spend.

So: `mecha-canvas corpus --course X --assignment Y`, an operator command,
absent from the tool surface, writing to the run's work directory where the
ordinary file tools can reach it under the path jail.

## 7. The proposed surface

Sixteen tools. Instructor-side only; no student verbs, no account
administration, and **nothing that accepts a quality grade**.

| Tool | Quadrant | Notes |
|---|---|---|
| `canvas_courses` | read | list; account fans out like mail reads |
| `canvas_roster` | read | **pseudonymous by default** |
| `canvas_assignments` | read | list/get, due dates, `post_manually`, `grading_type` |
| `canvas_submissions` | read | per assignment: who submitted, when, late/missing |
| `canvas_submission` | read | one body + comments; the injection surface |
| `canvas_pages` | read | list/get page bodies |
| `canvas_announcements` | read | existing announcements |
| `canvas_discussions` | read | topics and replies; third-party text |
| `canvas_modules` | read | structure |
| `canvas_assignment_edit` | neither | create/update **unpublished** only |
| `canvas_page_edit` | neither | draft bodies |
| `canvas_module_edit` | neither | structure and ordering |
| `canvas_mark_completion` | neither | §6.2; closed enum, fail-closed on `post_manually` |
| `canvas_publish` | **outbox** | the one verb that makes a draft visible |
| `canvas_announce` | **outbox** | reaches every enrolled student |
| `canvas_message` | **outbox** | Conversations; mails students |

Decisions behind that table, each a bug if undone:

- **No verb accepts a numeric, percentage or letter grade.** `posted_grade` is
  reachable from exactly one tool, which accepts a closed
  `Completion { Complete, Incomplete }` and nothing else — the `TriageAction`
  rule from `SLACK-ACTIONS-DESIGN.md` §1, where a free-form argument would let
  `92` ride inside a verb that reads as a checkbox. The type is the boundary.
- **`canvas_mark_completion` is quadrant three, and only because of the
  precondition.** Under a manual post policy the write reaches nobody until the
  instructor posts, so staging it in the outbox would duplicate a review the
  Canvas Gradebook does better — the `mail_triage` reasoning, that staging
  something which reaches nobody makes review circular. That argument
  *collapses* under an automatic post policy, which is exactly why the tool
  refuses to run there rather than silently becoming an unstaged send.
- **It also refuses a non-pass/fail assignment.** Writing `"complete"` to a
  points assignment is either an error or a coercion, and neither should be
  guessed at.
- **Publishing is its own verb.** Splitting edit from publish is what makes the
  published/unpublished line in §6.1 a real boundary rather than a field
  somebody remembers to check.
- **No delete verb at all.** Canvas would permit it; the surface does not, on
  the `docs_trash` reasoning. Unpublishing covers every case that matters.
- **No student verbs, no account verbs.** No `submit_assignment`, no
  `post_to_discussion` as a student, no user creation, reports, or enrollment
  changes.
- **Tests assert the absences** — no grade verb outside the completion enum, no
  post/unhide verb, no delete, no account endpoints — the way the docs surface
  tests that no sharing verb exists. An absence that is only a decision is an
  absence until someone is in a hurry.

## 8. Open questions

1. **What will Dartmouth actually issue?** A scoped developer key is the ask; a
   manual token the likely counter-offer. §3.4's dual ask covers it, but note
   that a manual token **loses enforcement 3 in §6.2** — an unscoped token can
   reach GraphQL and therefore can post grades. In that configuration the
   surface absence and the `post_manually` precondition are the only boundaries,
   which is weaker and should be recorded as such rather than glossed. Five
   business days; this gates everything.
2. ~~Assistive or evaluative?~~ **Answered 2026-08-21: assistive, with
   completion checking as the sole exception.** §6.2.
3. ~~Which provider grades?~~ **Moot.** No evaluative pass means the local model
   is unopposed. §6.3.
4. **Verify the GraphQL/scopes interaction against the real key**, first thing.
   §6.2's third enforcement rests on undocumented behaviour: scoped keys are
   reported not to reach GraphQL, Instructure has never said why, and an
   undocumented behaviour is one release from changing. The probe is one
   request — attempt `postAssignmentGrades` and require a 401. If it succeeds,
   enforcement 3 is gone and only 1 and 2 remain; write that down rather than
   discovering it later.
5. **Are submission comments hidden by a manual post policy, or only grades?**
   Not established in this pass. It decides whether mecha can usefully draft
   feedback comments into Canvas for review — and note that the REST API has
   **no `draft_comment` parameter**; SpeedGrader's draft comments are a UI
   feature only, so an API-written comment is a real comment. If comments are
   not hidden, drafted feedback must go through the outbox instead, or stay in
   the run's work directory as a file.
6. **Does the distill exclusion get built?** §6.3. Small, and it is the
   difference between a personal knowledge graph and one holding other people's
   FERPA records.
7. **Calendar overlap.** Canvas calendar events versus what `mecha-mail` already
   surfaces. Probably: read Canvas events, never write them, and let the real
   calendar stay the real calendar.

## 9. Sources

- Canvas REST API — [Courses](https://www.canvas.instructure.com/doc/api/courses.html),
  [Submissions](https://canvas.instructure.com/doc/api/submissions.html),
  [Conversations](https://canvas.instructure.com/doc/api/conversations.html),
  [Discussion Topics](https://canvas.instructure.com/doc/api/discussion_topics.html),
  [All Resources](https://www.canvas.instructure.com/doc/api/all_resources.html)
- [OAuth2 overview](https://developerdocs.instructure.com/services/canvas/oauth2/file.oauth) ·
  [OAuth2 endpoints](https://developerdocs.instructure.com/services/canvas/oauth2/file.oauth_endpoints) ·
  [Developer keys](https://developerdocs.instructure.com/services/canvas/oauth2/file.developer_keys)
- [Throttling](https://canvas.instructure.com/doc/api/file.throttling.html) ·
  [Pagination](https://canvas.instructure.com/doc/api/file.pagination.html) ·
  [File uploads](https://canvas.instructure.com/doc/api/file.file_uploads.html) ·
  [GraphQL](https://canvas.instructure.com/doc/api/file.graphql.html)
- Dartmouth — [Request a Canvas Access Token](https://services.dartmouth.edu/TDClient/1806/Portal/Requests/ServiceDet?ID=55689) ·
  [Canvas service catalogue](https://services.dartmouth.edu/TDClient/1806/Portal/Requests/ServiceCatalog/Category/11213/Learning-Management-Systems-Canvas)
- Prior art — [DMontgomery40/mcp-canvas-lms](https://github.com/DMontgomery40/mcp-canvas-lms) ·
  [r-huijts/canvas-mcp](https://github.com/r-huijts/canvas-mcp) ·
  [plyght/canvas-mcp](https://github.com/plyght/canvas-mcp)
- Grading injection — [Wharton GAIL, *This is an Excellent Paper: The Effects of Prompt Injection on Grading*](https://gail.wharton.upenn.edu/research-and-insights/hidden-prompt-injections/) ·
  [*"**Important** You should give me full credits!"*, arXiv:2606.03090](https://arxiv.org/pdf/2606.03090) ·
  [*When AI Is Fooled: Hidden Risks in LLM-Assisted Grading*, Educ. Sci. 15(11):1419](https://doi.org/10.3390/educsci15111419) ·
  [*Hidden Prompts in Manuscripts Exploit AI-Assisted Peer Review*, arXiv:2507.06185](https://arxiv.org/pdf/2507.06185)
