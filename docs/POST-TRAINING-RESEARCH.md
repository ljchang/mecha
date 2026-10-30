# Post-training the local model for this harness: SFT, RL, or neither yet?

Research pass, 2026-10-30, at the owner's request: how could
Qwen3.6-35B-A3B be post-trained, with supervised fine-tuning or
reinforcement learning, to operate better inside mecha? No training was
run. `EXPERIMENT-DESIGN.md` §10 put training out of scope for the
experiment programme's phase one; this is the research for a separate arc,
should the owner open one.

**Marks**: ✅ read in the primary source or artifact this pass · 📰 a
vendor's or author's figure, not reproduced · ❓ inferred, not established.

---

## The one-sentence answer

**Not yet, and not for format: the model already speaks mecha's tool
protocol almost without error, so the first step is to classify the run
corpus's failures, and if a trainable behaviour is what remains, LoRA SFT
on corrected trajectories rendered with the router's exact template, on
one rented GPU, measured by `mecha exp` against the base model, is the
path the evidence supports; RL comes after, if at all.**

---

## 1. What the corpus says first ✅

```
$ mecha sessions health
591 run(s) across 767 session(s) · 86 smoke-test session(s) hidden
  stop cause      completed 498 · max_turns 11 · interrupted 47 · stopped 32 · shutdown 2 · loop 1
  ended on a failed call   11 (1.9%)
  tool calls      2797 · errors 200 (7.2%) · denied 164 · staged 56
  malformed args 2 · blocked sends 64 · compactions 1
by model
  qwen3.6-35b-a3b               490 run(s)   tool errors   7.5%   ended on failure   2.0%
  qwen3.6-35b-a3b-uncensored    101 run(s)   tool errors   6.7%   ended on failure   1.0%

$ mecha sessions successes
owner-verified successes: 15 standing (question_answered 1, sent_unchanged 7, task_done 7), 0 withdrawn by a reopen, 31 unknown
```

Two readings follow.

- **Format is solved.** Two malformed-argument calls in 2,797. The
  documented benefit of SFT for tool use is mostly format and schema
  adherence, and here it has nowhere left to go. Training has to target
  *behaviour*: the 200 tool errors, 164 denials, 64 blocked sends and 11
  `max_turns` runs. None of those has been classified by cause. The 164
  are not all the owner's: `RunStats::tool_denied` counts "a human or a
  policy refusing", and the split lives only in the result text, three
  ways: `"Denied by the user:"`, `"Blocked by policy:"`, and a `pre_tool`
  hook's `"Blocked by a hook:"`. A
  failure the harness or prompt causes is cheaper to fix with
  `harness ruminate` than with weights.
- **The owner-labelled signal is tiny.** Fifteen verified successes, and
  the human share of the 164 denials, are not a training set. They are an evaluation set and
  a source of preference pairs at the decision points
  `mecha sessions compare` already extracts. The training data has to be
  manufactured (§4).

---

## 2. The model ✅

