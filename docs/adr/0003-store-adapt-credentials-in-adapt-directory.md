# Store Adapt credentials in the local Adapt directory

AdaptUI will support a persisted bearer token in the user-local `.adapt` directory for portability and consistency with existing local TOML configuration workflows. The credential must be stored separately from Local Adapt History, the file must use restrictive permissions where supported, and tokens must never be written to transcripts, logs, or terminal output.
