# ADR-0004: Fail closed until Adapt verifies a read-only capability

## Status

Accepted

## Context

Adapt currently exposes `ask_adapt`, but it is not guaranteed to be non-mutating. MCP `readOnlyHint` metadata is supplied by the remote server and is only a hint, so it cannot by itself establish the first-release safety boundary.

## Decision

AdaptTUI requires both an explicit `readOnlyHint: true` annotation and an Adapt-specific verified capability name before exposing or invoking a capability. The verified list is empty until Adapt provides a documented, verified non-mutating capability. `ask_adapt` is rejected even if it claims to be read-only.

## Consequences

ATUI-2 cannot execute a live query yet, but the client cannot accidentally turn an unsafe capability into a read-only one. When Adapt provides a suitable capability, adding its verified name and contract tests will enable the existing query seam without weakening the boundary.
