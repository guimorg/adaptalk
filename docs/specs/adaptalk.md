# AdaptTUI Specification

## Problem Statement

Adapt is currently most convenient to use through its web and Slack surfaces. That makes it harder to use from a terminal-centric engineering workflow, inspect responses in a durable local history, and work with Adapt without switching applications.

The user needs a small Rust terminal client that provides a direct, chat-first connection to Adapt through Adapt's hosted Model Context Protocol service. The first release must be safe to use for investigation: it must be read-only, preserve useful session history locally, and make the remote interaction visible without attempting to reproduce the entire Adapt or Slack product.

## Solution

Build AdaptTUI, a thin Rust terminal REPL for Adapt.

AdaptTUI connects to Adapt's hosted MCP endpoint using a bearer token stored in the user's local `.adapt` configuration area. It initializes the MCP connection, discovers available capabilities, permits only capabilities verified as non-mutating, and presents a chat-first terminal interaction.

The Terminal REPL accepts natural-language prompts and prints colored conversation output into native terminal scrollback. It displays structured read-only results and citations inline, and shows a working state while a request is pending. Every Adapt Session is persisted under the local `.adapt/sessions/` area so users can browse and reopen history. Reopening a transcript must always work locally; true remote continuation is supported only when Adapt exposes a stable remote session identifier.

The first implementation milestone is a connectivity slice: authenticate, initialize, discover tools, enforce the read-only boundary, invoke one verified read-only capability, and render a structured response. The REPL should be built around the same client seam after that slice is proven.

## User Stories

1. As an engineer, I want to launch AdaptTUI from my terminal, so that I can use Adapt without opening Slack or a browser.
2. As an engineer, I want AdaptTUI to connect to Adapt's hosted MCP Server, so that Adapt remains responsible for connected-tool access and reasoning.
3. As an engineer, I want to configure a bearer token locally, so that I can authenticate without entering it for every session.
4. As an engineer, I want the bearer token kept separate from session history, so that sharing or inspecting transcripts does not expose credentials.
5. As an engineer, I want the default Adapt endpoint to work without extra configuration, so that the first run is straightforward.
6. As an engineer, I want to override the endpoint locally, so that I can test staging or a future endpoint without changing the binary.
7. As an engineer, I want AdaptTUI to fail clearly when credentials are absent or invalid, so that authentication problems are distinguishable from MCP failures.
8. As an engineer, I want AdaptTUI to complete MCP initialization before showing the chat as ready, so that requests cannot race an unready connection.
9. As an engineer, I want AdaptTUI to discover the server's capabilities, so that the client can adapt to the current MCP contract.
10. As an engineer, I want AdaptTUI to show a simple chat prompt input, so that natural-language questions are the primary interaction.
11. As an engineer, I want to submit a Chat Prompt and see it in the transcript, so that the conversation remains understandable while a response is pending.
12. As an engineer, I want response content to stream into the transcript, so that I can begin reading without waiting for the complete answer.
13. As an engineer, I want a clear loading or completion state when streaming is unavailable, so that non-streaming responses remain understandable.
14. As an engineer, I want AdaptTUI to render structured MCP results inline, so that tables, source references, and other useful data are not reduced to opaque raw JSON.
15. As an engineer, I want citations visible with the relevant response, so that I can inspect the source behind an answer.
16. As an engineer, I want AdaptTUI to permit only verified non-mutating capabilities, so that the first release cannot change external systems.
17. As an engineer, I want capabilities with unclear mutation behavior rejected or hidden, so that the read-only guarantee fails closed.
18. As an engineer, I want no action approval flow in the first release, so that the product boundary remains strictly read-only.
19. As an engineer, I want each completed Adapt Session saved locally, so that useful conversations survive process exit.
20. As an engineer, I want session history stored under `.adapt/sessions/`, so that it has a predictable and user-owned location.
21. As an engineer, I want to browse saved sessions, so that I can find previous investigations.
22. As an engineer, I want to reopen a saved transcript, so that I can read prior context even when remote continuation is unavailable.
23. As an engineer, I want to resume a remote session when Adapt provides a stable session identifier, so that I can continue a conversation without losing server-side context.
24. As an engineer, I want the client to explain when reopening starts a new remote session, so that local transcript reopening is not confused with server-side continuation.
25. As an engineer, I want interrupted sessions saved safely, so that a partially completed investigation is not silently lost.
26. As an engineer, I want secrets excluded from transcripts, logs, and terminal output, so that local history remains safe to inspect and back up.
27. As an engineer, I want the local credential file protected with restrictive permissions where supported, so that the bearer token is not broadly readable.
28. As an engineer, I want the Terminal REPL to remain visually simple, so that the interface feels like a focused conversation rather than a dashboard.
29. As an engineer, I want native terminal scrollback to retain long answers and previous prompts.
30. As an engineer, I want normal terminal exit behavior, so that quitting does not corrupt the terminal or lose the current session.
31. As an engineer, I want the project to provide a reproducible Rust development environment, so that contributors can build it consistently.
32. As an engineer, I want formatting, linting, and tests available through the development environment, so that basic quality checks are easy to run.
33. As an engineer, I want the MCP client boundary separated from the TUI, so that protocol changes do not require rewriting terminal rendering.
34. As an engineer, I want the client to expose useful errors without leaking credentials, so that failures can be diagnosed safely.
35. As an engineer, I want the initial connectivity slice to work before the full TUI is built, so that the highest-risk remote contract is validated early.

