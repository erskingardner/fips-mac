# UI information architecture

FIPS is an operational utility. Its interface should answer three questions in
order: **Is the node healthy?**, **What is it doing?**, and **What can I change?**

## Persistent application frame

- **Sidebar** — four product destinations: Overview, Peers, Transports, and
  Settings. The node's current health and version remain visible at the bottom.
- **Page header** — the current destination, last refresh time, and at most one
  primary page action.
- **Environment banner** — Product Preview and exceptional system states sit
  above page content. They are never presented as ordinary content cards.

## Destinations

### Overview

1. **Node identity and lifecycle** — address, npub, health, ownership, and
   start/stop/restart controls form one operational summary.
2. **Operating facts** — uptime, mesh estimate, role, identity persistence,
   TUN state, and transport count use a single divided facts row.
3. **Network activity** — peer/session/mesh metrics and traffic quality share
   one section.
4. **Current routes** — a short peer list links to the complete Peers view.

### Peers

- One table is the primary object on the page.
- Connect is the page-level action.
- Selecting a peer opens a detail drawer; it does not add another page card.

### Transports

- Transports are comparable operational records, so they appear as compact
  rows with aligned state, address, MTU, and optional endpoint fields.
- Empty state guidance occupies the same section rather than a separate card.

### Settings

Settings has three explicit destinations:

- **General** — Mac app visibility and FIPS installation ownership.
- **Node** — guided configuration, Advanced YAML, review, and apply.
- **Developer** — `tauri dev`-only Product Preview and source-built node socket
  overrides. It is omitted from all packaged builds, including the fast local
  build.

Within Node settings, the left navigation selects one configuration category.
The active source and Guided/YAML mode are persistent workspace context rather
than another container.

## Surface rules

- Use typography, whitespace, alignment, and one-pixel dividers for ordinary
  hierarchy.
- Use a filled or outlined surface only for alerts, modals, drawers, inputs,
  destructive confirmations, and exceptional empty states.
- Avoid nested rounded rectangles. A rounded control may live in a flat
  section, but a rounded section should not contain another rounded section.
- Keep labels compact and values prominent; operational density is preferable
  to decorative spacing.
