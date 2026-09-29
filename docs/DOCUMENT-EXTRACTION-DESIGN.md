# Document extraction: PDFs, text layers and a local OCR model

2026-09-29. One extraction capability — a PDF's own text layer and a local
OCR model's transcript, per page, side by side — that every part of mecha can
call: the assistant reading a workspace file or an attachment
(`document_read`), the owner at a terminal (`mecha document extract`), and
later the persona notebooks. Built on branch `feat/document-ocr`; the OCR
server is installed on this machine, and the `[documents]` config goes live
only with an install, which is the owner's call.

The code is `mecha-core/src/document.rs` (the library),
`mecha-core/src/tool/document.rs` (the tool), `mecha-cli/src/commands/document.rs`
(the CLI), and `scripts/llama/` (the server's units and launcher).

## 1. Two outputs per page, never merged

**The text layer is the file's words; the transcript is a model's reading of
a picture of them.** They answer different questions, so each page carries
both under its own label and its own page number, and nothing substitutes one
for the other silently:

- **Text layer** — `pdftotext` in **reading order**, one call per document,
  split on poppler's form feeds (a count that does not match the page count
  is refused, never realigned by guess). Exact, fast, and what a citation is
  checked against: `grounding::holds` matches a quote as one unbroken run of
  words. The text layer is also where an injection hides — white-on-white
  text no reader sees — which is why it is marked third-party (§3).
- **OCR transcript** — PaddleOCR-VL 1.6's reading of the rendered page:
  LaTeX for equations, the only text a scan has. A reading, not evidence of
  what the file says; the tool's own label says so on every page.
- **Regions** — `pdftotext -bbox-layout` blocks with their boxes in PDF
  points, cached beside the text and returned by `--json`, so a citation can
  later point at a region of a page. The model returns no boxes on the
  whole-page prompt (§5), so regions come from the text layer only.

**Reading order, not `-layout`.** The brief suggested `pdftotext -layout`;
measured, it breaks exactly the property the text layer is for. `-layout`
sets a two-column page's columns side by side, so a sentence wrapped within
one column is interleaved with the other column's words. Over the 48 pages in
§8, `-layout` held a **median of 75% (mean 62%)** of the sentences the
reading-order text holds as one word run; on the ResNet paper's five pages
it held 3–14%.

`mode` chooses per call: `auto` (default — the text layer where a page has
one, OCR where it has fewer than 30 non-space characters), `text`, `ocr`,
`both`.

## 2. The model and its server

**PaddleOCR-VL 1.6, from the publisher's own GGUF repository**
(`PaddlePaddle/PaddleOCR-VL-1.6-GGUF`, snapshot `511b0964`): a 0.47B ERNIE
language model and a 0.44B vision tower, both BF16, 936 MB + 882 MB. 1.6 is
the newest version, and the GGUF is official.

**Asked of the artifact, not assumed.** The installed llama.cpp (build 1193,
commit `95887577`) carries `clip_graph_paddleocr` in `libmtmd.so` (`strings`);
the model loaded with `--mmproj` and `/props` reported
`modalities.vision: true`, `n_ctx_slot = 16384`. No other stack was needed.

**Flags** (`scripts/llama/mecha-ocr-server`, each justified there):
`--mmproj` (the vision tower is a separate file — `LLAMA-SERVER.md` §Vision),
`--temp 0` (the card's setting), `-c 32768 -np 2` (two slots of 16,384: a page
is at most 1,280 image tokens — the projector caps it at 1,003,520 px, one
token per 28×28 — and the densest measured page wrote 1,678), `-cram 0` (page
images share no prefix worth caching), `--no-webui`, `--jinja`.

**Memory**: 2,639 MiB of GPU memory while loaded (`nvidia-smi
--query-compute-apps`), 2.2–2.4 GB peak per systemd's accounting; KV for both
slots is 576 MiB (18 KiB/token: 18 layers × 2 KV heads × 256 × f16).

## 3. A builtin tool, a confined parser

**The parser is the attack surface, so the parser is what is confined —
whichever process calls it.** A PDF is a program for a page-description
interpreter and poppler has a long CVE history. Every poppler call
(`pdfinfo`, `pdftotext`, `pdftoppm`) runs through `sandbox::Sandbox` with its
own fixed policy — bwrap by default: user/pid/ipc/net namespaces, no network,
the system read-only, a private `/tmp`, the environment cleared, and exactly
one writable directory, a fresh 0700 scratch holding a *copy* of the file —
plus `RLIMIT_AS` (2 GB), `RLIMIT_CPU`, `RLIMIT_FSIZE` (256 MB) and a
wall-clock timeout. This does not reuse the operator's `[sandbox]`, which may
be `none` or docker (whose image has no poppler); `[documents] confine`
chooses `bwrap`, `landlock` or an explicit `none`, and `docker` is refused.

