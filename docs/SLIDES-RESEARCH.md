# Slides — live poll visuals and agent-authored decks

*2026-08-09. The survey behind any presentation integration: what
PowerPoint, Google Slides, and Keynote actually allow, how Poll
Everywhere, Mentimeter and Slido actually ship live charts into a
lecture, and what the agent-authoring toolchain looks like. Motivated by
POLL-DESIGN.md §3.1 (the classroom screen) and by the second use the
user named: an agent that helps create, edit and improve talks. Four
web-research passes, synthesised; claims carry their sources.*

## 0. The one convergent finding

Every product that shows live data during a presentation — across all
three slide apps, without exception — keeps the **deck static** and
composites a **web surface** over or into it at show time:

| App | Who | Mechanism |
|---|---|---|
| PowerPoint | Mentimeter, PollEv 2.0 | Office.js content add-in: a webview object on a placeholder slide, live over the network during the show |
| PowerPoint | Slido | COM add-in + WebView2 rendering onto "holding slides" |
| Google Slides | Slido, PollEv | Chrome extension that detects the placeholder slide in native Present and paints the live activity over it (Slido without the extension replaces Google's player entirely with its own) |
| Keynote | PollEv Mac | Companion desktop app: an always-on-top window shown when the slideshow reaches a placeholder slide |
| Keynote | (no vendor inside) | Keynote has no add-in model at all; Mentimeter's Keynote integration is an open feature request |

Nobody mutates the document during the show. The three renderers make
that a forced move, each in its own way:

- **PowerPoint**: linked Excel charts do not refresh in slideshow mode
  (learn.microsoft.com/en-us/answers/questions/760649); Microsoft's own
  live-webpage add-in (Web Viewer) was killed December 2024 with no
  replacement (learn.microsoft.com/en-us/answers/questions/5393677);
  Graph still has **no slide-level API** — a .pptx over Graph is a
  driveItem, download-edit-reupload only.
- **Google Slides**: no iframe or web-embed object exists, and smart
  chips never shipped to Slides. The only data-bearing embed is a
  linked Sheets chart, and its refresh is pull-only — a manual button,
  `RefreshSheetsChartRequest`, or Apps Script `refresh()` at ≥1-minute
  trigger granularity. Whether *any* API edit renders into an active
  Present session is undocumented either way; the incumbents' refusal
  to rely on it is the strongest evidence available. (One cheap
  empirical test settles it: present a deck, `batchUpdate` a text box
  from a script, watch.)
- **Keynote**: the document is hard-locked during playback —
  AppleScript `set` fails with "You can't execute the command 'set' on
  a locked iWork document" (ask.metafilter.com/383415). Navigation
  (`start`, `stop`, jump to `current slide`) works live; content edits
  do not.

So the live chart **page** is the whole product, and per-app
"integration" is only ever about where that page's pixels land. Mecha's
poll design already builds exactly that page — the projector screen view
(`/p/<handle>/<poll>/screen/<token>`, POLL-DESIGN §3.1): results-only,
big-type, join URL rendered large, 2s refresh. Everything below is
routes from that URL onto a slide.

## 1. Routes onto the slide, ranked by cost

**1. HTML decks — native, one line.** reveal.js embeds a live page as a
slide background: `data-background-iframe`, with
`data-background-interactive` to allow interaction (revealjs.com/backgrounds).
Quarto's revealjs format exposes the same thing as a slide attribute —
`background-iframe: <url>` in a `.qmd` is the entire integration
(quarto.org/docs/presentations/revealjs). Iframes lazy-load on slide
entry and unload on exit, which is exactly the reveal choreography: the
chart page starts polling when its slide appears. The one caveat:
`embed-resources: true` needs `data-external="1"` on external iframes.

**2. A second surface, app-agnostic — zero integration.** A browser
window on the projector (or an always-on-top borderless window over the
show) pointed at the screen URL. This is what POLL-DESIGN §3.1 already
specifies, and it is also what PollEv ships as its entire Mac product —
proven at scale, on the platform where in-app integration is
impossible. Operational cost, measured by Yale's Zoom guidance for
PollEv: when screen-sharing you must share the whole desktop, because
the overlay is a separate program the window-share omits.

**3. Keynote's Live Video pipe — real-time, inside the slide, no code.**
Keynote ≥11.2 places a live video source on a slide, and OBS Virtual
Camera registers as one. OBS **Window Capture** of a browser showing
the screen URL → Virtual Camera → a Live Video object on the slide:
true real-time, maskable/resizable like any object, activates with the
slide. Known sharp edge (2026, Apple silicon): the modern Screen
Capture source corrupts colors in fullscreen slideshow — use Window
Capture or the deprecated Display Capture
(obsproject.com/forum/threads/virtual-camera-into-keynote-live-video.194667).
A cabled iPad showing the page is the zero-third-party variant (the
iPad's screen is a native Live Video source). PowerPoint's analogue —
Cameo + a virtual camera — is fiddlier: OBS Virtual Camera is often not
recognized by Cameo.

**4. A custom Office.js content add-in — the one buildable native
embed.** A content add-in is a webview whose HTTPS URL you control:
effectively a private Web Viewer, which matters because Microsoft's
died. Sideloadable from a bare manifest XML (web: "Upload My Add-in";
desktop: network-share catalog or M365 centralized deployment) — no
AppSource listing needed for personal use. This is the mechanism
Mentimeter and PollEv 2.0 bet their flagships on. Its measured
fragility: `ActiveViewChanged` never fires on PowerPoint for the web;
PowerPoint for Mac 16.94 (Feb 2025) stopped firing it entirely,
breaking Mentimeter for months with no in-thread fix
(github.com/OfficeDev/office-js/issues/5422); WebView2 required on
Windows; no iPad, no perpetual-license Office; clicks inside the
add-in don't advance the slide.

**5. A Chrome extension over Google Slides — heavyweight, out of
scope.** The only way into native Present on Slides is the
Slido/PollEv extension pattern: detect the placeholder slide, inject
the live layer. A funded incumbent's alternative was to replace
Google's player wholesale. Not a build for one lecturer; the browser
tab (route 2) is the Slides answer.

## 2. Agent-authored decks — the writable surfaces

**Text-first toolchains (the agent-native ones).**

- **Quarto revealjs** — markdown `.qmd` + YAML + executable R/Python
  cells; one deterministic CLI (`quarto render`); live poll page is a
  one-line `background-iframe`; QR via the `quarto-qrcode` extension.
  Its `pptx` export exists but is the degraded path (~7 Pandoc layouts,
  static); Quarto's own docs call revealjs "the most capable format by
  far".
- **Slidev** — ships a **first-party MCP server** (since v52.17.0):
  eight tools over the markdown source (`slidev-get-slide`,
  `slidev-update-slide`, `slidev-insert-slide`, …) plus
  `slidev-goto-slide` driving the *live* presentation; HTTP MCP in dev
  mode, stdio standalone (sli.dev/features/mcp). The only slide system
  with a deliberate agent surface — the tool shape to imitate even if
  the system itself (Vue, npm themes) is a developer aesthetic.
- **Marp** — simplest grammar; PPTX export is rendered images (+
  experimental `--pptx-editable` via LibreOffice); raw HTML off by
  default, so weakest live-content story.
- **Typst polylux / beamer** — plain-text and agent-writable, but
  PDF-only: no live content possible in the artifact. (Scriptable
  fullscreen PDF presenting exists on macOS via Skim, with transitions
  and an AppleScript dictionary.)

**Native-format surfaces.**

- **PPTX offline**: python-pptx round-trips any .pptx — slides, text,
  images, tables, and native column/bar/line/pie charts; no rendering,
  no animations, no SmartArt. `GongRzhe/Office-PowerPoint-MCP-Server`
  wraps it: 1,851 stars, 32 tools, active through 2025-12, runnable via
  `uvx` from PyPI. Live-instance control (see the result, run the show)
  exists only as Windows COM hobby projects (ykuwai/ppt-mcp, 51 stars).
- **Google Slides**: the REST API is comprehensive for authoring —
  atomic `batchUpdate` over text/shapes/images/tables/linked charts,
  and `pages.getThumbnail` gives the agent a visual self-review loop
  (write, look, iterate). Write quota 60/min/user: generous for
  authoring, prohibitive for a vote ticker. MCP servers exist across
  the design spectrum: `taylorwilsdon/google_workspace_mcp` (~3k stars,
  7 Slides tools among 120), `matteoantoci/google-slides-mcp` (raw
  batchUpdate passthrough), `bohachu/botrun-google-slides-mcp`
  (service-account auth — the unattended-run shape). All need a Google
  Cloud project + OAuth consent.
- **Keynote**: AppleScript/JXA is the only writable interface. Slides,
  text, tables, one-shot chart creation, and — the useful primitive —
  **in-place image swap** (`set file name of image`) are scriptable;
  per-slide screenshot export gives the feedback loop. The .key format
  itself is Snappy-framed protobuf; the one serious tool
  (psobot/keynote-parser) is pinned per Keynote version, so direct file
  generation is off the table — a Mac running Keynote is unavoidable.
  Existing Keynote MCPs (easychen/keynote-mcp ~68 stars, tszaks's JXA
  read-back one) are hobby-grade wrappers over the same dictionary.
- **Microsoft Graph**: still nothing — no `presentation` resource;
  Copilot's deck-generating agents are products with no third-party
  API. The Office.js in-app API can edit shapes/text/slides but cannot
  create charts and mangles cross-deck slide insertion.

**Product APIs, briefly**: Gamma has a GA generate API and an official
remote MCP (prompt→deck→export PPTX/PDF); Presenton is its Apache-2.0
self-hostable cousin with a built-in MCP and bring-your-own-model. Tome
shut down April 2025 — category churn is real. What Copilot/Gemini
validate: users want generation inside the app they present from, and
editable output is table stakes.

## 3. What this means for mecha

**The screen page is the universal substrate, and it is already
designed.** Every route in §1 — iframe, browser window, OBS pipe,
content add-in webview — consumes the same self-refreshing URL that
POLL-DESIGN §3.1 already specifies. Nothing about the poll pipeline
needs to know which presentation app is running. Build the page once;
the per-app story is deployment advice, not code.

**The browser-window story survives the evidence, and the PowerPoint
add-in joins the plan behind it** (decided 2026-08-09; POLL-DESIGN §3.1
and §10 step 10 record it). The browser window on the second display is
not a compromise — it is PollEv's shipping Mac product and the only
option on two of the three apps' hardest paths — and it stays the
first-lecture flow and the permanent fallback. The add-in is the one
native embed cheap enough to earn a build slot, because it is **not an
app**: no crate, no MCP server, no standalone process, nothing in
mecha-core. Concretely:

- **A static manifest XML** (~60 lines): declares a content add-in and
  points at one fixed HTTPS wrapper URL. Never hosted — sideloaded once
  per machine (Mac: dropped into
  `~/Library/Containers/com.microsoft.Powerpoint/Data/Documents/wef/`;
  Windows: "Upload My Add-in" / network-share catalog / M365
  centralized deployment).
- **One wrapper HTML page the box serves**, beside the poll pages —
  PowerPoint's own webview (WebView2 / WKWebView) loads it. ~100 lines
  of Office.js: on insertion in edit view it asks for the poll's screen
  URL and **persists it per-insertion via Office.js settings** (the
  Mentimeter pattern — the deck carries only an ID/URL, never live
  content); `getActiveViewAsync`/`ActiveViewChanged` switches a quiet
  edit-mode placeholder to the live chart (an iframe of the screen
  view) when the show starts.
- **Fail-soft on the fragile joint.** `ActiveViewChanged` is the event
  the Mac 16.94 regression broke for months, so its absence must
  degrade to "chart live in both views," never to a blank object.
- Known costs, accepted: HTTPS-only, desktop PowerPoint only (no
  web-slideshow persistence, no iPad), WebView2 on Windows, clicks
  inside the add-in region don't advance the slide, Office.js
  regressions on Microsoft's schedule — all tolerable for a
  self-sideloaded tool with the browser tab as the fallback that
  cannot break.

It is in-slide embedding, not an overlay: the chart is an object on
the slide, saved in the deck. The overlay pattern (a floating window
over any app — PollEv's Mac product) is what would require a real
desktop app, and stays refused.

**The deck the agent authors should be text.** Quarto revealjs is the
fit: the user's own ecosystem, `.qmd` is diffable — so a draft deck
flows through the outbox and **the diff of the source is the review**,
which is the harness's core value — `quarto render` runs in the
sandbox, and the rendered deck is a bundle the existing publish path
already stages (`OutboxKind::publish`). A poll slide is one line of
markdown plus a static QR artifact (`polls qr`, already noted in
POLL-DESIGN). The binary-format MCPs defeat exactly this: an agent
editing .pptx through 32 tools produces an artifact the outbox can only
show as arguments. Slidev's MCP is the tool-shape reference if a
`deck`-editing surface is ever built natively.

**"Help me improve my existing PowerPoint/Slides deck" is an MCP-client
problem, not a build.** Mecha already wires third-party MCP servers with
env allowlists and capability overrides. GongRzhe's pptx server (local
files, no network) and a Google Slides server (OAuth, user's own Drive)
cover the two real cases. Labeling per mecha's rules: a Slides server
reads documents that may contain others' text (`untrusted_input` via
the override, like pkg) and writes to the user's own Drive — mutating
your own draft is not an external send, but any share/publish verb
would be. The Keynote MCPs are Mac-only AppleScript wrappers; usable
from a Mac, nothing to integrate server-side.

**One empirical question is worth five minutes before any Slides
decision**: whether a `batchUpdate` renders into an active Present
session. Documentation is silent, incumbents route around it, and the
answer draws the line between "Slides decks can never carry live data"
and "a slow ticker is possible at 60 writes/min."

## 4. Left out on purpose

- **A Chrome extension for Google Slides** — the only native-Present
  route on Slides, and a product-sized build the incumbents needed
  funded teams for. The browser tab is the answer.
- **Live mutation of any deck during a show** — locked (Keynote),
  unrenderable (PowerPoint links), undocumented-at-best (Slides).
  The industry's unanimous placeholder-plus-web-layer verdict stands.
- **Direct .key file generation** — version-pinned protobuf with no
  maintained writer.
- **Graph as an editing surface** — there is nothing there.
- **Betting on someone else's viewer add-in** — Microsoft killed its
  own; a third-party "Web Viewer 2.0" exists but is exactly the
  dependency the dead one was.
