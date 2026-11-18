# torch-placer-rs

A reach-only automatic torch placer for [Horizon](https://github.com/horizon-rust/horizon),
as a native mixin mod (`torchplacer.hnmod`).

It scans the cells around you every tick at 20 TPS (configurable) and, when it finds
an empty cell in reach whose light level is at or below the threshold, selects
a torch from your hotbar and places it on the solid block below through the
normal use action. Vanilla reach, item consumption and placement rules apply.
It never walks anywhere: it lights up wherever you happen to be.

## Commands

| Command | What it does |
| --- | --- |
| `/torch` | Toggle auto-torch on/off |
| `/torch on` / `/torch off` | Set the state explicitly |
| `/torch status` | Show state, threshold, radius, interval |
| `/torch level <0-15>` | Light level at or below which a torch is placed (default 4) |
| `/torch radius <2-16>` | Scan radius in blocks (default 7; actual placement reach is ~4.5) |
| `/torch now` | Force a scan on the next tick |
| `/torch clear` | Forget recent placement cells |

Settings are persisted through Horizon's per-mod store and can also be
changed from the mod's settings page (toggle / light level / radius).

## Behavior notes

- Only **block light** counts (0–15); skylight is intentionally ignored.
  Default threshold: 4, so any cell with block light 4 or less is dark enough.
- A cell is only used if it is air or replaceable, is not inside your body,
  has a solid block under it *or beside it* — full blocks, snow, slabs and
  stairs all count (floor or wall attachment) — has
  no torch in its 3×3×3 neighbourhood, and
  was not used in the last 200 ticks (60 ticks if the placement failed).
- If there is no torch in hotbar slots 1–9 it says so once and waits; it
  detects a torch again as soon as you add one.
- Right-click and the forced hit target are only held for the few ticks of a
  placement and are always released afterwards, so no button gets stuck.

## Build / test

```sh
horizon-launcher native-build mixins/torchplacer --trust-native-code --profile dev
```

Pinned to Horizon `9f42064` (game 26.3), the same revision as the other local
mods.
