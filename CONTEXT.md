# AdaptTUI

AdaptTUI is a local interactive client for using Adapt through its hosted MCP service.

## Language

**Adapt**:
The hosted AI platform that answers questions over connected business tools and can take actions on a user's behalf.
_Avoid_: AdaptUI, Slack bot

**Adapt MCP Server**:
The hosted Model Context Protocol service through which external clients discover and invoke Adapt capabilities.
_Avoid_: Adapt API, Slack integration

**AdaptTUI**:
The local terminal application that connects a user to Adapt through the Adapt MCP Server.
_Avoid_: Adapt server, Adapt clone, AdaptUI

**Adapt Session**:
A user-visible conversational interaction with Adapt, including its requests, responses, tool activity, and action approvals.
_Avoid_: Slack thread, MCP connection

**Chat Prompt**:
One natural-language request submitted by the user within an Adapt Session.
_Avoid_: Tool command, MCP request

**Terminal REPL**:
The minimal AdaptTUI interaction surface: a line-oriented prompt and colored conversation output preserved in native terminal scrollback, with structured read-only results shown inline.
_Avoid_: Dashboard, tool browser

**Action Approval**:
The user's explicit confirmation that Adapt may perform a mutating operation requested during an Adapt Session.
_Avoid_: Tool permission, automatic execution

**Read-only Mode**:
An AdaptUI operating mode in which the client permits only verified non-mutating capabilities and does not execute actions that change external systems.
_Avoid_: Safe mode, dry run

**Local Adapt History**:
The locally persisted record of Adapt Sessions maintained by AdaptUI for later browsing and resumption.
_Avoid_: Chat cache, Slack archive

**Adapt Credential**:
A local authentication secret used by AdaptUI to connect to the Adapt MCP Server, such as a bearer token.
_Avoid_: Session history, API response

**Citation**:
The source reference attached to an Adapt response that lets the user inspect where the answer came from.
_Avoid_: Link, footnote
