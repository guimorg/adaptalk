# ADR-0006: Expand local file references in chat prompts

## Status

Pending

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

### Accepted path forms

An explicit local reference may name:

- a path relative to the current working directory (CWD);
- an absolute path; or
- a path beginning with `~`, expanded using the user's home directory.

After `~` expansion and relative-path resolution, adaptalk canonicalizes the
path before applying any security check. The canonical path is the path used
for deny-list comparison, size checks, reading, and the emitted `file path`
attribute. Missing paths, paths that cannot be canonicalized, and paths that
resolve outside the permitted local-file policy are rejected.

### Syntax and parsing

The supported forms are a bare reference such as `@README.md` and a quoted
reference such as `@"path with spaces.md"`. A literal at-sign is written
`\@`.

An `@` in the middle of a word and a trailing `@` are literal text. The
parser must not interpret an arbitrary email address, mention-like token, or
unfinished trailing marker as a file reference. Quoted references may contain
spaces; an unterminated quoted reference is invalid rather than partially
expanded.

### Expansion

Each reference is replaced with a block containing the canonical file path
and the file contents:

```text
<file path="/canonical/path/to/README.md">
file contents
</file>
```

The path attribute is escaped for the block's syntax, and file contents are
inserted as text without interpreting their contents as additional
references. Expansion is silent: the user sees the resulting prompt and
response flow, not a separate progress message or file-content dump. Errors
are reported as prompt errors without exposing credential material.

### Safety and resource policy

The implementation applies checks in this order:

1. resolve `~` and CWD-relative paths;
2. canonicalize the path;
3. compare the canonical path against the built-in deny-list;
4. inspect metadata and reject files over the configured maximum size (and
   non-regular files); and
5. read the file and construct the expansion block.

The deny-list is built in, fixed by the application, and compared against
canonical paths. It covers credential/configuration locations and other
known secret-bearing local state, including Adapt credentials and
`.adapt/sessions/`. Users cannot edit or disable it through this feature.
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
- user-editable deny-lists; and
- regex-based redaction of file contents.

These may be reconsidered in separate ADRs. In particular, regex redaction
is rejected as a security boundary because it cannot reliably identify
secrets; deny-listing and the existing credential redaction boundary remain
the controls for this first version.

## Consequences

Users get a concise way to provide local context while the Adapt integration
and read-only policy remain unchanged. Canonicalization-before-checking makes
security decisions consistent across aliases and symlinks, while the
all-or-nothing rule avoids sending incomplete prompts.

The parser, path policy, expansion formatter, and history behavior will need
focused tests before implementation. A maximum file size and the exact
built-in deny-list entries must be chosen in the ATUI-8 implementation and
documented there or in a follow-up ADR if they become durable policy.
