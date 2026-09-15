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

| Scope | Why it is needed |
| --- | --- |
| `gmail.modify` | Read your mail so you can ask questions about it, and change message labels and read state during triage. It does **not** include permanent deletion. |
| `gmail.send` | Send mail you have reviewed and approved. |
| `calendar` | Read your calendar so scheduling questions can be answered, and see free/busy time. |
| `calendar.events` | Create and update events — for example, turning a confirmed booking into a meeting on your calendar. |

mecha asks for these only if you choose to connect a Google account. Connecting
one is optional; mecha runs without any mail or calendar access at all.

## Where the data goes

**It stays on your computer.** Specifically:

- **OAuth tokens** are written to `~/.mecha/mail/<account>/oauth.json` on your own
  machine, with owner-only file permissions. They are never transmitted anywhere
  except to Google, to refresh themselves.
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

mecha never sells Google user data, never uses it for advertising, and never uses
it to train a model. It is not shared with any third party other than the model
provider you yourself configure, as described above.

## Limited Use

mecha's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Sending is not automatic

Worth stating because it is unusual: mecha does not send mail on your behalf
without your review. Outbound messages are staged as drafts in a local review
queue, and a person has to read and release each one before it leaves. Calendar
invitations for meetings that a visitor booked through your own published booking
page are the deliberate exception, because you approved those slots when you
published them.

## Retention and deletion

All of it is on your machine, so you control it directly:

- **Disconnect one account:** delete `~/.mecha/mail/<account>/`.
- **Remove everything:** delete `~/.mecha/`.
- **Revoke access from Google's side:** visit
  [myaccount.google.com/permissions](https://myaccount.google.com/permissions)
  and remove the app. This invalidates the stored tokens immediately.

Uninstalling mecha removes the software; deleting `~/.mecha/` removes the data.

## Children

mecha is developer tooling and is not directed at children under 13.

## Changes

Material changes to this policy will be published on this page with an updated
date above.

## Contact

Questions about this policy: [lukejchang@gmail.com](mailto:lukejchang@gmail.com),
or open an issue at
[github.com/ljchang/mecha/issues](https://github.com/ljchang/mecha/issues).
