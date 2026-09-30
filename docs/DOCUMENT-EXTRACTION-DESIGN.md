# Document extraction: PDFs, text layers and a local OCR model

> **Addendum (2026-09-29):** built, merged (#404, #406) and installed the same
> evening; §7's embeddings switch was applied at 20:43Z, after mecha-graph #26.
> What shipped is in HISTORY under 2026-09-29; the minors left are mecha
> #410–#413.

2026-09-29. One extraction capability — a PDF's own text layer and a local
OCR model's transcript, per page, side by side — that every part of mecha can
call: the assistant reading a workspace file or an attachment
(`document_read`), the owner at a terminal (`mecha document extract`), and
later the persona notebooks. Built on branch `feat/document-ocr`; the OCR
server is installed on this machine, and the `[documents]` config goes live
only with an install, which is the owner's call.

The code is `mecha-core/src/document.rs` (the library),
`mecha-core/src/layout.rs` (the layout stage, §5), `mecha-core/src/tool/document.rs`
(the tool), `mecha-cli/src/commands/document.rs` (the CLI), `scripts/llama/`
(the server's units and launcher) and `scripts/layout/` (the layout model's
installer).

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
  later point at a region of a page. A transcript read through the layout
  stage (§5) carries its own regions — the layout model's boxes, in the same
  frame, each with what was read there; a whole-page reading has none.

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

`<pipeline>` is `document::OCR_PIPELINE` (`wp1`) for a whole-page reading and
`LAYOUT_PIPELINE` plus the first 16 hex of the layout model file's sha256
(`ly1-45bf71750b00739a`) for a layout reading; each is bumped when its
prompt, render size or post-processing changes, and `<model>` is the
configured model name, so a model change never serves a stale reading and a
whole-page reading is never served as a layout one. A layout page with a
region that failed is not cached. A
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

## 5. The layout stage: region by region

**PaddleOCR-VL is an element recogniser, and its full pipeline is a layout
model plus the VLM.** The card's six prompts (`OCR:`, `Table Recognition:`,
`Formula Recognition:`, `Chart Recognition:`, `Seal Recognition:`,
`Spotting:`) are meant for regions that PP-DocLayoutV3 has cropped and
classified; llama.cpp serves the VLM only. The first build sent the whole
page with `OCR:`, and measured (§8) that was good for prose and equations and
**not for tables — the failure the dangerous kind**: the Transformer paper's
Table 2 came back once with its EN-FR column dropped and once as one cell per
line, and BERT's Table 5 came back with its first row labelled "No NSP" where
the source says BERT-base — plausible, and wrong. The same Table 2 cropped by
hand and sent with `Table Recognition:` came back whole. Headings were not
marked at all. **Owner's ruling, 2026-09-29: "tables: your rec"** — option
(a) of the three this section listed: PP-DocLayoutV3 as ONNX, ahead of the
VLM. Built on `feat/document-layout` (`mecha-core/src/layout.rs`).

**The pipeline, per OCR page** (`Extractor::read_by_layout`):

1. poppler renders the page at 144 dpi (PaddleX's zoom 2; lower for a page
   over 4 Mpx), confined as always, and this process decodes it under limits.
2. The decode is resized to 800 × 800 (bicubic, aspect not kept — the
   model's `inference.yml`) and written to the layout worker as a float
   tensor. The worker answers with boxes, classes, scores and a
   reading-order score.
3. PaddleX's own post-processing for this model, ported
   (`layout::postprocess`): threshold 0.3, NMS (IoU 0.6 within a class, 0.98
   across), a whole-page `image` dropped, anything ≥ 90% inside a title,
   formula or chart swallowed by it, sorted by the model's reading order,
   then the VL pipeline's `filter_overlap_boxes` (an inline formula half
   inside a paragraph is read as part of it; of two mostly overlapping boxes
   the larger stays, unless a table overlaps a figure). Its `merge_blocks`,
   which stitches neighbouring text regions into one image to batch them, is
   a throughput trick and is not ported.
