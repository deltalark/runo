# Phone bridge

The phone bridge is the API Runo's mobile app talks to. It lets a phone follow every
session and answer it. Turn it on in Runo under **Settings > Phone**, or run it without a
window with `runo --phone`.

```
phone ──HTTP──▶ the bridge (on your computer) ──▶ ~/.runo state, approvals
                         │
                         └──▶ ~/.runo/phone-inbox ──▶ the Runo app runs it
```

## Turning it on

In **Settings > Phone**, switch on **Phone bridge**. It runs inside the app, on a background
thread, while Runo is open, and stays on across launches. The page shows the address it
listens on.

Then press **Pair a new phone**. Runo makes a new key for that phone and shows its pairing
link, `runo://pair?url=…&token=…`, as a QR code to scan with the Runo mobile app, with a
**Copy** button. Each paired phone is listed with the date it was paired; **Show link**
shows its link again and **Remove** unpairs it.

## Where it listens

The bridge speaks plain HTTP, so by default it never listens on a shared network:

- If this computer has a [Tailscale](https://tailscale.com) address (in `100.64.0.0/10`),
  the bridge listens only there. Only devices signed in to your tailnet can reach it, and
  Tailscale encrypts the connection.
- Otherwise it listens on `127.0.0.1`, which only this computer can reach. Start
  Tailscale, then turn the bridge off and on again to pick up its address.

To listen on every network instead, choose **Local network (unencrypted)** under
**Reachable from**, or give `runo --phone` an address. Anyone on the same Wi-Fi can then
read a phone's key as it is sent, so only do this on a network you trust.

## Running it without a window

```sh
runo --phone                     # your Tailscale address, else 127.0.0.1, port 7631
runo --phone 0.0.0.0             # every network, unencrypted
runo --phone 192.168.1.20:9000   # one address and port
```

From source, use `cargo run -p runo -- --phone`. It prints a QR code and the pairing link
of the most recently paired phone, pairing one first if there are none. Pair more phones,
each with its own link, in Settings > Phone. Settings and `runo --phone` share the same
list of phones, and both use port 7631, so run one or the other.

## How it works

- **Reads** come from the saved state (the same reader Runo's MCP server uses) and the live
  approvals folder, so the phone sees what the desktop app sees.
- **Permission requests** are answered by the bridge directly, the same way the corner card
  answers them, so an agent waiting on one carries on as soon as you tap Allow.
- **Follow-ups, Stop, plan approvals and answers** are left in `~/.runo/phone-inbox`,
  whether the bridge runs in the app or from `runo --phone`. The open Runo app picks them
  up within a couple of seconds and runs them as if they were typed, so queues, limits and
  per-agent settings still apply. Keep Runo open for these.
- Turns on SSH and Cove connections keep running with your laptop asleep; the phone sees
  them whenever the computer running the bridge is awake.

## Security

Every request needs `Authorization: Bearer <token>`. Each paired phone has its own 256-bit
token, kept with its name and pairing date in `~/.runo/phone.json` (readable only by you).
A request is accepted when its token matches any paired phone's; tokens are compared in
constant time. Removing a phone in Settings revokes only its token: its next request gets
`401`, and the other phones carry on, with no restart needed.

A `phone.json` from before per-phone tokens, with a single `token`, is read as one phone
named "Phone", so a phone paired then stays paired.

The bridge speaks plain HTTP. Keep it on Tailscale or this computer, which is the
default; listen on a local network only if you trust everyone on it, and never open it to
the internet.

## API

`GET`s return an `ETag`; send it back as `If-None-Match` to get `304` when nothing changed,
so polling every few seconds stays cheap.

| Method and path | What it does |
| --- | --- |
| `GET /v1/hello` | `{ app, version, api }`, to check a pairing |
| `GET /v1/sessions` | `{ projects, sessions }`: every session with its status, a preview and `needs` |
| `GET /v1/sessions/:id` | `{ session, messages, queue, approvals }`: the last 200 messages, queued prompts and pending permission requests |
| `POST /v1/sessions/:id/prompt` | `{ text }`: a follow-up, queued while the agent works |
| `POST /v1/sessions/:id/stop` | Stop the running turn |
| `POST /v1/sessions/:id/plan` | Approve the proposed plan and build it |
| `POST /v1/sessions/:id/allow` | `{ tools }`: allow denied tools once and continue |
| `POST /v1/sessions/:id/answers` | `{ picks }`: one answer (or `null`) per question |
| `POST /v1/approvals/:id` | `{ allow, message? }`: answer a live permission request |

Actions on a session return `202`: they wait for the app. Errors are
`{ "error": "…" }` with `400`, `401`, `404` or `500`.

A session in the list:

```json
{
  "id": 11, "project": 1, "title": "Fix tab overflow", "agent": "Claude Code",
  "branch": "fix/tab-overflow", "status": "working",
  "unread": false, "pinned": false, "archived": false, "elsewhere": false,
  "updated": "21:04", "preview": "Looking at split.rs…",
  "needs": { "kind": "approval", "text": "Wants to run `cargo test`" }
}
```

`status` is `idle`, `working`, `done`, `failed` or `stopped` (the person stopped the turn). `needs` is `null` or one of `approval`
(a live permission request), `denied` (tools the last turn wasn't allowed), `questions`
or `plan`.

A message in a session:

```json
{
  "role": "agent", "text": "…", "playbook": "", "steps": ["Explored 6 files"],
  "time": "21:04", "meta": "48s · 3 turns · $0.21",
  "plan": "", "questions": [{ "question": "…", "options": ["A", "B"] }], "denied": []
}
```

A pending permission request: `{ "id", "tool", "what": "run `cargo test`", "asked" }`.

The code is in `native/runo-app/src/phone.rs`; its tests run with `cargo test -p runo phone::`.
