---
title: Architecture
keywords: architecture, diagram, system diagram, c4, draw.io, drawio, mermaid, boundary, beautify, structure, decisions, adr, architecture notes, model, components, services, infrastructure, docker compose, terraform, aws, logos, icons, stories, story player, export svg, export png, code map
shortcuts: go-architecture
open: architecture
---
Architecture ({go-architecture}, under the project's badge on the rail) maps the current project. **Structure** reads the project's manifests and imports into its parts and what depends on what; **Notes and decisions** keeps architecture notes and decisions (ADRs) in the repository; and the diagram editor draws the system.

Diagrams are kept as readable TOML in the project's `architecture/` folder: `model.toml` holds the parts and how they connect, and `views/<id>.toml` is each diagram, so renaming a part once updates every diagram it is on. Draw by hand from the shape palette (people, systems, containers, databases, queues, boundaries, notes and more), drag from a shape's side dot to connect it, and pick a logo from the searchable library of brand and tech icons. **Start from code** draws a first diagram from the project's code, or from how it is deployed (Docker Compose, Terraform for AWS, ECS and Copilot).

**Ask AI to change the diagram…** has an agent edit the files; its change shows as a preview to **Accept** or **Reject**. **Beautify** tidies the layout so connectors don't cross and groups the parts by role; **Tidy layout** and the Arrange menu (align, spread, bring to front) do it by hand. Other asks: **Explain with AI**, **Expand into parts with AI**, **Find risky coupling** and **Check the code against the notes**. **Stories** play a request's path through the diagram step by step. Export as PNG, SVG, an animated SVG, a Mermaid file or draw.io, copy as Mermaid, and import draw.io files or a picture. **Discard all diagram changes** puts the diagrams back as they were at the last commit. On the canvas, Esc steps out, Tab moves to the next shape, ⌘C copies and ⌘F finds.
