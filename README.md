# adapt-tui

A small Rust chat terminal for using Adapt through its hosted MCP server.

> AdaptTUI is read-only: it exposes only MCP capabilities explicitly marked `readOnlyHint: true`. Mutating and ambiguously classified capabilities are rejected.

The project is intentionally starting as a thin Adapt client:

- chat-first terminal interaction;
- streamed responses when the MCP transport supports them;
- capability discovery through Adapt's MCP server;
- local session history under `.adapt/sessions/`;
- bearer-token configuration under `.adapt/`, separate from history.

## Development

Enter the pinned development environment with:

```sh
nix develop
```

Then run:

```sh
cargo run
just check
```

## Configuration

AdaptTUI reads its bearer token from `~/.adapt/config.toml`:

```toml
bearer_token = "paste-your-bearer-token-here"
# Optional; defaults to Adapt's hosted MCP endpoint.
endpoint = "https://app.adapt.com/mcp"
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

The connectivity milestone initializes against Adapt's hosted endpoint and discovers only verified read-only capabilities. The client query seam accepts a prompt, invokes a selected verified capability, and preserves structured MCP results for the terminal layer.

## Documentation

- [Domain glossary](./CONTEXT.md)
- [Architecture decisions](./docs/adr/)