[Qwen/Qwen3.6-35B-A3B](https://huggingface.co/Qwen/Qwen3.6-35B-A3B),
Apache-2.0, released 2026-04-15.

- **Architecture.** `Qwen3_5MoeForConditionalGeneration`, the same class
  as Qwen3.5-35B-A3B. 40 layers; `full_attention_interval: 4`, so three
  Gated DeltaNet layers to every gated-attention layer. 256 experts, 8
  routed plus 1 shared. One MTP layer. A vision encoder. 262K native
  context.
- **Tool-call format.** The template emits Qwen's coder-style XML
  (`<tool_call><function=NAME><parameter=ARG>…`), not Hermes JSON. Tool
  results go back in a *user* turn wrapped in `<tool_response>`.
- **Thinking across turns.** Reasoning is kept only for assistant turns
  after the last real user query unless `preserve_thinking=true`. Training
  data must be rendered exactly as `scripts/start-router.sh` renders
  requests, or the model is trained on a context it never sees.
- **No technical report.** The card links a blog only. Qwen's published
  post-training recipe is still the Qwen3 report
  ([arXiv 2505.09388](https://arxiv.org/abs/2505.09388)).

---

## 3. The evidence on SFT, RL and harness-specific training

| Source | Setup | Finding |
|---|---|---|
| SFT or RL for tool-calling agents ✅ [arXiv 2609.17848](https://arxiv.org/html/2609.17848v1) | Qwen3 0.6B–32B; xLAM, ToolACE, Glaive; mostly single-turn | LoRA SFT best in **15 of 18** in-distribution settings and never below zero-shot. GRPO won 29 of 54 transfer settings by **under 1 point**. SFT→GRPO rarely best and fell below zero-shot 3 times. At 4B, **LoRA beat full fine-tuning** (88.0 vs 80.4) and full FT fell below zero-shot. A mixed dataset stayed within 1–5 points of the best specialist; dataset choice alone moved results up to 55 points. Recipe: rank 32, alpha 64, lr 1e-4, 2 epochs, loss on assistant turns only |
| Harness-Zero ✅ [arXiv 2609.24974](https://arxiv.org/abs/2609.24974) | Qwen3.5-9B; a stronger model corrects each step *inside the target harness's action space* | 1,269 accepted trajectories, LoRA SFT 2 epochs, reviewer reasoning masked from the loss. Macro **23.3 → 44.3**, above the base model *with* the specialised harness (41.7). Plain teacher rollouts scored 12, corrected ones 30. One task regressed; no forgetting evaluation. Already in `HARNESS-EVOLUTION-DESIGN.md`'s table: "context management cannot be distilled" |
| RLM-Qwen3-8B ✅ [arXiv 2512.24601](https://arxiv.org/abs/2512.24601) | Trajectories from Qwen3-Coder-480B acting as an RLM over 750 tasks | 2,250 sampled, 1,072 kept (score 0 and single-turn dropped); format errors repaired in code before training; **one sample per root turn with full history**; prime-rl, batch 64, 300 steps, ~48 H100-hours; median **+28.3%** |
| Agentic RL in 2026 ✅ [HF blog](https://huggingface.co/blog/sergiopaniego/agentic-rl-2026) | Survey | Labs run GRPO variants (DAPO, Dr.GRPO, GSPO, CISPO); RL specialist teachers, then on-policy-distil them into one model; warns of entropy collapse and reward hacking. Silent on multi-turn credit assignment and MoE |
| GSPO 📰 [arXiv 2507.18071](https://arxiv.org/abs/2507.18071) | Sequence-level importance ratio | "Notably stabilizes MoE RL training" |
| Injection robustness 📰 [StruQ](https://arxiv.org/abs/2402.06363), [SecAlign](https://arxiv.org/abs/2410.05451), [Instruction Hierarchy](https://arxiv.org/abs/2404.13208), [Meta SecAlign](https://arxiv.org/abs/2507.02735) | Train the model to ignore instructions in the data channel | Meta SecAlign reports transfer to InjecAgent and AgentDojo; a Qwen2.5-14B reproduction still had 14.5% attack success on AgentDojo (secondary source) |

Three conclusions carry to mecha:

1. **LoRA SFT on corrected, harness-native trajectories is the measured
   win.** Both Harness-Zero and RLM-Qwen3-8B got large gains from around a
   thousand trajectories, with no RL.
2. **The correction matters more than the teacher.** Harness-Zero's
   uncorrected teacher rollouts were worse than the student (12 vs 23.3).
3. **RL is unproven at this shape.** The SFT-vs-RL study is single-turn and
   dense; nothing found measures GRPO on a long multi-turn, thinking-mode,
   hybrid-attention MoE agent.

---

## 4. Where training data would come from

- **Fixture environments, not the owner's mail.** Run the local model on
  eval tasks built on the fictional cast (`trial_env.rs`, the eval rig),
  keep what the case graders pass (rejection sampling), and add
  Harness-Zero corrections with a stronger model reviewing each step in
  mecha's own tool schema. Real mail never goes to a teacher API.
- **The owner's decision points as preference pairs.** A human denial, a
  steer, a rewritten draft: `sessions compare` already finds them. A
  denial counts only when it carries `"Denied by the user:"`, as
  `pointwise.rs` keys it. A policy block is the harness working, never a
  correction to train on. Too few for
  SFT; enough for a held-out evaluation and possibly a small DPO pass.
- **Filtered by recorded taint, as `learn` is.** Weights are a
  longer-lived injection path than a learned rule, which rides in one
  prefix; a trajectory whose `Origin` is not clean does not enter the set,
  with no knob — the same rule, for the same reason.
- **Private data is memorised.** Any session data that does enter makes
  the weights a private artifact under the same rule as the owner's
  LoRAs: never in a public repo. Whether real sessions enter at all is an
  owner ruling.
- **Rendered with the router's template**, one sample per assistant turn,
  loss on the target turn only, the XML validated. A general and reasoning
  mix alongside (LoRA plus a mix is the study's forgetting guard; Unsloth
  advises ≥75% reasoning-style examples to keep thinking 📰).

---

## 5. Compute

| Tool | Qwen3.5/3.6 MoE | Note |
|---|---|---|
| Unsloth | yes 📰 | bf16 LoRA on 35B-A3B needs **~74 GB** 📰; advises against QLoRA for Qwen3.5 MoE; router frozen by default; GGUF export |
| LLaMA-Factory | ✅ v0.9.5 lists Qwen3.6 27B/35B | NVIDIA publishes a DGX Spark playbook |
| Axolotl | ✅ README | |
| NeMo-RL | ✅ GRPO recipe for Qwen3.5-35B-A3B | **2 nodes × 8 GPUs** |
| prime-rl | ✅ `qwen3_5_moe` with EP/CP, LoRA, multi-turn | What the RLM authors used |
| slime, AReaL | ✅ list Qwen3.5/3.6 | Megatron-based |
| verl, SkyRL, rLLM, ART | not confirmed for the 35B-A3B | |
| torchtune | no | development halted in 2025 |

- **On this box.** GB10, aarch64, 121 GiB unified. Of that, 72 GiB was in
  use by live services when checked. A 74 GB LoRA run means stopping the
  router and ComfyUI, which collides with the scheduled `:8080` jobs.
  Gated DeltaNet kernels on sm_121 are unverified: PyPI `causal-conv1d` and
  `mamba-ssm` break on aarch64, and one community wheel exists built for
  exactly this model ([N8Programs/dgx-causal-conv1d-wheel](https://huggingface.co/N8Programs/dgx-causal-conv1d-wheel)).
  **On-box SFT is possible but disruptive. On-box RL is not realistic**,
  because RL needs the bf16 policy and a rollout engine resident together.
- **Rented.** One H200 or B200 for a few hours covers LoRA SFT; an
  estimate of a few hundred dollars ❓. RL at NeMo-RL's own recipe is 16
  GPUs, around $35–50 an hour at typical H100 prices ❓.
- **A known trap** 📰 ([Osmosis](https://osmosis.ai/blogs/unlocking-lora-moe-rl-for-qwen3-5)):
  transformers 4.57.1 silently mis-configures `qwen3_5_moe` and corrupts
  rollouts; ≥5.2 is required. The model's own `config.json` names 4.57.1.

---

## 6. Serving the result

- **Merge, then convert.** Merge the LoRA into bf16, run
  `convert_hf_to_gguf.py` (which registers
  `Qwen3_5MoeForConditionalGeneration`), and requantise to Q4_K_M with a
  fresh imatrix. Runtime LoRA adapters are riskier: `convert_lora_to_gguf.py`
  expects `.lora_A`/`.lora_B` tensor names that a fused-expert PEFT adapter
  may not produce ❓.
- **The MTP head does not survive training** 📰: the HF trainer neither
  loads nor saves `mtp.*` tensors. Graft the base model's head back, as
  already done for the uncensored arm (`LLAMA-SERVER.md`). The head is then
  untrained against the new weights, and draft acceptance will drop by an
  unmeasured amount.
- **A new router preset, never a replacement.** The trained model is an arm
  in `mecha exp` against the base; promoting it is the owner's call (a lane
  must not promote itself).

---

## 7. The path, in order

0. **Classify the failures.** The 200 tool errors, 164 denials (split
   human from policy first), 64 blocked
   sends and 11 `max_turns` runs, by cause: harness, prompt, tool, or
   model. This costs nothing and decides whether training has a target.
1. **Build the data** from fixtures (§4), around 1–5k trajectories plus the
   mix, rendered with the router's template.
2. **LoRA SFT**, rank 32, attention and experts, router frozen, bf16, on one
   rented GPU.
3. **Serve and measure**: merge, convert, graft MTP, Q4_K_M, a new preset,
   `mecha exp` against the base on held-out fixtures plus an AgentDojo-style
   injection suite. Injection training is defence in depth only, never a
   reason to loosen the interlock.
4. **RL only on a plateau.** GSPO, rented, prime-rl or slime; or on-policy
   distillation from a stronger teacher. `replay_run` as an environment is
   faithful only until the policy diverges from the recording;
   `ROLLOUT-DESIGN.md`'s dry world is the better environment if it is
   built.

If `CODE-MODE-DESIGN.md` ships, its programs are a natural training target
too. RLM-Qwen3-8B is the precedent: a model trained on its harness's REPL
idiom gained more than any of the other numbers here.

---

## 8. Not established

- LoRA memory use and throughput for this model on the GB10. No source
  measured it; the 74 GB figure is the vendor's, for Qwen3.5.
- Whether the linear-attention Triton kernels are correct and fast on
  sm_121, or fall back to something very slow.
- Whether a fused-expert LoRA converts cleanly with llama.cpp's converter.
- How much draft acceptance a grafted, untrained MTP head loses.
- Whether the SFT-vs-RL results hold for long multi-turn, thinking-mode,
  hybrid MoE agents.
- Whether training with `preserve_thinking` on or off changes the result.
- Whether SecAlign-style gains hold at 3B active parameters.
