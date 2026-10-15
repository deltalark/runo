<!-- Written by Runo. It describes the files next to it; edit them, not this. -->
# Architecture diagrams

This folder holds the project's architecture as plain TOML, drawn and edited in Runo's
Architecture page. People edit it on the canvas; agents can edit these files directly, and
the canvas shows the change as a preview to accept or reject.

- `model.toml` holds the parts of the system and how they connect. A part appears once
  here even when several diagrams show it.
- `views/<id>.toml` is one diagram: which parts it shows, where, and how they look. The
  file name is the diagram's id.

Every id is lowercase words joined by dashes and unique in the model. Keep ids stable: a
diagram refers to parts by id, so renaming a part means changing `name`, not `id`.
Coordinates are in pixels; `x` and `y` are a shape's top left corner and `y` grows down.
Colours are `#rrggbb`. Leave out any field you don't need; unset fields take the kind's
defaults.

## model.toml

```toml
[[element]]
id = "orders-api"          # required
name = "Orders API"        # required; may hold line breaks
kind = "container"         # see the kinds below; default "container"
tech = "Go"                # shown as [Container: Go]
description = "Takes and tracks orders"
owner = "Team Checkout"
tags = ["checkout", "tier-1"]
code = ["services/orders/**"]   # paths or globs of the code behind it
icon = "server"            # a Lucide icon name shown on the shape
image = "icons/docker.svg" # a picture in this folder: the whole shape for an `icon`, a corner badge otherwise
link = "orders-internals"  # the id of a diagram that zooms into this part

[[relation]]
id = "web-to-orders-api"   # required
from = "web"               # an element id; required
to = "orders-api"          # an element id; required
label = "Places orders"
tech = "HTTPS / JSON"
```

Kinds: `person`, `system`, `container`, `component`, `database`, `queue`, `external`
(an external system), `boundary` (a group such as a VPC, region or team, drawn behind
what it holds), `box`, `rounded`, `ellipse`, `diamond` (a decision), `cloud`, `note`
(a sticky note), `text` and `icon` (a logo or picture with its name under it).
Pictures of things: `actor` (a standing figure, its name under it), `users` (a group of
people), `browser`, `phone`, `desktop`, `server`, `document`, `folder` and `shield`
(security, a firewall). More outlines: `hexagon`, `triangle`, `parallelogram` (data in
or out), `terminator` (a flowchart's start or end), `callout` (a speech bubble) and
`arrow` (a block arrow).

Pictures live in `icons/` next to these files, as SVG, PNG, JPG, WebP or GIF. To use a
brand or technology logo, save it there first: Simple Icons has one-colour logos at
`https://raw.githubusercontent.com/simple-icons/simple-icons/develop/icons/<name>.svg`
(`docker`, `nodedotjs`, `postgresql`, `amazonwebservices`) and Devicon has colour ones at
`https://raw.githubusercontent.com/devicons/devicon/master/icons/<name>/<name>-original.svg`.

## views/*.toml

```toml
name = "Containers"        # the diagram's title
order = 0                  # its place among the diagram tabs
still = false              # true turns all motion off on this diagram

[[node]]                   # one per element shown
id = "orders-api"          # the element's id
x = 520.0
y = 230.0
w = 220.0
h = 110.0
parent = "shop"            # the boundary it sits in; it moves with it and stays inside it
locked = false             # true keeps it from being moved
layer = "Services"         # a layer below; none is the base layer
effect = "pulse"           # none, pulse (a breathing glow), blink (an alert) or spin
collapsed = false          # a boundary folded to one box; what it holds is hidden
open-size = [620.0, 420.0] # a collapsed boundary's size when open
style = { fill = "#dbeafe", stroke = "#2563eb", text = "#0f172a", font = 13.0, radius = 8.0, opacity = 1.0, dashed = false, bold = false }

[[edge]]                   # one per relation shown
id = "web-to-orders-api"   # the relation's id
route = "elbow"            # straight, elbow (goes round boxes) or curved
arrow = "end"              # end, both or none
motion = "flow"            # flow (marching dashes), packets, pulse or none; default flow
speed = "normal"           # slow, normal or fast
from-side = "bottom"       # top, right, bottom or left; leave out to pick the facing side
to-side = "top"
waypoints = [[600.0, 380.0]]   # bends placed by hand; leave out to route automatically
style = { stroke = "#dc2626", dashed = true }

[[layer]]                  # layers hide or lock shapes together
name = "Services"
hidden = false
locked = false

[[story]]                  # a request walked through the diagram, step by step
name = "Checkout"

[[story.step]]
edge = "web-to-orders-api" # a relation shown on this diagram
caption = "The cart is sent to the API"
```

## Making changes

- To add a part, add an `[[element]]` to `model.toml` and a `[[node]]` to each diagram
  that should show it. Place it near what it connects to, clear of other shapes (about
  60 px apart), and inside its boundary when it has a `parent`.
- To connect parts, add a `[[relation]]` and an `[[edge]]` on each diagram that shows both
  ends.
- To remove a part from one diagram, delete its `[[node]]` and the edges that touch it.
  To remove it everywhere, also delete its `[[element]]` and its relations.
- To split a diagram, copy its file under a new id and drop nodes from each copy; set
  `link` on a part to drill into the detailed one.
- Keep both files valid TOML. If an edit doesn't read, Runo keeps showing the last good
  diagram and says what's wrong until it's fixed.