## Implementation Decisions

- AdaptTUI is a thin Adapt Client, not a reimplementation of Adapt and not a general-purpose MCP client.
- The first release is chat-first and presents a minimal Terminal REPL rather than a dashboard, tool browser, or command catalog.
- The first release is strictly read-only. Only verified non-mutating MCP capabilities may be exposed or invoked. Unknown or ambiguous capabilities fail closed.
- Mutating operations and approval workflows are out of the first release.
- Adapt's hosted MCP endpoint is the default endpoint. A local configuration override is supported for testing and future endpoint changes, while arbitrary third-party MCP servers remain out of scope.
- Bearer-token authentication is persisted in the user-local `.adapt` configuration area, separate from session history. Tokens are never written to transcripts, logs, or terminal output, and credential files use restrictive permissions where supported.
- Local Adapt History is persisted under `.adapt/sessions/`. History is a local transcript archive and may also carry a remote session identifier when Adapt supplies one.
- Reopening a local transcript is always supported. Remote continuation is conditional on a stable Adapt session identifier; without one, AdaptTUI starts a new remote Adapt Session and only uses prior context when explicitly requested.
- The MCP client uses the official Rust RMCP SDK and keeps transport/protocol details behind an Adapt-specific client adapter.
- The REPL uses Crossterm for terminal styling and cursor control. Tokio provides the asynchronous runtime needed for MCP work.
- The REPL shows `Adapt: is typing…` while a request is pending. Completed responses are mock-streamed into the terminal by default in word-sized chunks; `/stream` toggles this presentation behavior for the current process, and `stream_delay_ms` configures the pacing. Real MCP progress-event streaming remains a future client enhancement.
- The repository uses a minimal Nix flake with a pinned stable Rust toolchain, Cargo, rustfmt, Clippy, Rust Analyzer, and basic developer commands. The sibling project's container and Kubernetes tooling is not carried over.
- The highest test seam is the Adapt client boundary: a fake MCP server/transport should drive authentication, initialization, capability discovery, read-only filtering, tool invocation, response events, errors, and remote-session identifiers. The REPL should be tested at the user-visible behavior seam.

## Testing Decisions

- Tests should assert external behavior and observable client/REPL events, not private data structures or the exact MCP SDK calls used internally.
- The Adapt client adapter will be tested with a deterministic fake MCP server or transport covering successful initialization, invalid credentials, endpoint errors, capability discovery, read-only filtering, ambiguous capability rejection, structured results, citations, streaming chunks, non-streaming fallback, and remote session identifiers.
- Session persistence will be tested with an isolated temporary `.adapt` directory. Tests will verify transcript creation, reopening, interrupted-session persistence, explicit context behavior, credential/history separation, and secret redaction.
- REPL tests will cover prompt submission, loading states, response/error rendering, secret redaction, and normal terminal exit behavior without asserting terminal-control implementation details.
- Configuration tests will cover defaults, endpoint overrides, bearer-token loading, missing configuration, malformed configuration, and restrictive credential-file permissions where the platform supports them.
- Repository checks will include formatting, Clippy with warnings denied, unit/integration tests, and Nix flake validation.
- The existing repository has no comparable Rust test suite yet; the initial test style should therefore establish deterministic fake-transport tests and terminal behavior tests as the project's local prior art.

## Out of Scope

- Reimplementing Adapt's reasoning, integrations, connected-tool access, or Slack experience.
- Supporting arbitrary MCP servers or a general MCP server registry.
- Mutating MCP tools, action approval, write operations, or “approve all” session modes.
- A dashboard, graphical interface, tool catalog, command palette, or complex workspace navigation.
- Local LLM inference or an independent answer-generation model.
- Synchronizing session history across machines or accounts.
- Encrypting local history beyond the operating system's normal filesystem controls.
- Automatically injecting prior transcripts into new prompts.
- Assuming that a local transcript is equivalent to a live remote Adapt conversation.
- Deployment infrastructure, containers, Kubernetes, or a hosted service for AdaptTUI.

## Further Notes

- The repository was bootstrapped as a Git project with a working Rust/Cargo manifest and minimal Nix development environment.
- The current implementation compiles and passes formatting, Clippy, and tests. It includes MCP connectivity, capability discovery, the read-only policy boundary, and the native terminal REPL; session persistence and remote session resume remain planned work.
- The exact Adapt MCP endpoint contract, authentication headers, streaming behavior, tool mutation metadata, and remote session semantics must be verified against Adapt's current documentation and a live or deterministic test endpoint before implementation is considered complete.
- The spec is intentionally stored locally. It has not been published to Linear, per the user's instruction.
