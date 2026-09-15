---
title: Privacy policy
description: What mecha does with Google account data, where it is stored, and who else can see it.
hide_table_of_contents: false
---

# Privacy policy

_Last updated: 15 September 2026_

**mecha** is an open-source agent harness that runs on a person's own computer.
It is maintained by Luke Chang and distributed under the MIT license at
[github.com/ljchang/mecha](https://github.com/ljchang/mecha).

This policy describes what happens to Google account data when someone connects
a Google account to their own installation of mecha. It is short because the
architecture is simple: **there is no mecha server.** Every copy of mecha runs on
its user's machine, holds its own credentials, and talks to Google directly. The
maintainer operates no service that receives, stores, or processes another
person's Google data, and has no ability to read it.

## What mecha asks for, and why

When you connect a Google account, mecha requests these OAuth scopes:

| Scope | What it allows |
| --- | --- |
| `gmail.modify` | Read your mail, and change labels and read state during triage. It stops deliberately short of `https://mail.google.com/`, so it does **not** allow permanent deletion. |
| `gmail.send` | Send mail. |
| `calendar` | Full access to your calendars — read, create, update and **delete** events, and read free/busy time. Deletion is a shipped capability, not a theoretical one. |
| `calendar.events` | Create and update individual events — for example, turning a confirmed booking into a meeting on your calendar. |

Google Docs support is a **separate, optional grant** with its own OAuth client
and its own token file. It asks for one scope, `drive.file`, which gives access
only to files you explicitly pick and files the app itself creates — never your
whole Drive.

mecha asks for any of this only if you choose to connect an account. Connecting
one is optional; mecha runs with no mail, calendar or document access at all.

## Where the data goes

**It stays on your computer.** Specifically:

- **OAuth tokens** are written on your own machine — mail and calendar at
  `~/.mecha/mail/<account>/oauth.json`, documents at
  `~/.mecha/docs/<account>/oauth.json` — with owner-only permissions (`0600` on
  the file, `0700` on the directory). They are never transmitted anywhere except
  to Google, to refresh themselves.
- **Message and calendar content** is read on demand over the network from Google
  to your machine. Anything cached is written under `~/.mecha/` on the same machine.
- **Nothing is sent to the maintainer.** There is no telemetry, no analytics, no
  crash reporting, and no hosted backend.

## Who else can see it

Nobody, with one disclosure you should read carefully.

mecha is an agent harness, which means it passes text to a language model in order
to answer questions about it. **Which model, and therefore where that text goes,
is set by you** in `~/.mecha/config.toml`:

- **A local model.** mecha exists to make a local, open-weight model usable as a
  personal assistant, and many installations point it at a model served on the
  same machine. Configured that way, your mail and calendar content does not
  leave your computer at all.
- **A hosted provider.** mecha also supports hosted APIs, and its built-in
  default configuration names one (Anthropic). Configured that way, the content
  you ask it to work with is sent to that provider, and is handled under that
  provider's own terms and privacy policy rather than this one.

Neither choice sends anything to the maintainer of mecha. If you want to know
which applies to your installation, `default_provider` in your configuration file
is the answer.

Two further destinations exist if you enable them, and both are your
configuration to make: **web search backends**, which receive the search queries
mecha issues (a query is itself a channel, which is why mecha treats search as
outbound), and **MCP servers** you connect, which receive whatever the tool call
you asked for sends them. mecha ships with neither pointed anywhere by default.

mecha never sells Google user data, never uses it for advertising, and never uses
it to train a model. Beyond the destinations you configure yourself — the model
provider, any search backend, any MCP server, and the Slack transport if you
connect one — it is not shared with anyone. Every one of those is off until you
turn it on, and each is named in your own configuration file.

## Limited Use

mecha's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Sending is not automatic

mecha can route outbound tools through a local review queue: a call becomes a
draft, and nothing leaves until a person reads it and releases it. **This is off
until you turn it on** — the setting is `[outbox] tools` and it is empty by
default, because routing a tool is a policy decision rather than something to
assume. Configured, it is the strongest guarantee here. Unconfigured, a send is
a send.

Two things send without passing through that queue even when it is configured,
and both are worth knowing:

- **Creating a calendar event with attendees notifies them immediately.** mecha
  sets `sendUpdates=all`, so the provider mails an invitation from your account
  the moment the event exists. This is true of any event with attendees, not
  only booked meetings.
- **Deleting a calendar event mails a cancellation to every attendee**, and
  unlike creation this is unconditional. Of the three calendar verbs, updating
  an event is the only quiet one.
- **Bookings taken through your own published booking page** become calendar
  events by a scheduled, deterministic path with no model and no review step.
  That is deliberate: you approved those slots when you published them, and a
  visitor who books one should not wait on you to find out whether it took.

## Retention and deletion

All of it is on your machine, so you control it directly:

- **Disconnect one mail/calendar account:** delete `~/.mecha/mail/<account>/`.
- **Disconnect a documents account:** delete `~/.mecha/docs/<account>/`. This is
  a separate grant, so removing the mail one does not revoke it.
- **Remove everything:** delete `~/.mecha/`.
- **Revoke access from Google's side:** visit
  [myaccount.google.com/permissions](https://myaccount.google.com/permissions)
  and remove the app. This invalidates the stored tokens immediately.

Uninstalling mecha removes the software; deleting `~/.mecha/` removes the data.

## About this website

Everything above describes the software. This documentation site is a separate
thing and deserves its own sentence, because "no telemetry, no analytics" is a
claim about mecha and a reader could reasonably hear it as a claim about the page
they are reading.

The site runs no analytics and sets no cookies. It does load its typefaces from
Google Fonts, which means opening any page here — including this one — sends your
IP address and browser user-agent to Google, as loading any third-party asset
does. There is nothing else.

## Children

mecha is developer tooling and is not directed at children under 13.

## Changes

Material changes to this policy will be published on this page with an updated
date above.

## Contact

Questions about this policy: [lukejchang@gmail.com](mailto:lukejchang@gmail.com),
or open an issue at
[github.com/ljchang/mecha/issues](https://github.com/ljchang/mecha/issues).
