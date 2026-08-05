## What and why

<!-- What changed, and the reasoning a reviewer cannot reconstruct from the diff. -->

## How it was verified

<!-- Which of these actually ran, and what they showed. A fix should fail on the
     old behaviour — say how you established that. -->

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --all-targets --all-features`
- [ ] `cargo fmt --all --check`
- [ ] Tried it by hand (say how)

## Checklist

- [ ] A user-visible change has a `## [Unreleased]` entry in `CHANGELOG.md`
- [ ] A new `Config` field also exists on `ConfigLayer` (otherwise its TOML table
      becomes a startup parse error while every unit test stays green)
- [ ] Every model-supplied path goes through `ToolCtx::resolve`
- [ ] A tool that reaches the network calls `.from_outside()`
- [ ] Nothing that used to fail closed now fails open
- [ ] Documentation under `website/docs/` updated if behaviour changed
