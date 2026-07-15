# adapt-tui

A small Rust terminal REPL for using Adapt through its hosted MCP server.

> AdaptTUI is read-only by default: it exposes only MCP capabilities explicitly marked `readOnlyHint: true`. Mutating and ambiguously classified capabilities are rejected.

**Requirements:** Rust 1.85 or later (edition 2024).

The project is intentionally starting as a thin Adapt client:

- chat-first terminal interaction;
- completed responses rendered in the terminal;
- capability discovery through Adapt's MCP server;
- local session history under `.adapt/sessions/`;
- bearer-token configuration under `.adapt/`, separate from history.

## Installation

Download the latest release binary from the [Releases page](https://github.com/guimorg/adaptui/releases), then:

```sh
chmod +x adapt-tui
mv adapt-tui /usr/local/bin/
adapt-tui
```

Or build from source:

```sh
cargo install --git https://github.com/guimorg/adaptui
adapt-tui
```

## Development

With Nix:

```sh
nix develop
cargo run
```

Without Nix, you need Rust 1.85+, rustfmt, clippy, and `just`.

Run tests:

```sh
cargo test
```

Check everything (format, lint, tests):

```sh
just check
```

## Configuration

AdaptTUI reads its bearer token from `~/.adapt/config.toml`:

```toml
bearer_token = "paste-your-bearer-token-here"
# Optional; defaults to Adapt's hosted MCP endpoint.
endpoint = "https://app.adapt.com/mcp"
# Optional; mock response delay between word chunks, in milliseconds.
stream_delay_ms = 35
```

Create the directory and protect the file before adding your token:

```sh
mkdir -p ~/.adapt
touch ~/.adapt/config.toml
chmod 700 ~/.adapt
chmod 600 ~/.adapt/config.toml
```

To get a bearer token, sign in to Adapt and follow the token setup instructions in the [Adapt MCP Server documentation](https://adapt.com/docs/platform/mcp-server). Copy the token into `~/.adapt/config.toml`; do not commit the file or paste the token into source code, logs, or session history.

AdaptTUI requires an `https://` endpoint because the bearer token is sent as authentication data. The `endpoint` setting is optional and is intended for an HTTPS Adapt endpoint such as a staging environment.

## Using the Terminal REPL

After configuring a token, start the interactive REPL with:

```sh
cargo run
```

AdaptTUI first connects to the Adapt MCP Server and discovers capabilities, then prints a compact terminal prompt. Type at the `You ›` prompt and press Enter to submit it. Your prompt is cyan, Adapt replies are magenta, and the normal terminal scrollback preserves the conversation.

- Use your terminal's normal scrolling controls to browse the conversation.
- Press Ctrl-C to exit.
- AdaptTUI invokes only capabilities verified as read-only. If none are available, the terminal displays that error rather than invoking an ambiguous capability.

After a successful connection, AdaptTUI saves a redacted JSON snapshot after every prompt and response in `~/.adapt/sessions/` (next to, but separate from, `~/.adapt/config.toml`). A session left running by Ctrl-C or process termination is shown as **interrupted** and can still be read. The configured bearer token, bearer strings, and common sensitive JSON fields are redacted before anything is written.

Two REPL commands work entirely locally, before any configuration is read or MCP connection is made:

- `/history` lists saved sessions with their ID, status, and first-prompt summary.
- `/open <id>` saves the active transcript, clears the terminal, and renders the selected transcript. The next prompt starts a new local snapshot linked to it. With `--allow-unverified-ask-adapt`, that prompt also sends the saved Adapt chat ID to continue the remote conversation; normal read-only mode never does.
- `/stream` toggles mock response streaming for new replies. It is enabled by default and reveals completed responses in word-sized chunks; reopened transcripts render immediately.

Type `/` at the prompt to see these commands in a compact suggestion palette. Fuzzy matches can be accepted with Tab or Enter before supplying `/open`'s session ID. Mock streaming is a terminal presentation effect; real MCP progress-event streaming is not enabled yet.

The connectivity milestone initializes against Adapt's hosted endpoint and discovers its capabilities. The client query seam accepts a prompt, invokes only a selected verified read-only capability, and preserves structured MCP results for the terminal layer.

For development investigations only, `ask_adapt` can be enabled with an explicit process-only opt-in:

```sh
cargo run -- --allow-unverified-ask-adapt "your prompt"
```

The interactive REPL prints a development-mode warning because `ask_adapt` is not verified as read-only and may perform mutations. The flag is not stored in configuration, and no other unverified capability can be enabled by it.

Set `stream_delay_ms` in `~/.adapt/config.toml` to change the mock response pacing. It defaults to `35`; use `0` for immediate chunk rendering. The `/stream` command still controls whether chunked rendering is enabled for the current process.

## Troubleshooting

**"Authentication rejected" error**

Your bearer token is invalid or expired. Refresh it in `~/.adapt/config.toml` from your Adapt account. Check that the file is readable and that the token has no extra whitespace.

**"No read-only capability available"**

No verified read-only capabilities are currently exposed. This is intentional until Adapt documents and verifies a non-mutating operation. You can test with `--allow-unverified-ask-adapt` in development mode, but the interactive REPL will show this error in normal mode.

**Session history write fails with "Permission denied"**

Check that `~/.adapt/` exists and is writable:

```sh
ls -la ~/.adapt/
chmod 700 ~/.adapt
chmod 600 ~/.adapt/config.toml
```

The client creates `~/.adapt/sessions/` with restrictive permissions (0o700 directory, 0o600 files).

**"Cannot connect to MCP server" or timeout**

Check your network connection and that the endpoint is reachable. If using a custom endpoint, verify it's HTTPS and accepts the bearer token.

## Documentation

- [Domain glossary](./CONTEXT.md)
- [Architecture decisions](./docs/adr/)
