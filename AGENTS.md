# AdaptTUI Agent Rules

AdaptTUI is a Rust terminal client for Adapt's hosted MCP server. The domain glossary is [CONTEXT.md](./CONTEXT.md); keep it glossary-only. Record hard-to-reverse, surprising trade-offs in [docs/adr](./docs/adr).

Use the Nix development shell for local commands:

```sh
nix develop
```

Project conventions:

- Keep the first release read-only. Reject MCP capabilities that cannot be verified as non-mutating.
- Keep Adapt integration behind a client adapter; the TUI should not own MCP protocol details.
- Never write bearer tokens to transcripts, logs, or terminal output.
- Store local session history under `.adapt/sessions/`, separate from credential configuration.
- Use gitmoji-style commit subjects when commits are requested.
- Run `just check` before handing off Rust changes.

