# Phone bridge

`runo --phone` is the API Runo's mobile app talks to. It runs from the Runo binary itself,
without a window, and lets a phone follow every session and answer it.

```
phone ──HTTP──▶ runo --phone (on your computer) ──▶ ~/.runo state, approvals
                         │
                         └──▶ ~/.runo/phone-inbox ──▶ the Runo app runs it
```

## Running it

```sh
runo --phone                     # listens on 0.0.0.0:7631
runo --phone 100.x.y.z:7631      # or only on one address, such as your Tailscale one
```

From source, use `cargo run -p runo -- --phone`. It prints a QR code and a pairing link,
`runo://pair?url=…&token=…`, that the phone app reads.

## How it works

- **Reads** come from the saved state (the same reader Runo's MCP server uses) and the live
  approvals folder, so the phone sees what the desktop app sees.
- **Permission requests** are answered by the bridge directly, the same way the corner card
  answers them, so an agent waiting on one carries on as soon as you tap Allow.
- **Follow-ups, Stop, plan approvals and answers** are left in `~/.runo/phone-inbox`. The
  open Runo app picks them up within a couple of seconds and runs them as if they were
  typed, so queues, limits and per-agent settings still apply. Keep Runo open for these.
- Turns on SSH and Cove connections keep running with your laptop asleep; the phone sees
  them whenever the computer running the bridge is awake.

## Security

Every request needs `Authorization: Bearer <token>`. The 256-bit token is made on first run
and kept in `~/.runo/phone.json` (readable only by you). The bridge speaks plain HTTP: use it
over your home Wi-Fi or a private network such as [Tailscale](https://tailscale.com), never
open to the internet. To revoke a phone, delete `~/.runo/phone.json` and restart the bridge.

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

`status` is `idle`, `working`, `done` or `failed`. `needs` is `null` or one of `approval`
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
