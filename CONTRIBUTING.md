# Contributing to adaptalk

## Setup

You need Rust 1.85+ (edition 2024). Get it from [https://rustup.rs/](https://rustup.rs/).

**With Nix:**
```bash
nix develop
```
Everything is there.

**Without Nix:**
- Rust (1.85+) with rustfmt and clippy
- `just` — install with `cargo install just`
- `git`

## Working on the Code

```bash
just fmt          # Format code
just check        # Check formatting, lint, run tests
just build        # Debug build
just release      # Release build
cargo run         # Run it
```

Before you push, run `just check`. That's what CI runs too.

## Code Rules

No unsafe code. Clippy warnings are errors. No credentials or bearer tokens in test data — mock data goes in with a clear test-only marker.

## Commits

Use a gitmoji prefix:
- `✨` new feature
- `🐛` bug fix
- `♻️` refactoring
- `🔧` config or tooling
- `📝` docs
- `✅` tests
- `🎨` style
- `⚡` performance

```
✨ Add read-only query capability
🐛 Fix session history serialization error
```

## Pull Requests

Make a branch from `main`. Write a clear description of what you're changing and why. Reference any related issues.

Each PR should do one thing. If you're refactoring across multiple areas, bundle it intentionally and explain why.

Make sure `just check` passes before you push.

If you're implementing a slice from `docs/specs/adaptalk-issues.md` (ATUI-1, ATUI-2, etc.), reference it in the PR and check off the acceptance criteria.

## License

MIT. By opening a PR, you agree to that.