4. Each region is cropped from the same decode, re-encoded (downscaled past
   the projector's budget), and sent with its task's prompt — `Table
   Recognition:` for tables (OTSL → a Markdown table), `Formula Recognition:`
   for display and standalone inline formulas, `OCR:` for everything else.
   Images, charts and seals are **not sent** and are marked `*[image]*` in
   place: PaddleOCR-VL 1.6's defaults leave chart and seal recognition off,
   and so text inside a figure is not transcribed (§8). Two regions are in
   flight at once — the server's two slots.
5. The page is assembled in reading order: `doc_title` as `#`, a
   `paragraph_title` as `##` plus one `#` per level of its numbering (`3.2.1`
   → `####`, PaddleX's rule), display maths as `$$ … $$`.

The regions are stored with the transcript (`OcrPage::regions`: label,
score, box in PDF points from the top-left — the frame the text layer's
`Region` uses — and what was read there), so a citation can later open the
region it came from. `--json` returns them.

**The runtime: a confined Python child running ONNX Runtime, not the `ort`
crate.** §3's rule for the parser holds for anything else that reads what a
document's author chose: it runs confined and fails closed. That rules out
running the model inside mecha: `ort` in-process would put ONNX Runtime's C++ kernels — including
data-dependent ones this graph uses (`GridSample`, `TopK`, `GatherND`,
`ScatterND`) — in the process that holds the owner's keys and transcripts.
Confining `ort` means a second binary, and `ort`'s default build downloads a
prebuilt ONNX Runtime from its maintainers' CDN at compile time (a
supply-chain step inside `cargo build`, and no offline build); its
`load-dynamic` mode still needs a `libonnxruntime.so` from somewhere at run
time. The child is instead the publisher's own runtime: Microsoft's
`onnxruntime` 1.30.0 wheel (MIT) and numpy 2.5.3 (BSD-3-Clause), every file
pinned by hash (`scripts/layout/requirements.txt`), in a venv
`scripts/layout/install.sh` creates under `~/.mecha/layout/` (111 MB). The
Cargo dependency graph does not change. The worker script
(`layout_worker.py`) is compiled into the binary and run with `python -I -B
-c`, so nothing on disk stands in for it.

**Confined like poppler, and it never sees an image file.** The child runs
under the `[documents] confine` backend (bwrap by default: no network, a
private `/tmp`, the system read-only, a fresh 0700 scratch as its only
writable path and `$HOME`, the environment cleared) with exactly three more
read-only paths — the venv, the interpreter's real prefix, and the model
file's canonical path — plus `RLIMIT_AS` (`layout_memory_mb`, 4096; measured
peak RSS 1.1 GB), `RLIMIT_CPU` and a 16 MB `RLIMIT_FSIZE`. Its input is a
fixed-size float tensor this process built from its own decode, so the only
thing a hostile page controls in there is pixel values; its answer is parsed
as untrusted (row count capped at 1,000, unknown classes and non-finite
values dropped, boxes clipped). Tested, not asserted
(`the_layout_worker_cannot_reach_outside_its_confinement`): a fake
interpreter that reports ready only if it can read a file outside its paths
starts unconfined and is refused under bwrap. **The GPU is not used**: the
confinement exposes no `/dev/nvidia*`, the GPU is shared with a 42 GB chat
model and ComfyUI, and CPU costs 0.6 s a page.

**Nothing stays resident.** One worker per extraction call, started on the
first page that needs it (0.85 s: interpreter, imports, model load), fed
every page of the call, killed when the call ends; a worker that fails mid-
page is dropped and the next page starts a fresh one.

**Unavailable is said, never passed off.** `layout = true` (the default) and
the model or interpreter missing, or the worker unable to start under the
confinement: every OCR page of the call is read whole, the transcript carries
`fallback` ("the layout stage is unavailable"), the page label reads `read
whole — …` where a layout reading reads `read by region: N`, and
`Extraction::layout_unavailable` names the cause once, at the top of the
render and on stderr from the CLI. `layout = false` reads whole with no
fallback note. A page where the model finds nothing to read is read whole,
with that reason. A region that fails (cut off, empty, past the per-page cap
of 120 regions) is named in place (`*[table not transcribed: …]*`), keeps its
page out of the cache, and makes `mecha document extract` exit non-zero.

**The model** is the publisher's own ONNX export,
`PaddlePaddle/PP-DocLayoutV3_onnx` at `46bbdf18`, Apache-2.0, one 130.5 MB
file (`inference.onnx`, sha256 `45bf7175…28ba`, checked by the installer),
opset 17, inputs `image` (N × 3 × 800 × 800), `im_shape`, `scale_factor`,
outputs boxes (N × 7: class, score, box, reading order), a count and masks
(not requested). 25 classes. It is the layout stage of PaddleOCR-VL 1.5 and
1.6 (`PaddleOCR-VL-1.6.yaml`), so no nearer alternative was needed.

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

### The layout stage, before and after