**No unconfined fallback.** Each uncached extraction first runs `pdfinfo -v`
through the confinement; if that fails, the extraction fails and says why.
The confinement cannot degrade silently into an unconfined parse.

**What comes out is text and PNGs, and the PNGs are re-encoded here.** A
renderer compromised by the file could otherwise hand the OCR server a crafted
image, which would reach llama.cpp's own image decoder. `document::reencode_png`
decodes under explicit limits (8000 px a side, 512 MB) with the memory-safe
`image` crate and writes a fresh PNG. The model server only ever sees bytes
this process encoded.

**Why a builtin and not an MCP server (`mecha-pdf`).** An MCP server is
spawned once in one directory (`Tool::fixed_workspace`), and mecha jails each
run separately (`ToolCtx::resolve`, per web-chat session) — the reason
`image_generate` is a builtin. The confinement that matters sits around the
parser either way, so a server would add a process boundary around code that
is already outside the parser's reach, and lose the per-run jail. An MCP
server for other hosts (Claude Code) is a thin wrapper over
`document::Extractor` and is out of scope here (§9).

**Capabilities.** `document_read` declares `private` (it reads the owner's
files, as `fs_read` does) and `untrusted` (a document's words are its
author's), `Egress::None`, read-only. Every result carrying document content
is `.from_outside()`, so the taint arms on it; the tool's own refusals (not a
PDF, over the cap, outside the jail, a FIFO) are not, because they are not the
document's words. The path goes through `ToolCtx::resolve`, the file is opened
non-blocking and must be a regular file, and it is read bounded to the cap.

**Egress is `None` because the OCR URL must be loopback** —
`document::ocr_url` is `imagegen::is_loopback`'s rule, refused at
registration, and `[documents]` is global-file only (a project layer's is
stripped with a warning, tested on both sides): `ocr_url` is where page images
go, and `confine` is the confinement a cloned repository must not be able to
turn off.

**Caps at the door**, before anything parses: file size (`max_file_mb`, 100),
a `%PDF-` header in the first kilobyte (the extension is a claim), page count
(`max_pages`, 2000, from `pdfinfo`), pages sent to OCR per call
(`max_ocr_pages`, 30 — the rest are named in the answer, not dropped), and a
per-page timeout (`page_timeout_secs`, 180) on both rendering and OCR.

## 4. The cache

**Extraction is paid once per file.** Keyed by the sha256 of the bytes that
were rendered (the scratch copy — a file changing under the read cannot put
one file's text under another's key):

```
~/.mecha/documents/<sha256>/layer.json                 text layer + regions + sizes, all pages
~/.mecha/documents/<sha256>/ocr/<model>-<pipeline>/<page>.json   one transcript each
```

`<pipeline>` (`document::OCR_PIPELINE`, `wp1`) is bumped when the prompt,
the render size or the post-processing change, and `<model>` is the
configured model name, so a model change never serves a stale reading. A
failed page is never cached. Writes are atomic (temp file, rename), the
directories are 0700, and `$MECHA_DOCUMENTS_DIR` moves it.

**Retention is by age.** Reading an entry refreshes `layer.json`'s mtime;
writing a new entry prunes entries unread for `cache_days` (30); `mecha
document prune [--days N]` and `mecha document forget <file|sha256>` do it by
hand. `cache = false` writes nothing.

**What `forget` does not reach, said out loud.** The cache is
content-addressed and holds no path, name or session, so `mecha sessions
forget` (`forget.rs`) does not enumerate it: a PDF read in a forgotten session
stays extracted until it ages out or is forgotten by hash. So the forget
report says so: whenever `~/.mecha/documents/` holds anything, `Report::residue`
names it and the two commands that clear it, and a report can no longer read
`complete` while a forgotten conversation's PDFs sit extracted in silence
(found on review). Teaching `forget` to hash the session workspace's PDFs is
the fix and is not built.
**Incognito chats never offer the tool**: it is not in
`incognito::ALLOWED_BUILTINS`, because its cache writes outside the room.

## 5. Whole-page OCR, and the missing layout stage

**PaddleOCR-VL is an element recogniser, and its full pipeline is a layout
model plus the VLM.** The card's six prompts (`OCR:`, `Table Recognition:`,
`Formula Recognition:`, `Chart Recognition:`, `Seal Recognition:`,
`Spotting:`) are meant for regions that PP-DocLayoutV3 has cropped and
classified; llama.cpp serves the VLM only. This build sends the whole page
with `OCR:`.

**Verdict, measured (§8): good enough for prose and equations, not for
tables or headings.**

- **Prose**: median word recall 0.944 against the text layer over 48 pages,
  page-1 median CER 0.059. What remains is mostly reading order on two-column
  and figure-heavy pages and LaTeX against Unicode math, not misread words.
