# adapt-tui

A small, read-only Rust chat terminal for using Adapt through its hosted MCP server.

The project is intentionally starting as a thin Adapt client:

- chat-first terminal interaction;
- streamed responses when the MCP transport supports them;
- read-only capabilities only;
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

The first implementation milestone is MCP connectivity: initialize against Adapt's hosted endpoint, discover tools, permit only verified read-only tools, and print one structured response.

## Documentation

- [Domain glossary](./CONTEXT.md)
- [Architecture decisions](./docs/adr/)