**Commands.** Built code only: `target/release/mecha` at `feat/document-layout`
code commit `99d132d6` (then on `feat/document-ocr` `240e52ec`). The
branch was rebased twice after measuring — to `bfb46a4a` on `16169afa`, then
to `cb10e8cf` on `5b662129` — and neither rebase touched the layout
pipeline: the second changed only the per-page error arm (a thin text layer
is now shown beside a failed OCR) and added `ParserSaid`. The numbers below
are `99d132d6`'s; re-measure before quoting them against a later commit. The
layout stage installed by `scripts/layout/install.sh` into a scratch
`MECHA_HOME`, bwrap confinement, the same server as above. Both recipes go
through mecha, uncached (`--no-cache`), one CLI call per page — "before" is
the same binary with `layout = false`:

```
MECHA=target/release/mecha MECHA_HOME=$H_WHOLE  OUT=rel-whole  uv run --with rapidfuzz python scripts/ocr-measure.py
MECHA=target/release/mecha MECHA_HOME=$H_LAYOUT OUT=rel-layout uv run --with rapidfuzz python scripts/ocr-measure.py
python3 scripts/ocr-compare.py rel-whole rel-layout

# tables: the table pages through both recipes, then the comparison
P=1706.03762.pdf:8,1512.03385.pdf:6,1609.02907.pdf:7,1810.04805.pdf:8,2106.09685.pdf:4,2005.14165.pdf:37
PAGES=$P MECHA=target/debug/mecha MECHA_HOME=$H_WHOLE OUT=tab-whole uv run --with rapidfuzz python scripts/ocr-measure.py
MECHA=target/debug/mecha MECHA_HOME=$H_LAYOUT python3 scripts/table-measure.py \
  pdfs/1706.03762.pdf:8 pdfs/1512.03385.pdf:6 pdfs/1609.02907.pdf:7 \
  pdfs/1810.04805.pdf:8 pdfs/2106.09685.pdf:4 pdfs/2005.14165.pdf:37 --before tab-whole
```

The table runs used the debug build of the same code, earlier the same
afternoon while ComfyUI was generating — accuracy only; no table timing is
quoted.

**Conditions**, 2026-09-29 18:38–18:47Z: ComfyUI loaded but idle for the
whole run (377 MiB of GPU memory throughout, sampled every 5 s), the chat
model resident and idle; GPU utilisation peaked at 96% — the OCR server's
own. All 96 pages finished `stop`; no region failed; every layout page was
read by region (none fell back).

**Tables** — every table on the measured pages, plus the Transformer's
Table 2 (page 8) that failed before: nine tables, 89 rows.
`scripts/table-measure.py` takes the text layer's words inside each table's
box as the truth, groups them into rows by baseline (a raised exponent or
superscript folded into its row), and counts a row **held** when all its
tokens appear on one line of the reading — a Markdown table row after, any
line before — with LaTeX reduced to glyphs (`10^{20}` → `10 20`, `\pm` →
`±`).

| table | rows | held, before | held, after |
|---|---|---|---|
| Transformer Table 2 (p8) | 13 | 1 | **13** |
| ResNet Table 3 (p6) | 11 | 0 | **11** |
| ResNet Table 4 (p6) | 11 | 0 | **11** |
| ResNet Table 5 (p6) | 7 | 0 | **7** |
| GCN Table 2 (p7) | 8 | 8 | 8 |
| GCN Table 3 (p7) | 12 | 2 | 6 |
| BERT Table 5 (p8) | 7 | 5 | 6 |
| LoRA Table 1 (p4) | 6 | 6 | 6 |
| GPT-3 Table 6.1 (p37) | 14 | 2 | **14** |
| **all** | **89** | **24 (0.27)** | **82 (0.92)** |

Tokens found anywhere in the reading: 458/511 (0.90) before, 504/511 (0.99)
after. Checked by eye against the PDF:

- **Transformer Table 2**: before, one cell per line with values out of
  order and one misread (`23.3`); after, all eleven rows and five columns,
  both EN-DE and EN-FR, the spanning training cost of the two Transformer
  rows as a merged (empty) cell. The model doubles braces in exponents
  (`10^{{20}}`).
- **BERT Table 5**: before, the first row labelled "No NSP" (the wrong
  label, twice over); after, `BERT_{BASE}` with its own numbers, every row
  right. The one row "not held" is the two-line header, which the model
  splits into two Markdown rows — the cells are right.
- **ResNet Tables 3–5**: before, flattened; after, every row and value.
- **GCN Table 3** (propagation models): the numbers are right in every row;
  after, the Chebyshev rows' `K = 3` / `K = 2` sub-labels are **dropped**
  (a real loss), and the formula cells are LaTeX the metric cannot match to
  the text layer's glyphs — the rest of the "not held" rows.
- **LoRA Table 1** and **GCN Table 2**: right both ways.

**Prose and latency, 48 pages:**

