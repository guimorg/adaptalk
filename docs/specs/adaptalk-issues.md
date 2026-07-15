# AdaptTUI Local Issue Plan

These are the approved tracer-bullet implementation slices for AdaptTUI. They are intentionally local planning artifacts; no Linear issues or labels have been created.

## ATUI-1 — First-run Adapt MCP connectivity

**Type:** HITL  
**Blocked by:** None  
**User stories:** 1–9, 31–35

### What to build

Connect AdaptTUI to Adapt's hosted MCP Server using the configured bearer token. Provide the default Adapt endpoint, a local endpoint override, clear configuration/authentication errors, MCP initialization, capability discovery, and a terminal-visible connection status. Keep the transport behind the Adapt Client boundary.

### Acceptance criteria

- [ ] A user can configure a bearer token in the local `.adapt` configuration area.
- [ ] The default Adapt endpoint is used when no override is configured.
- [ ] A configured endpoint override is honored.
- [ ] The token is sent only as authentication data and never rendered or logged.
- [ ] Missing, malformed, and rejected credentials produce distinct user-facing errors.
- [ ] MCP initialization completes before AdaptTUI reports the connection as ready.
- [ ] Available MCP capabilities are discovered and represented at the Adapt Client boundary.
- [ ] A deterministic fake MCP server/transport covers success and failure paths.
- [ ] The live Adapt endpoint, authentication headers, and initialization contract are verified with a human before the slice is considered complete.

## ATUI-2 — Read-only Adapt query

**Type:** AFK  
**Blocked by:** ATUI-1  
**User stories:** 10–11, 16–18, 33–35

### What to build

Submit one Chat Prompt through the Adapt Client, select a verified non-mutating MCP capability, invoke it, and render the completed structured response. Capabilities that are mutating or cannot be confidently classified must be rejected or hidden.

### Revised implementation plan

Adapt currently exposes `ask_adapt`, which is not a verified read-only capability. ATUI-2 therefore remains fail-closed and does not invoke it. The client keeps the query seam and requires both an explicit `readOnlyHint: true` annotation and an Adapt-specific verified capability allowlist entry. The allowlist remains empty until Adapt documents and verifies a genuinely non-mutating capability.

### Acceptance criteria

- [ ] A user can enter and submit a natural-language Chat Prompt.
- [ ] The submitted prompt appears in the conversation transcript.
- [ ] The client invokes a verified read-only capability and returns its result.
- [ ] Mutating capabilities are not exposed or invoked.
- [ ] Capabilities with ambiguous mutation behavior fail closed.
- [ ] No action approval or write-operation path exists in this slice.
- [ ] Structured response content is rendered without leaking authentication data.
- [ ] Client errors are visible and distinguishable from successful responses.
- [ ] Fake-transport tests cover read-only filtering, invocation, structured results, and rejection behavior.

## ATUI-3 — Interactive Terminal REPL

**Type:** AFK  
**Blocked by:** ATUI-2  
**User stories:** 12–15, 28–30, 33–34

### What to build

Build the simple Crossterm Terminal REPL around the client boundary. Use normal terminal input and native scrollback, show a typing/completion fallback, and display structured results and citations inline. Until Adapt's progress semantics are verified, completed responses use local mock streaming by default and `/stream` can disable it.

### Acceptance criteria

- [ ] The terminal presents a line-oriented prompt and native scrollback conversation output.
- [ ] Complete responses and structured results render with speaker-identifying output.
- [ ] A response has a clear loading and completion state.
- [ ] Structured results and citations are visible inline with the relevant response.
- [ ] Native terminal scrolling remains available for long responses and prior prompts.
- [ ] Connection, authentication, and MCP errors render without exposing secrets.
- [ ] Normal terminal exit leaves the terminal in its normal mode.
- [ ] REPL tests assert user-visible behavior rather than terminal-control internals.

## ATUI-4 — Local Adapt Session history

**Type:** AFK  
**Blocked by:** ATUI-3  
**User stories:** 19–22, 25–27

### What to build

Persist Adapt Sessions as local transcripts under `.adapt/sessions/`. Save completed and interrupted sessions, provide session browsing and local transcript reopening, keep credentials separate, and ensure secrets never enter history.

### Acceptance criteria

- [ ] Each completed Adapt Session is persisted under `.adapt/sessions/`.
- [ ] An interrupted session is persisted without silently losing its available transcript.
- [ ] A user can browse saved sessions and select one.
- [ ] A selected session can be reopened as a local transcript without a live connection.
- [ ] Session records preserve prompts, response content, structured results, citations, and relevant timestamps.
- [ ] Credential configuration is stored separately from session records.
- [ ] Bearer tokens are absent from transcripts, logs, and rendered output.
- [ ] Credential files use restrictive permissions where supported.
- [ ] Temporary-directory tests cover creation, reopening, interruption, separation, and redaction.

## ATUI-5 — Remote Adapt Session resume

**Type:** HITL  
**Blocked by:** ATUI-1 and ATUI-4  
**User stories:** 23–24

### What to build

Preserve a remote Adapt session identifier when Adapt provides one. When a saved session has a valid remote identifier, attempt remote continuation; when the MCP contract does not provide or accept one, reopen the local transcript and start a new remote Adapt Session with no implicit transcript injection.

### Acceptance criteria

- [ ] Session records can preserve a remote Adapt session identifier when supplied.
- [ ] Reopening a resumable session attempts remote continuation.
- [ ] The user can distinguish remote continuation from local transcript reopening.
- [ ] Unsupported, expired, or rejected remote identifiers produce a clear fallback state.
- [ ] Fallback starts a new remote Adapt Session.
- [ ] Prior transcript content is not automatically injected into the new session.
- [ ] Explicit prior-context use is distinguishable from ordinary prompt submission.
- [ ] Tests cover both remote continuation and new-session fallback.
- [ ] Adapt's actual remote session semantics are verified with a human before acceptance.

## ATUI-6 — Security and release hardening

**Type:** AFK  
**Blocked by:** ATUI-2 and ATUI-4  
**User stories:** 4, 7, 17, 26–27, 31–32, 34

### What to build

Harden the read-only client and local persistence boundary for regular use. Add secret redaction, credential-file permission enforcement, safe configuration and MCP errors, fail-closed checks, and the repository's complete formatting, linting, test, and flake validation workflow.

### Acceptance criteria

- [ ] No bearer token appears in logs, error messages, transcripts, structured results, or terminal output.
- [ ] Credential files are created with restrictive permissions where the platform supports them.
- [ ] Permission failures are surfaced clearly rather than silently ignored.
- [ ] Unknown or ambiguous MCP capabilities remain blocked.
- [ ] Malformed configuration has actionable user-facing errors.
- [ ] Network, timeout, initialization, and server errors are rendered safely.
- [ ] Formatting, Clippy with warnings denied, tests, and Nix flake validation pass.
- [ ] The README documents local configuration, read-only behavior, session history, and the development commands.
- [ ] The release binary remains Adapt-specific and does not accept arbitrary MCP server configurations.