- **Equations**: inline and display math come back as LaTeX, correctly on the
  pages checked by eye (the Transformer paper's Eq. 1 and its
  `\frac{1}{\sqrt{d_k}}`).
- **Tables: not reliable, and the failure is the dangerous kind.** The
  Transformer paper's Table 2 came back once with its EN-FR column dropped
  (whole-page, 120 dpi probe) and once as one cell per line (the built
  pipeline, 103 dpi); BERT's Table 5 came back with its first row's label
  replaced by the next row's ("No NSP" where the source says BERT-base) —
  plausible, and wrong. The same Table 2 **cropped by hand and sent with
  `Table Recognition:`** came back complete, as OTSL, with one misplaced
  cell. `document::otsl_to_markdown` turns OTSL into a Markdown table
  whenever the model emits it.
- **Headings** are not marked (`#`); the transcript is structured text, not
  Markdown structure.

So the text layer stays the default, and the transcript's label says it is a
reading. **Options for the layout stage, none built:** (a) PP-DocLayoutV3 as
ONNX through the `ort` crate — one more native model, CPU is enough for a
layout pass, and region crops then go to the right element prompt; (b) a
Python sidecar running the official PaddleOCR pipeline against this same
llama-server (the card's documented path, a PyTorch/Paddle stack like
Chatterbox's); (c) crop from the text layer's own blocks (`Table N:`
captions) — only for born-digital files, which are the ones that need OCR
least. (a) is the recommendation if tables matter; it is the owner's call,
because it is a second model stack.

## 6. On demand: a socket that holds no memory

**Owner's ruling, 2026-09-29: nothing holds memory when OCR is not in use.**
The first request starts the server and waits until it answers; it stays up
while requests keep arriving; it stops after ten idle minutes.

**The mechanism is systemd socket activation, three units**
(`scripts/llama/`, installed by `scripts/llama/install.sh`):

```
llama-ocr.socket          127.0.0.1:8085, enabled at boot (sockets.target); holds no memory
llama-ocr-proxy.service   systemd-socket-proxyd --exit-idle-time=10min → 127.0.0.1:18085
                          Requires= and After= llama-ocr.service
llama-ocr.service         the llama-server; StopWhenUnneeded=yes; no [Install]
                          ExecStartPost = mecha-wait-healthy 18085 120
```

- **Idle-stopped never looks absent.** The port is always listening, so a
  request after a reboot or an idle stop is a cold start, never "connection
  refused" — `llama-embed`'s 2026-08-19 incident (a reboot restored the
  consumers and not the server, and a nightly logged SUCCESS having done
  nothing) cannot recur in this shape.
- **A listening server is not a ready one.** llama-server answers `/health`
  with 503 while loading; `mecha-wait-healthy` requires HTTP 200 *and*
  `"status":"ok"`, the backend unit is active only when it returns, and the
  proxy is ordered after the backend's start job — so the caller's connection
  waits through the load instead of being forwarded into it. The client
  checks `/health` the same way before its first page, with
  `ocr_ready_secs` (120) to wait.
- **Concurrent first requests start one server** — systemd merges start jobs.
  Measured: four simultaneous cold `/health` requests all answered 200 in
  3.03 s, and the journal shows one `Starting llama-ocr.service`.
- **The idle stop is the proxy's**: it exits after `MECHA_LLAMA_IDLE`
  (default `10min`) with no open connection, and the backend, no longer
  needed, stops. Measured with a 20 s runtime override: proxy and backend
  inactive at +20 s, the socket still listening, the process gone from
  `nvidia-smi`. Change it with `systemctl --user edit llama-ocr-proxy.service`
  (`[Service] Environment=MECHA_LLAMA_IDLE=20min`). A client holding a
  keep-alive connection keeps the model up while it holds it, so
  `OcrClient` pools idle connections for 5 s only.
- **Failure stays visible**: `Restart=on-failure`, not `always`; a model that
  cannot load fails the start, the proxy never starts, and the caller gets a
  named error. A missing GGUF makes the launcher exit with the `hf download`
  that fixes it.

**Why not llama-server's own `--sleep-idle-seconds`.** This build has it (asked
`--help`, read `server-context.cpp` at `95887577`): the process frees the model
and reloads it on the next request. Measured on this model, a sleeping
process still held **189 MiB of GPU memory and 352 MB RSS** (the CUDA context
and the binary) — not "nothing". Socket activation holds zero, and the port
is systemd's, so the process can stop entirely.

**Cold start, measured:** unit start → `/health` ok **2.96 s** from a cold page
cache, **1.16 s** warm; unit start → first page answered **3.66 s** (Transformer
page 1, 655 output tokens) — the load is under a second
(`loaded multimodal model` at 0.95 s in the journal), and the rest is the
page.

