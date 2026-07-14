# ADR-0004: Fail closed until Adapt verifies a read-only capability

## Status

Accepted

## Context

Adapt currently exposes `ask_adapt`, but it is not guaranteed to be non-mutating. MCP `readOnlyHint` metadata is supplied by the remote server and is only a hint, so it cannot by itself establish the first-release safety boundary.

## Decision

AdaptTUI requires both an explicit `readOnlyHint: true` annotation and an Adapt-specific verified capability name before exposing or invoking a capability. The verified list is empty until Adapt provides a documented, verified non-mutating capability. `ask_adapt` is rejected by the normal query path even if it claims to be read-only.

As a temporary development exception, the process-only `--allow-unverified-ask-adapt` flag permits the narrowly scoped `ask_adapt` query method. The CLI must print a warning that the capability is not verified as read-only and may perform mutations. The flag does not authorize arbitrary unverified capabilities and is never persisted.

Capability policy is discovered lazily and cached as the raw MCP tool list for the lifetime of one `AdaptClient` session. Discovery and query validation therefore share one authoritative snapshot, while every invocation still validates the cached tool's name and annotation at the client boundary. A failed initial discovery is not cached, so later access may retry. There is no automatic refresh in this slice; a long-lived client can therefore observe stale capability metadata until a future explicit refresh operation or a new client is created.

## Consequences

The default remains fail-closed, while development users have an explicit, visible escape hatch for `ask_adapt`. The snapshot removes repeated discovery round trips and keeps policy decisions inside the adapter, trading freshness during a client session for predictable validation. When Adapt provides a suitable verified capability, adding its name and contract tests will enable the normal query seam without weakening the boundary; the temporary exception can then be removed.