| | before (whole page) | after (layout) |
|---|---|---|
| word recall vs text layer | median 0.944 (p25 0.874, p75 0.971) | median **0.951** (p25 0.851, p75 0.977) — better on 28, worse on 13 |
| prose word recall (outside formula boxes) | median 0.928 (p25 0.897, p75 0.959) | median **0.928** (p25 0.896, p75 0.976) — better on 25, worse on 12 |
| CER vs text layer | median 0.096; page 1 0.059 | median 0.105; page 1 0.077 — worse on 34 |
| per page, wall clock (one CLI call each) | median 5.01 s (p90 6.4, max 7.6) | median 5.30 s (p75 6.45, max 8.55) |
| OCR time inside mecha | median 4.53 s | median 4.07 s |
| layout model, per page (CPU, 4 threads) | — | median 0.62 s (0.58–2.11) |
| prompt / output tokens per page | 1,253 / 1,010 | 2,579 / 988 |
| regions per page | — | median 14 (3–26) |

The whole-page numbers reproduce the direct measurement above to the third
decimal (recall median 0.944, p25 0.874), so the two recipes are compared
under one harness.

- **Recall held; CER rose, and it is order, not misreading.** Of the 34
  pages whose CER rose, 24 have prose recall equal or better. The layout
  reading includes what the whole-page one skipped — the arXiv margin stamp,
  page numbers, headers — in the layout model's reading order, which is not
  `pdftotext`'s (DDPM page 1: recall 0.944 → 1.000, CER 0.029 → 0.054).
- **The largest prose losses are by design.** BERT page 3 (0.833 → 0.812):
  Figure 1's labels ("Mask LM", "NSP", "Question Answer Pair") were read by
  the whole-page prompt and are not now — figures are marked, not sent
  (PaddleOCR-VL 1.6's default). GAN page 4 (0.945 → 0.914): the missing
  "words" are the variables `x`, `z` of display maths now written as
  `\pmb{x}`.
- **Latency.** One page per call, the layout recipe costs +0.3 s at the
  median: the worker's start (≈0.85 s: interpreter, imports, model load) is
  paid per call and the layout model 0.6 s per page, against OCR that is
  faster because regions go two at a time to the server's two slots. **Over
  a multi-page call the layout recipe is faster**: BERT pages 1–10 in one
  call took 48.3 s and 47.9 s by layout against 81.9 s and 84.4 s whole
  (release build, ComfyUI idle, two runs each).
- **The layout model on CPU and GPU**, in isolation (ONNX Runtime 1.30, one
  800 × 800 page, steady state after one warm-up): CPU 0.47 s with 4 threads
  (the default), 0.41 s with the default thread count, 0.31 s with 8; load
  0.3 s. **GPU** (`onnxruntime-gpu` 1.30 CUDA provider, measured once for the
  record, not shipped): 0.058 s steady, 0.69 s first run, 0.74 s load. The
  in-pipeline 0.62 s includes the bicubic resize and the 7.7 MB tensor
  through the pipe.
- **Memory**: the worker's peak RSS is 1.1 GB for the ~1–3 s it lives; mecha
  itself peaked at 54 MB on the 10-page call (30 MB whole-page).

## 9. Deliberately out of scope

- **Text inside figures and charts** is not transcribed (§5, §8: BERT's
  Figure 1). PaddleOCR-VL can be asked (`Chart Recognition:`, or `OCR:` on a
  figure — PaddleX's `use_ocr_for_image_block`); whether to spend the
  requests is the owner's call.
- **Region re-reads** — a `region` + element prompt on the tool, to re-read
  one stored region (e.g. a table with a different prompt). The regions and
  their boxes are now stored (§5), so this is a small addition when a
  surface wants it.
- **The GPU for the layout model**: 10× faster (§8) but it needs
  `/dev/nvidia*` inside the confinement and the CUDA wheels, on a GPU that is
  already the box's bottleneck.
- **An MCP server for other hosts** — a thin wrapper over `Extractor` when a
  host needs it.
- **Formats other than PDF**, and images as documents (`image_view` covers a
  look; OCR of a photographed page would take the same path minus poppler).
- **Web uploads and Slack attachments** reach the tool as workspace files
  (`inbox/…`) and need nothing new; a surface that renders the regions is not
  built. A persona chat's uploads reach it the same way, once the owner lists
  `document_read` for that persona (PERSONA-DESIGN D24, 2026-09-30).
- **`forget` over the cache** (§4) and **incognito support** (it would need
  `cache = false` for the room's runs and an OCR server that keeps nothing,
  which llama-server does — no prompt cache, `-cram 0`).
- **The embeddings conversion** (§7).
