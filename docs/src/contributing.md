# Contributing

This page covers the essential code style rules and contribution process. For the full guide, see [CONTRIBUTING.md](https://github.com/user/cronymax/blob/001-webgpu-terminal-app/CONTRIBUTING.md) in the repository root.

## Code Style Rules

### File size: 500 lines max

No `.rs` file in `src/` may exceed 500 lines. CI enforces this via `scripts/check-line-count.sh`.

To decompose a large file:
1. Convert `module.rs` to `module/mod.rs`
2. Extract logical groups into sibling files
3. Use `pub(super)` for internal APIs
4. Re-export public items from `mod.rs`

### Function arguments: 5 max

Functions should accept at most 5 parameters. For complex signatures, use struct parameters.

### Clippy: zero warnings

```bash
cargo clippy -- -D warnings  # Must exit 0
```

### Format: rustfmt

```bash
cargo fmt --check  # CI check
cargo fmt          # Auto-format
```

## Running Tests

```bash
cargo test                    # All tests
cargo test --test all         # Integration + unit via tests/all.rs
cargo test unit::channel_test # Single test module
```

## PR Checklist

Before submitting a pull request:

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `cargo test` passes
- [ ] `bash scripts/check-line-count.sh` passes
- [ ] No new files exceed 500 lines
