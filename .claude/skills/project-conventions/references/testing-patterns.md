# Testing Patterns — fterm

## Test Organization

- Unit tests: `#[cfg(test)] mod tests` within each source file
- Integration tests: `crates/fterm/tests/integration_test.rs`
- Doc tests: inline `///` examples (run via `mise run test:doc`)
- `#![allow(clippy::unwrap_used)]` is permitted in test code

## Arrange / Act / Assert Pattern

All tests follow the AAA pattern:

```rust
#[test]
fn test_example() {
    // Arrange
    let input = "...";

    // Act
    let result = my_function(input);

    // Assert
    assert_eq!(result, expected);
}
```

## Miri Compatibility

For universal Miri rules and decision flowchart, see
`~/.claude/skills/rust-implementation/references/testing-miri.md`.

### Miri Annotation Pattern

Use `#[cfg_attr(miri, ignore)]` (runtime skip) — NOT `#[cfg(not(miri))]`
(compile-time removal). The `cfg_attr` form keeps the test compiled so
compile-time errors are caught; the test is simply skipped at runtime.

```rust
#[cfg_attr(miri, ignore)]
#[test]
fn test_with_env_mutation() { ... }
```

For tests that also use `#[serial(env)]`, the Miri guard goes above `#[test]`:

```rust
#[cfg_attr(miri, ignore)]
#[test]
#[serial(env)]
fn test_with_serial_and_env() { ... }
```

### Crate-Level Exclusions

| Crate  | Reason                         | Tests |
| ------ | ------------------------------ | ----- |
| (none) | No crates excluded at CI level | —     |

### Per-Test Skip Categories

1. **Process spawning (assert_cmd)** — 9 tests. Integration tests in
   `crates/fterm/tests/integration_test.rs` spawn the `fterm` binary via
   `Command`. Miri cannot trace across process boundaries.

2. **FFI / C bindings (nix)** — 1 test. `crates/fterm-ssh-config/src/validate/identity.rs`
   uses `nix::sys::stat::stat` (libc FFI).

3. **Environment variables (serial_test)** — tests using `#[serial(env)]`
   that mutate env vars with `std::env::set_var`. The `serial_test` crate
   uses `scc::HashMap` internally, which causes false-positive Miri leak
   errors. All `#[serial(env)]` tests carry `#[cfg_attr(miri, ignore)]`.

4. **File system (tempfile)** — tests using `tempfile::TempDir` for real
   filesystem operations. Real I/O has limited Miri support.

5. **Process spawning (RealCommandRunner)** — tests that invoke external
   commands via `RealCommandRunner`. Miri cannot execute `std::process::Command`.

### Statistics

| Metric                      | Count |
| --------------------------- | ----- |
| Total tests                 | 436   |
| Miri-ignored (per-test)     | 209   |
| Miri-compatible             | 227   |
| Miri-excluded (crate-level) | 0     |

## Serial Tests

Use `#[serial]` from the `serial_test` crate for tests that cannot run in
parallel (e.g., tests that mutate environment variables or shared state).
All `#[serial(...)]` tests must also carry `#[cfg_attr(miri, ignore)]`
because `serial_test`'s internal `scc::HashMap` is Miri-incompatible.

## Coverage Exclusions

Use `#[cfg_attr(coverage_nightly, coverage(off))]` for functions that cannot
be meaningfully tested. Requires `#![cfg_attr(coverage_nightly, feature(coverage_attribute))]`
at the crate root (currently in `fterm/src/main.rs` and `fterm-core/src/lib.rs`).

Excluded functions (paired with `NOTEST` comments):

| File                                 | Function                    | Category      |
| ------------------------------------ | --------------------------- | ------------- |
| `crates/fterm/src/main.rs`           | `fn main()`                 | `unreachable` |
| `crates/fterm/src/external.rs`       | `exec_passthrough()`        | `ffi`         |
| `crates/fterm/src/command/fssh.rs`   | `run_fzf_selection()`       | `infra`       |
| `crates/fterm/src/command/flog.rs`   | `run_fzf_log_selection()`   | `infra`       |
| `crates/fterm-core/src/util/path.rs` | `msys2_home()`              | `env`         |
| `crates/fterm-core/src/util/path.rs` | `resolve_win_ssh_command()` | `env`         |
| `crates/fterm-core/src/util/path.rs` | `to_win_mixed()`            | `env`         |

## OTel Test Tracing

Use `tracing-mock` for testing that spans and events are emitted correctly.
The `opentelemetry_sdk` dev dependency includes `features = ["testing"]` for
in-memory exporter-based assertions.

## SSH/SCP Integration Tests

Real SSH integration tests require `mise run ssh:setup` to configure test
SSH infrastructure in `tmp/.ssh/`. These tests are not part of the standard
`mise run test` suite.
