# ADR-0004: Fail closed until Adapt verifies a read-only capability

## Status

Accepted

## Context

Adapt currently exposes `ask_adapt`, but it is not guaranteed to be non-mutating. MCP `readOnlyHint` metadata is supplied by the remote server and is only a hint, so it cannot by itself establish the first-release safety boundary.

## Decision

AdaptTUI requires both an explicit `readOnlyHint: true` annotation and an Adapt-specific verified capability name before exposing or invoking a capability. The verified list is empty until Adapt provides a documented, verified non-mutating capability. `ask_adapt` is rejected by the normal query path even if it claims to be read-only.

As a temporary development exception, the process-only `--allow-unverified-ask-adapt` flag permits the narrowly scoped `ask_adapt` query method. The CLI must print a warning that the capability is not verified as read-only and may perform mutations. The flag does not authorize arbitrary unverified capabilities and is never persisted.

## Consequences

The default remains fail-closed, while development users have an explicit, visible escape hatch for `ask_adapt`. When Adapt provides a suitable verified capability, adding its name and contract tests will enable the normal query seam without weakening the boundary; the temporary exception can then be removed.
