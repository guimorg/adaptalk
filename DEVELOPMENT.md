# Development Guide

## Layout

```
src/
  main.rs                    # REPL and CLI
  config.rs                  # Load and validate ~/.adapt/config.toml
  auth.rs                    # Validate bearer token; redact transport errors
  adapt_client.rs            # MCP client boundary
  session_history.rs         # Save/load sessions from ~/.adapt/sessions/
  conversation_controller.rs # Manage session state and redaction
  repl.rs                    # Terminal I/O with Crossterm
  redaction.rs               # Strip tokens from errors and output
  transcript.rs              # Response types and display

docs/
  specs/
    adaptalk-issues.md       # ATUI-1 through ATUI-6 slices
  adr/
    0001-*.md                # Architecture decisions
```

## Modules at a Glance

**config** loads `~/.adapt/config.toml` and validates it. Gives clear errors if the file is missing, malformed, or has bad values.

**auth** checks the bearer token with a preflight request. Redacts tokens from network errors so you don't leak credentials to stderr.

**adapt_client** wraps RMCP. Only exposes verified read-only capabilities. The list stays empty until Adapt documents what's actually safe.

**session_history** saves sessions to `~/.adapt/sessions/` as JSON. Strips the token before writing. Sets file permissions to 0o600 on Unix.

**conversation_controller** holds the session in memory and makes sure everything gets redacted before being saved or shown.

**repl** handles terminal I/O. Line-based input, styled output, typing indicators. Leaves the terminal clean when you exit.

**redaction** centralizes token stripping. Handles error messages, JSON responses, transcript text.

## Running Tests

All tests:
```bash
cargo test --all-targets
```

One module:
```bash
cargo test session_history --lib
cargo test config --lib
```

Watch a module while you work:
```bash
cargo watch -x "test --lib session_history"
```
(need `cargo install cargo-watch`)

## Adding a Read-Only Capability

1. Read `docs/adr/0004-fail-closed-read-only-capability-verification.md` first.
2. Add it to `VERIFIED_READ_ONLY_CAPABILITIES` in `adapt_client.rs`.
3. It needs `readOnlyHint: true` in the server's tool annotations.
4. Write a test that verifies both checks pass.
5. Update the README.

## Nix

Dev environment:
```bash
nix develop
```

Build with Nix:
```bash
nix build
# Binary at result/bin/adaptalk
```

## Releasing

1. Commit everything. Clean branch.
2. Tag: `git tag v0.2.0`
3. Push: `git push origin v0.2.0`
4. GitHub Actions builds it and creates a release.
5. Release-drafter has a draft (based on PR titles). Publish it.
6. Users download from Releases or run `cargo install --git https://github.com/guimorg/adaptui`.

## Common Commands

```bash
just check        # Everything before push
just fmt          # Auto-format
just build        # Debug
just release      # Optimized
cargo run         # Run it
```

The release build strips symbols, uses LTO, and aborts on panic. Stream delay is 35ms by default (configurable).

