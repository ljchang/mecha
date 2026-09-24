# Can a classifier take friction out of the interlock? — research

**2026-09-24.** The question: *the owner's conversations arm on their first
graph or mail read, and from then on everything that addresses the world is
refused or staged. Can a learned classifier — a prompt-injection detector, a
tool-call monitor, or a small general-purpose decision model like Jev or
Laya — decide when that restriction is unnecessary?*

**Answer: no, not as the thing that lifts it.** A classifier may add friction,
triage, or choose what the *system* then removes. It must never be the gate
that grants a send, because that gate reads the attacker's text and grants
the attacker's goal, and every detector measured against an attacker who
adapts to it has been broken. `PROVENANCE-DESIGN.md` is what this document
produced.

Numbers below were checked against the paper or primary page by the research
pass; anything checked only through a summariser or a vendor says so.

---

## 1. Detectors, measured against an attacker who adapts

- **"The Attacker Moves Second"** (Nasr, Carlini et al., authors from OpenAI,
  Anthropic and Google DeepMind; USENIX Security '26,
  [arXiv 2510.09023](https://arxiv.org/abs/2510.09023)). Twelve published
  defences, attack success above 90% against most. A score-guided search
  beat ProtectAI, PromptGuard and Model Armor at >90%, PIGuard at 71%,
  MELON at 76% blind and 95% with knowledge; DataSentinel fell "in every
  case". Human red-teamers (500+) succeeded in 100% of scenarios. In their
  words, "simply adding more filters or stacking additional detectors does
  not resolve the underlying robustness problem" — while detectors "can
  still provide practical value by blocking some unsophisticated or
  opportunistic attacks".
- **Monitors are an attack surface.** "Agent-as-a-Proxy"
  ([arXiv 2602.05066](https://arxiv.org/abs/2602.05066)): the agent carries
  the attack string to its own monitor. Against LlamaFirewall's
  AlignmentCheck, attack success within five tries was 42–93% depending on
  the agent model, and adding Prompt Guard 2 did not lower it. A 72B monitor
  was bypassed too.
- **Activation probes** (TaskTracker, [arXiv 2406.00799](https://arxiv.org/abs/2406.00799),
  AUC ≥ 0.99 against static attacks) have no published adaptive evaluation,
  and "Obfuscated Activations" ([arXiv 2412.09565](https://arxiv.org/abs/2412.09565))
  drove probe recall from 100% to 0% while keeping the jailbreak working.
  They would also need hidden states llama-server does not expose.

## 2. What a detector costs on ordinary mail

- **Over-defence.** NotInject (InjecGuard/PIGuard, ACL 2025,
  [arXiv 2410.22770](https://arxiv.org/abs/2410.22770)): 339 benign prompts
  full of trigger words — "ignore", "urgent", "instructions" — put most
  shipped guards near random. That is what a faculty inbox is made of.
- **Utility.** On AgentDojo (Llama-3.3-70B, "Attacker Moves Second" Table 7)
  an undefended agent completes 56.7% of tasks; behind ProtectAI 28.9%,
  behind PIGuard 30.9%. The official AgentDojo board shows the same shape
  for GPT-4o: a PI detector cuts utility from 69% to 41%.

A detector that fires on every third email would be a second, noisier
version of the refusal this work is trying to remove.

## 3. Jev and Laya

Both launched in September 2026, after the model writing this was trained;
everything here is from the sources linked, not from memory.

- **Jev** (TypeSafe AI, [launch post](https://typesafe.ai/blog/introducing-system-one-models-and-jev)).
  A "decision model": text plus a typed question in, a calibrated choice,
  score or yes/no out, never generated text. ~100 ms, very cheap, and LiteLLM
  measured 95% on an 80-case routing task
  ([benchmark](https://docs.litellm.ai/blog/jev-auto-router-benchmark));
  LangChain uses it to gate tool calls. **API-only and proprietary** — every
  classification ships the text to a US cloud service, so on this machine
  the classifier would itself be an egress path for private data. Ruled out.
- **Laya** (ConvAI Innovations, [model card](https://huggingface.co/convaiinnovations/laya)).
  The open-weights answer: ModernBERT-large plus a decision head, 421M,
  Apache 2.0, ~800 MB, ~33 ms per question on a small GPU. It would sit
  beside llama-server easily. But one independent 78-case comparison put it
  at 0.59 against Jev's 0.97, it ships overconfident until recalibrated, the
  English model reads 512 tokens, and its card documents no injection or
  tool-call evaluation.

Neither changes the answer in §1; they change only what a friction-adding
classifier would cost to run.

## 4. What survives: structure, and two shapes where a label may drop

- **Structural defences** — CaMeL ([2503.18813](https://arxiv.org/abs/2503.18813)),
  FIDES ([2505.23643](https://arxiv.org/abs/2505.23643)), Progent
  ([2504.11703](https://arxiv.org/abs/2504.11703)) — stop the injections in
  their benchmarks at a utility cost (CaMeL 77% of AgentDojo against 84%
  undefended). **Caveat that matters:** these too were validated on static
  attacks; "Adaptive Evaluation of Out-of-Band Defenses"
  ([2606.26479](https://arxiv.org/abs/2606.26479)) has one small adaptive
  data point (Progent, held at 2.6%).
- **Why CaMeL's email suite barely refused anything:** tool outputs "can be
  easily annotated (e.g., the set of people who can read the content of an
  email are the recipients)". Field-level labels, set by a parser.
- **Lowering a label safely.** Permissive IFA (Siddiqui et al., TMLR 2025,
  [2410.03055](https://arxiv.org/abs/2410.03055)) and RTBAS
  ([2502.08966](https://arxiv.org/abs/2502.08966)) both let a model *choose*
  what was relevant, then have the **system regenerate or redact** from that
  choice, so a wrong choice costs utility and never safety — "because it is
  enforced by the system, rather than the model". RTBAS stopped every
  targeted AgentDojo attack at a 2% utility loss.
- **Design Patterns** (Beurer-Kellner et al., [2506.08837](https://arxiv.org/abs/2506.08837))
  §4.3 is an email-and-calendar case study close to this one: plan-then-execute
  "would not decrease utility for most use cases", and nothing stops injected
  words inside a reply the user genuinely asked for — which is the outbox's
  job here, not the interlock's.

## 5. The rule this leaves

| Role | Wrong answer costs | Allowed |
|---|---|---|
| Escalate, flag, add a prompt | a needless prompt | yes |
| Rank or label an outbox draft for review | a reviewer's attention | yes |
| Anomaly signal for the diagnostician | a false lead | yes |
| Choose what the system redacts before re-deriving a call | a refused call | yes — the system enforces |
| Decide content is clean, so do not arm | **the leak** | **no** |
| Decide a send is benign, so let it through armed | **the leak** | **no** |

None of the four decisions in `PROVENANCE-DESIGN.md` depends on a
classifier. If one is added later, it goes in the top four rows.
