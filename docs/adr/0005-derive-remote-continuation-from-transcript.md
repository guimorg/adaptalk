# ADR-0005: Derive remote continuation from the transcript

## Status

Accepted

## Context

An Adapt remote chat identifier is returned with a response and needed only for the next development-mode `ask_adapt` request. Storing it separately on the local session and active conversation duplicated state and required manual synchronization across transitions.

## Decision

Persist the identifier only in the response transcript entry. The conversation controller derives the continuation value from the most recent response whenever it submits a prompt. Local session creation records only lineage through `resumed_from_session_id`.

The controller receives an already-connected query adapter and returns a viewed session or submission outcome. Connection/configuration remains terminal wiring, so a failed connection cannot disturb a history-viewing state.

## Consequences

The transcript is the authoritative record for resuming a conversation, and continuation behavior is deterministic to test with a narrow query trait. Older local history files with a session-level identifier simply resume without one unless their transcript contains a response identifier.
