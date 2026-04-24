# Behaviour Decisions

Confirmed design choices that differ from the Fish implementation or require
explicit documentation to avoid re-opening as bugs.

---

## fgen: Template Substitution Rules (Finding 2)

**Decision**: Keep the current `String::replace`-based substitution.

The Fish and Rust implementations are equivalent. The four replacement pairs are:

| Placeholder | Replaced with |
| ----------- | ------------- |
| `org.dev`   | `{ORG}.{ENV}` |
| `org.env`   | `{ORG}.{ENV}` |
| `org_dev`   | `{ORG}_{ENV}` |
| `org_env`   | `{ORG}_{ENV}` |

**Why not `{ORG}` / `{ENV}` literal placeholders?**
`~/.ssh/template.conf` must remain a valid `ssh_config` that can be tested
with `ssh -F template.conf` before substitution. Curly-brace tokens are not
valid ssh_config syntax.

**Template naming rules**:

- Use `org.dev.*` / `org.env.*` as `Host` patterns.
- Use `org_dev` / `org_env` as identifiers (e.g. key paths like
  `~/.ssh/conf.d/keys/org_dev_id_ed25519`).
- Avoid words that contain `org.dev` / `org_dev` as a substring
  (e.g. `org_developer`) — they will be unintentionally substituted.

---

## Logging: `@fterm_logging` Key Name (Finding 3)

**Decision**: Keep `set-option -p @fterm_logging` (pane-scoped).

| Aspect          | Fish                                   | Rust                      |
| --------------- | -------------------------------------- | ------------------------- |
| Key name        | `@{session}_{window}_{pane}`           | `@fterm_logging`          |
| Scope           | global tmux option                     | pane-scoped (`-p`)        |
| Cleanup on exit | manual unset required (leaks on crash) | automatic on pane destroy |

**Incompatibility**: If you have a `status-right` or script that reads the
Fish-style key `@{session}_{window}_{pane}`, it will not work with the Rust
version. Use `#{@fterm_logging}` (pane-scoped interpolation) instead.

**Teardown**: `stop::stop` delegates to `finalize_logging`, which unsets
`@fterm_logging` _after_ gzip completes. This ordering ensures that any
process querying the option can rely on the `.log` file still existing while
the option is set. Automatic pane-destroy cleanup provides a secondary
safety net.

---

## ssh: `-i` Flag Skips Agent Check (Finding 4)

**Decision**: Keep the Rust behaviour (skip `ssh-agent` check when `-i` is given).

| Scenario                            | Fish             | Rust                             |
| ----------------------------------- | ---------------- | -------------------------------- |
| No `-i`, no agent running           | abort with error | abort with error                 |
| `-i /path/to/key`, no agent running | abort with error | **proceed** (agent not required) |

**Rationale**: When an explicit identity file is provided, agent forwarding is
not used, so a missing `ssh-agent` is not a blocker. This enables CI pipelines
and local testing with a key file without running an agent.

**Note**: The splash screen's `matched_keys` field will be empty when `-i` is
used. A dedicated display for the `-i` key path can be added separately if
needed.

---

## SSH Helper Timeout: `SSH_HELPER_TIMEOUT_SECS = 1` (Finding 10)

**Constant**: `SSH_HELPER_TIMEOUT_SECS` in `crates/fterm/src/external.rs`

**Value**: `1` second

**Applies to**:

- `ssh_resolve` (`ssh -G`) — resolves host config
- `ssh_agent_list` (`ssh-add -l`) — lists agent keys
- `ssh_keygen_fingerprint` (`ssh-keygen -lf`) — reads key fingerprint

**Does NOT apply to**:

- `exec_with_config` — the live `ssh` / `scp` session (user is interacting)
- `exec_passthrough` — the `ssh-add` / `ssh-keygen` sub-commands (user-facing)

**Why 1 second?**
Matches the Fish `__fterm_run_ssh_cmd` wrapper (`timeout --foreground
--kill-after=1 1`). The gpg-agent or ssh-agent can freeze when forwarding is
active, which would hang the terminal indefinitely. A 1-second hard kill
ensures the terminal stays responsive regardless of agent state.

**Implementation note**: `RealCommandRunner::run` uses a 50 ms poll loop
followed by `child.kill()` + `child.wait()`. This achieves the same outcome
as Fish's `timeout --kill-after=1` (kill after 1 s), though the mechanism
differs slightly.
