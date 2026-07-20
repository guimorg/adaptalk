# ADR-0006: Expand local file references in chat prompts

## Status

Accepted, with staged implementation

## Context

Local context is adaptalk's wedge over Slack: a user can bring a nearby file
into a Chat Prompt without first uploading it to another system or manually
copying its contents. This is useful for asking Adapt about source code,
notes, and other working files while keeping the interaction in the
Terminal REPL.

The feature must remain a small, predictable prompt convenience. It must not
turn adaptalk into a general file browser, silently expose sensitive local
files, or change the read-only boundary of the Adapt MCP Server.

## Decision

### ATUI-8 path forms

ATUI-8 accepts only paths relative to the current working directory (CWD).
Absolute paths, paths beginning with `~`, and any path containing a `..`
component are rejected. Before every safety check, adaptalk canonicalizes the
candidate and requires the canonical path to remain under the canonical CWD.
This prevents symlink escapes.

### Syntax and parsing

ATUI-8 supports only whitespace-delimited bare references such as `@README.md`
and `@./notes/file.md`. Quoted references, escaped at-signs, and path
completion are deferred.

An `@` in the middle of a word and a trailing `@` are literal text. The
parser must not interpret an arbitrary email address, mention-like token, or
unfinished trailing marker as a file reference. Quoted references may contain
spaces; an unterminated quoted reference is invalid rather than partially
expanded.

References are resolved only when the user submits a complete Chat Prompt.
Typing `@` does not inspect the filesystem or show path suggestions. A lone
or trailing `@` therefore remains literal text, and ATUI-8 does not include an
interactive completion UI.

### Expansion

Each reference is replaced with a block containing the user-typed reference
and the file contents:

```text
<file path="@README.md">
file contents
</file>
```

The path attribute and file contents are escaped for the block's syntax, so a
referenced file cannot forge the closing `</file>` marker or add prompt-shaped
markup. Escaped file contents are not interpreted as additional references.
Canonical paths remain mandatory for all safety checks. A later broadening of
the syntax may emit canonical paths, but ATUI-8 preserves the typed reference
outwardly. Expansion is silent. Errors are reported as prompt errors without
exposing credential material.

### Safety and resource policy

The implementation applies checks in this order:

1. resolve the CWD-relative path;
2. canonicalize the path before every safety check;
3. compare the canonical path against the built-in deny-list;
4. open the canonical file once, inspect metadata from that handle, and reject
   files over the configured maximum size (and non-regular files); and
5. perform a bounded read from that same handle, then construct the escaped
   expansion block.

The deny-list is built in, fixed by the application, and compared against
canonical paths. ATUI-8 denies `~/.adapt/config.toml` and every path under
`~/.adapt/sessions/`. Users cannot edit or disable it through this feature.
Credential values must never appear in terminal output, transcripts, or logs.

If any reference in a prompt fails parsing, security checks, size checks, or
reading, the entire prompt is rejected and sent to the Adapt MCP Server
neither in original nor partially expanded form. No partial result is
displayed or persisted.

### History and input preservation

The original user input is preserved in Local Adapt History as the Chat
Prompt the user entered. The expanded prompt sent to Adapt is kept as
request metadata only when needed to reproduce the request, and must follow
the existing history redaction boundary. File contents are not written to a
separate cache, log, or transcript entry merely because they were expanded.
Normal session history therefore remains navigable without turning it into a
copy of the local filesystem. Replaying a historical prompt does not reread
local files implicitly; a user must submit a fresh prompt to expand current
file contents.

## Deferred or rejected alternatives

The following are explicitly outside this decision:

- skill references or a skill-discovery mechanism;
- adapting adaptalk into an MCP server;
- connecting to local MCP servers;
- globbing or wildcard expansion;
- interactive path suggestions or completion while typing;
- user-editable deny-lists; and
- regex-based redaction of file contents.

These features are out of scope for this decision and are not part of ATUI-8.
If interactive path suggestions are added later, they must apply the same
canonicalization and deny-list policy and must avoid exposing protected
directory contents. In particular, regex redaction is rejected as a security
boundary because it cannot reliably identify secrets; deny-listing and the
existing credential redaction boundary remain the controls for this first
version.

## Consequences

Users get a concise way to provide local context while the Adapt integration
and read-only policy remain unchanged. Canonicalization-before-checking makes
security decisions consistent across aliases and symlinks, while the
all-or-nothing rule avoids sending incomplete prompts.

ATUI-8 permits regular UTF-8 files of at most 1 MiB, including exactly 1 MiB.
The parser, path policy, expansion formatter, and history behavior need
focused tests before later path forms are introduced.
