# Recorded agent runs

Each `<agent>-<version>-<what>.jsonl` is the raw output of one real turn of an agent CLI. The `.events.json` beside it holds the events Runo must turn that output into. `harness::fixtures` replays every file through the parser for that agent and fails when the events change. It also checks that every running step gets exactly one result.

To record a new run, start Runo with `RUNO_RECORD=<folder>` set. Every structured agent's output is copied to `<folder>/<agent>-<session>-<turn>.jsonl` as it arrives. Before adding a file here:

- Replace home folders with `/Users/dev` and the project folder with `/work/app`.
- Cut Claude's `system` `init` line down to `type`, `subtype`, `session_id`, `model` and `cwd`. It lists your MCP servers, plugins and account details.
- Drop `rate_limit_event` lines, and anything else that names an account or holds a token.

Then run `RUNO_UPDATE_FIXTURES=1 cargo test fixtures` to write the expected events. Read the diff before committing it.

These runs were recorded on macOS. The Claude run has the noise above taken out, and its other lines are as the CLI printed them.
