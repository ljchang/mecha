---
title: Sandbox
sidebar_position: 7
description: Confining shell with bwrap, docker, or landlock, and the three rules that make the confinement mean something.
---

# Sandbox

Every other tool is bounded by the [path jail](/docs/features/security):
`ToolCtx::resolve` proves a path stays inside the workspace before anything
touches it. `shell` cannot work that way. The jail cannot see inside
`bash -c`, and a command is free to `cd /`, read `~/.aws/credentials`, and
`curl` it out.

The capability model has always said as much — `shell` declares
`private_data`, `external_send` and `destructive` — but saying it is not
enforcing it. `sandbox.rs` is the enforcement.

## Configuration

```toml
[sandbox]
kind = "bwrap"              # none | bwrap | docker | landlock
network = false             # confined commands reach the network
writable = []               # extra paths mounted read-write
readable = ["/opt/toolchain"]   # extra paths mounted read-only
env = ["CARGO_HOME"]        # environment variables passed through, by name
image = "debian:stable-slim"    # docker only
memory_mb = 2048            # docker only
cpus = 2.0                  # docker only
```

| `kind` | What it does |
|---|---|
| `none` | The default. Commands run directly, as you, with your credentials. The only sane default for a supervised CLI on a machine where the alternatives may not be installed. |
| `bwrap` | User namespaces via `bwrap`. Cheap — no daemon, a few milliseconds per command. |
| `docker` | A throwaway container (`--rm`). Works where user namespaces are locked down; costs a container start per command. |
| `landlock` | Landlock LSM rules applied in the child between fork and exec. Needs no privilege at all, so it works where bwrap fails even installed (Ubuntu 23.10+'s AppArmor userns switch). Kernel 6.2+ required — older ABIs cannot restrict truncation, and half-confinement is refused rather than served. |

`mecha tools` prints the active sandbox, including whether the network is on
and whether reads can leave the workspace.

### What landlock deliberately does not claim

Landlock confines **files**: workspace writable, system read-only, your home
directory denied wholesale. It cannot close the network — TCP is denied on
6.7+ kernels as defense in depth, but UDP is unrestrictable at any ABI, and
`echo x > /dev/udp/host/port` works in bash alone. So a landlocked `shell`
**never earns the trifecta interlock's relaxation**: it stays an
`external_send` sink whatever `network` is set to, and the denial message
says so rather than pointing at a setting that would not help. Its preflight
proves the denial, not just the apply: it plants a file in your real home
and requires the confined read to *fail* — "confined" with nothing denied is
the state the preflight exists to forbid. Three known ways it is weaker than
bwrap: `/tmp` is shared rather than private, `/proc` shows every process,
and there is no PID/IPC isolation.

## What a confined command gets

- **The workspace**, mounted writable, plus anything named in `writable`.
- **A read-only system** — `/usr`, `/etc`, `/opt`, and the `/bin`, `/lib`
  family (bound as directories or recreated as symlinks, depending on whether
  the host has a merged `/usr`), plus anything named in `readable`.
- **A private `/tmp`**, so one command cannot leave anything behind for the
  next or read what the last one left.
- **No home directory.** `HOME` is set to the workspace.
- **No environment except an allowlist.** `bwrap` gets `--clearenv`, then a
  fixed `PATH`, then only the variables named in `env`. API keys live in the
  environment; a confined command that inherits them is confined in the least
  interesting way.
- **No network**, unless `network = true`.

Under `bwrap` the process also gets `--die-with-parent` (a wedged command
cannot outlive mecha) and `--new-session` (which detaches the controlling
terminal, blocking the TIOCSTI trick of pushing characters into the parent's
input queue). Under `docker` it runs as your uid/gid — root inside the
container writing into a bind-mounted workspace would leave root-owned files
you then cannot delete — with `--cap-drop ALL` and
`--security-opt no-new-privileges`.

The same machinery confines **MCP servers**, which need it more: an MCP server
is third-party code running on your machine, where `shell` at least runs
commands a model asked for out loud. `wrap_argv` confines an explicit argv
with no shell in between, because quoting caller-supplied arguments into
`bash -lc` is a command-injection bug waiting for one entry with a space. A
server's `network` setting overrides the global one, so one server can reach
its own API without opening `shell` to satisfy it.

This is not a boundary against a determined kernel exploit. It is the
difference between "an injected command can read your SSH keys" and "an
injected command can read the files you pointed the agent at" — which is the
difference that decides whether an agent can safely be woken by an email.

## Three rules

### 1. A configured sandbox that does not work stops the run

`Sandbox::preflight` runs a real command through the real backend at startup
and checks for its marker in the output. A failure is an error with
instructions, not a warning, and `shell` is refused rather than run
unconfined.

```
sandbox preflight failed — refusing to run `shell` unconfined
```

Silently falling back to unconfined execution is worse than having no sandbox
at all, and not only because the operator writes policy on a false belief:
**`shell` declares narrower capabilities when confined, and the interlock
believes it.** A degraded sandbox would leave the loop treating a fully
capable shell as one that cannot reach the network.

The check happens once at startup rather than on the first tool call, so a
misconfiguration is a clear message at launch instead of a confusing tool
error twenty turns in. If the sandbox cannot be built at call time either, the
call is refused with "Nothing was executed" rather than falling through.

### 2. Only `external_send` narrows; `private_data` stays true

```rust
Capabilities {
    private_data: true,
    untrusted_input: false,
    external_send: self.sandbox.can_reach_network(),
    destructive: true,
}
```

No network means no way out, so a confined `shell` genuinely stops being a
trifecta sink and the interlock stops refusing outbound-looking work that
provably cannot go anywhere. That relaxation is earned by `preflight`: the
label narrows only because something else enforces it.

`private_data` does **not** narrow. A confined shell still reads the
workspace, and `fs_read` — which reads exactly the same bytes — is marked
private on the grounds that your files are the definition of private data.
Narrowing it here would open a hole rather than close one: `shell: cat
secrets` would set no taint where `fs_read: secrets` does, and the cheapest
route around the interlock would be to use the more dangerous tool.

Note that an extra `readable` or `writable` bind puts data back in reach.
`reaches_beyond_workspace()` reports true whenever either list is non-empty,
because an extra bind is exactly how private data gets back in scope.

### 3. The policy lives on the tool, not in `ToolCtx`

`capabilities()` takes no context, so it has nothing to consult — the sandbox
is handed to the tool when the registry is built:

```rust
let registry = Registry::new().with_builtins(&cfg.tools, Arc::clone(&sandbox));
```

The workspace still comes from `ToolCtx` at call time, so a per-run jail — an
eval case's private fixture copy, a batch item's directory — is what actually
gets mounted. Confinement policy is fixed per process; what it is pointed at
is per run.

## Ubuntu 23.10 and later

:::warning
On Ubuntu 23.10+, `bwrap` fails even when it is installed and
`kernel.unprivileged_userns_clone` is 1, because AppArmor gained a separate
switch that nothing mentions:

```bash
sysctl kernel.apparmor_restrict_unprivileged_userns   # 1 means blocked
```

Either install an AppArmor profile for `bwrap`, set
`kernel.apparmor_restrict_unprivileged_userns=0` (system-wide, and weaker), or
use `kind = "docker"` instead.
:::

`preflight` recognises this case and prints that advice with the failure,
along with the other common ones: a `bwrap` that cannot configure loopback in
a new network namespace, and a docker socket you do not have permission to
use.

## Where to go next

- [Security model](/docs/features/security) — what the capability labels feed.
- [Tools and MCP](/docs/features/tools-and-mcp) — confining servers, and the environment allowlist.
- [Configuration reference](/docs/reference/configuration) — every `[sandbox]` key.