## 7. The embeddings server: on demand too, once mecha-graph can wait

The owner asked (2026-09-29) for `llama-embed.service` (:8081) to move to §6's
mechanism, and approved it in the owner's own session. The units are in
`scripts/llama/` — `llama-embed.socket` on :8081, `llama-embed-proxy.service`
idling out after ten minutes, `llama-embed.service` as the backend on :18081
behind `mecha-wait-healthy` — and `scripts/llama/install-embed.sh` performs
the switch, keeping the always-on unit and launcher as `*.always-on.bak` so
`install-embed.sh --remove` restores them in one step.

**The switch waits on a consumer, and the installer enforces it.** Every
consumer keeps its address — the point of a socket — but not every consumer
waits. mecha-graph's `Embedder::available()` probed `/health` with a **1.5 s**
timeout, and a cold start takes ~4 s; the probe's own request would wake the
server and then give up on it. It gates semantic search on every query
(`search.rs`), the `kg_search` handler in `mecha-graph-mcp`, `embed`,
`precheck` and the TUIs, so each would fall back to keyword-only with no
error — the unit's own 2026-08-19 incident again, where absence was silent.
mecha-graph `fix/embed-probe-cold-start` raises it to 20 s
(`embed::AVAILABLE_TIMEOUT`), with a test that fails at 1.5 s. Until that
build is installed (`mecha-graph` *and* `mecha-graph-mcp`),
`install-embed.sh` refuses to run without `MECHA_EMBED_COLD_START_OK=1`.

Every other consumer found waits long enough: mecha-graph's embedding
requests allow 300 s, and mecha's TUI grouping 360 s.

## 8. Measurements

**Command**: `scripts/ocr-measure.py` (its header has the download line), run
against `llama-ocr.socket` on :8085 — llama.cpp `95887577`, the GGUF snapshot
above, PaddleOCR-VL 1.6, 2026-09-29. Ten arXiv papers (1406.2661, 1412.6980,
1505.04597, 1512.03385, 1609.02907, 1706.03762, 1810.04805, 2005.14165,
2006.11239, 2106.09685), pages 1–4 and the middle page of each: 48 pages,
rendered at 101–103 dpi (the projector's budget), 1,253–1,273 prompt tokens
each. All 48 finished `stop`.

| | value |
|---|---|
| per page, quiet GPU (first 23 pages) | median **4.5 s**, range 2.3–7.5 s |
| per page, all 48 | median 7.0 s, p90 36.4 s, max 53.3 s |
| output tokens per page | median 1,010, max 1,678 |
| word recall vs text layer | median **0.944** (p25 0.874, p75 0.970, min 0.667) |
| CER vs text layer | median 0.096 (p25 0.027, p75 0.229); page 1 of each paper 0.059 |
| sentences holding as one run in `-layout` text | median 0.75, mean 0.62 |

**The slow tail is contention, not the model.** Latency climbed from the
24th page on: during those pages ComfyUI was generating (its GPU memory went
from 6.6 to 16.1 GB), the chat model had a slot processing, and GPU
utilisation read 96%. Latency is the GPU's
to share: the model is bandwidth-bound like every other here
(`LLAMA-SERVER.md`), and on a busy box a page costs 20–50 s.

**CER overstates the error.** It is a character edit distance against the
text layer, so reading-order differences (a caption before its figure, two
columns) and LaTeX against Unicode math both count as errors; word recall,
order-free, is the better read of whether the model read the words. The
worst pages (0.667 recall, DDPM p3; 0.697, Adam p4) are equation-dense.

**End to end through the built code** (`target/debug/mecha document extract`,
bwrap confinement, a scratch `MECHA_HOME`): Transformer page 1 text layer in
0.30 s; page 8 OCR in 21.9 s under the same contention; the same page again
from the cache in 0.06 s.

## 9. Deliberately out of scope

- **The layout stage** (§5): no second model stack without the owner's ruling.
- **Region re-reads** — a `region` + element prompt on the tool, to send a
  cropped table to `Table Recognition:`. Cheap to add once something decides
  where the table is; without a layout model that something is the model
  guessing coordinates it cannot see.
- **An MCP server for other hosts** — a thin wrapper over `Extractor` when a
  host needs it.
- **Formats other than PDF**, and images as documents (`image_view` covers a
  look; OCR of a photographed page would take the same path minus poppler).
- **Web uploads and Slack attachments** reach the tool as workspace files
  (`inbox/…`) and need nothing new; a surface that renders the regions is not
  built.
- **`forget` over the cache** (§4) and **incognito support** (it would need
  `cache = false` for the room's runs and an OCR server that keeps nothing,
  which llama-server does — no prompt cache, `-cram 0`).
- **The embeddings conversion** (§7).
