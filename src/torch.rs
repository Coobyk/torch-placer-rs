// Torch Placer — a reach-only helper. Each tick it finds the nearest dark,
// empty, in-reach cell a torch can attach to, selects a torch from the
// hotbar and places it through the normal use action, so vanilla placement
// rules, item consumption and reach checks still apply.

use std::collections::HashMap;
use std::sync::Mutex;

pub const ID: &str = "torchplacer";

/// 4.5 blocks — vanilla reach for placing a block.
const REACH2: f64 = 20.25;

/// The `radius` setting's bounds and default, shared by `sync` and
/// `/torch radius`.
const RADIUS_MIN: i32 = 2;
const RADIUS_MAX: i32 = 16;
const DEFAULT_RADIUS: i32 = 7;
/// The `threshold` setting's bounds and default, shared by `sync` and
/// `/torch level`.
const THRESHOLD_MIN: i32 = 0;
const THRESHOLD_MAX: i32 = 15;
const DEFAULT_THRESHOLD: i32 = 4;

/// Ticks right-click stays claimed for one placement: the use action's
/// `use_delay` is 4 ticks, so this gives it a chance to land.
const HOLD_TICKS: u8 = 6;
/// Ticks a cell is skipped after a placement landed, or after it failed.
const COOLDOWN_PLACED: u32 = 200;
const COOLDOWN_FAILED: u32 = 60;

/// Items `torch_slot` counts as a torch.
const TORCH_ITEMS: [&str; 4] = [
    "minecraft:torch",
    "minecraft:soul_torch",
    "minecraft:redstone_torch",
    "minecraft:copper_torch",
];
/// The blocks a torch is or becomes once placed (standing and wall forms):
/// `is_torch_at` and `supported`'s exclusion — the old `contains("torch")`
/// also matched torchflowers.
const TORCH_BLOCKS: [&str; 8] = [
    "minecraft:torch",
    "minecraft:wall_torch",
    "minecraft:soul_torch",
    "minecraft:soul_wall_torch",
    "minecraft:redstone_torch",
    "minecraft:redstone_wall_torch",
    "minecraft:copper_torch",
    "minecraft:copper_wall_torch",
];

struct S {
    enabled: bool,
    radius: i32,
    threshold: i32,
    /// Ticks left holding right-click for the placement in flight.
    held: u8,
    hit: Option<crate::game::Hit>,
    target: [i32; 3],
    warned: bool,
}

/// `ST`/`COOL` are lazily-filled `Mutex<Option<T>>`s: `locked` locks one,
/// fills it with `new()` when empty and runs `f` on the value inside.
fn locked<T, R>(
    m: &Mutex<Option<T>>,
    new: impl FnOnce() -> T,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(new());
    }
    f(g.as_mut().unwrap())
}

static ST: Mutex<Option<S>> = Mutex::new(None);
fn with<R>(f: impl FnOnce(&mut S) -> R) -> R {
    locked(&ST, fresh, f)
}
fn fresh() -> S {
    S {
        enabled: true,
        radius: DEFAULT_RADIUS,
        threshold: DEFAULT_THRESHOLD,
        held: 0,
        hit: None,
        target: [0; 3],
        warned: false,
    }
}

/// Cells we placed a torch in (or failed to) recently, with ticks remaining.
static COOL: Mutex<Option<HashMap<[i32; 3], u32>>> = Mutex::new(None);
fn cool<R>(f: impl FnOnce(&mut HashMap<[i32; 3], u32>) -> R) -> R {
    locked(&COOL, HashMap::new, f)
}

/// Input flags we set ourselves. The engine rewrites them on real input
/// events, so we must release only the ones we actually claimed.
static OWNED: Mutex<(bool, bool)> = Mutex::new((false, false)); // (right, hit)

fn claim(game: &mut crate::game::Game<'_>, hit: Option<crate::game::Hit>) {
    let mut o = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    // Re-select every tick: another mod can change the held slot between
    // ours, and the use action must act on a torch.
    if let Some(slot) = torch_slot(game) {
        game.selected = slot;
    }
    game.right = true;
    o.0 = true;
    game.forced_hit = hit;
    o.1 = hit.is_some();
    game.swing_arm();
}

fn release(game: &mut crate::game::Game<'_>) {
    let mut o = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    if o.0 {
        game.right = false;
        o.0 = false;
    }
    if o.1 {
        game.forced_hit = None;
        o.1 = false;
    }
}

/// A translated chat line with the mod's `torch:` prefix.
fn say(game: &mut crate::game::Game<'_>, text: horizon_client::text::Text) {
    let mut line = horizon_client::text::Text::literal("torch: ");
    line.push(text);
    game.chat_line(line);
}

/// `Text::tr` shortened: a translatable message whose args are plain values.
fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> horizon_client::text::Text {
    horizon_client::text::Text::tr(key, args)
}

/// Read persisted settings. Missing keys leave the defaults in place.
fn sync(game: &crate::game::Game<'_>) {
    if let Some(v) = game.setting(ID, "enabled") {
        with(|s| s.enabled = v == "true");
    }
    if let Some(v) = game.setting(ID, "radius") {
        if let Ok(n) = v.parse::<i32>() {
            with(|s| s.radius = n.clamp(RADIUS_MIN, RADIUS_MAX));
        }
    }
    if let Some(v) = game.setting(ID, "threshold") {
        if let Ok(n) = v.parse::<i32>() {
            with(|s| s.threshold = n.clamp(THRESHOLD_MIN, THRESHOLD_MAX));
        }
    }
}

fn set_enabled(game: &mut crate::game::Game<'_>, v: bool) {
    with(|s| s.enabled = v);
    game.set_setting(ID, "enabled", if v { "true" } else { "false" });
    if !v {
        with(|s| s.held = 0);
        release(game);
    }
}

fn feet(p: &crate::motion::PlayerMotion) -> [i32; 3] {
    [
        p.position.x.floor() as i32,
        p.position.y.floor() as i32,
        p.position.z.floor() as i32,
    ]
}

fn center(p: [i32; 3]) -> [f64; 3] {
    [
        p[0] as f64 + 0.5,
        p[1] as f64 + 0.5,
        p[2] as f64 + 0.5,
    ]
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// First hotbar slot holding any kind of torch.
fn torch_slot(game: &crate::game::Game<'_>) -> Option<usize> {
    game.inventory
        .iter()
        .take(9)
        .enumerate()
        .find_map(|(i, it)| {
            it.as_ref()
                .filter(|s| TORCH_ITEMS.contains(&s.item.as_ref()))
                .map(|_| i)
        })
}

fn is_torch_at(game: &crate::game::Game<'_>, p: [i32; 3]) -> bool {
    TORCH_BLOCKS.contains(
        &game
            .res
            .facts()
            .get(game.world.block(p[0], p[1], p[2]))
            .name
            .as_ref(),
    )
}

/// Skip cells next to an existing torch — avoids stacking torches in the
/// same spot before the light engine has updated.
fn torch_near(game: &crate::game::Game<'_>, p: [i32; 3]) -> bool {
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                if is_torch_at(game, [p[0] + dx, p[1] + dy, p[2] + dz]) {
                    return true;
                }
            }
        }
    }
    false
}

/// Don't place a torch where it would intersect the player's body.
fn intersects_player(game: &crate::game::Game<'_>, p: [i32; 3]) -> bool {
    let Some(pl) = game.player.as_ref() else {
        return false;
    };
    let (px, py, pz) = (pl.position.x, pl.position.y, pl.position.z);
    // The body's box: half its width each side of the position, its height up.
    let half = f64::from(pl.dimensions.width) / 2.0;
    let top = f64::from(pl.dimensions.height);
    let (x0, y0, z0) = (p[0] as f64, p[1] as f64, p[2] as f64);
    px + half > x0
        && px - half < x0 + 1.0
        && py + top > y0
        && py < y0 + 1.0
        && pz + half > z0
        && pz - half < z0 + 1.0
}

/// Nearest dark, empty, in-reach cell that can hold a torch. Returns the
/// target cell and the solid block under it (the face we click on).
fn scan(
    game: &crate::game::Game<'_>,
    me: [i32; 3],
    eye: [f64; 3],
) -> Option<([i32; 3], crate::game::Hit)> {
    let (radius, threshold) = with(|s| (s.radius, s.threshold));
    let facts = game.res.facts();
    // A block can hold a torch if it is solid enough: full blocks, snow,
    // slabs and stairs. Ignore torches, fluids and unbreakable blocks.
    let supported = |f: &horizon_items::blocks::StateFacts| {
        !TORCH_BLOCKS.contains(&f.name.as_ref())
            && f.fluid == horizon_items::blocks::Fluid::None
            && f.hardness >= 0.0
            && (f.full_collision
                || f.name.contains("snow")
                || f.name.contains("slab")
                || f.name.contains("stairs"))
    };
    let mut best: Option<(f64, [i32; 3], crate::game::Hit)> = None;
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            for dy in -3..=4 {
                let cell = [me[0] + dx, me[1] + dy, me[2] + dz];
                let d2 = dist2(eye, center(cell));
                if d2 > REACH2 {
                    continue;
                }
                // Block light only — skylight is deliberately ignored, so a
                // cell is "dark" whenever it has no artificial light source.
                let block = game.world.light(cell[0], cell[1], cell[2]) & 15;
                if block as i32 > threshold {
                    continue;
                }
                let st = game.world.block(cell[0], cell[1], cell[2]);
                if !(facts.is_air(st) || facts.get(st).replaceable) {
                    continue;
                }
                if intersects_player(game, cell)
                    || torch_near(game, cell)
                    || cool(|c| c.contains_key(&cell))
                {
                    continue;
                }
                let below = [cell[0], cell[1] - 1, cell[2]];
                let bf = facts.get(game.world.block(below[0], below[1], below[2]));
                let hit = if supported(bf) {
                    Some(crate::game::Hit {
                        pos: below,
                        face: [0, 1, 0],
                        point: [cell[0] as f64 + 0.5, below[1] as f64 + 1.0, cell[2] as f64 + 0.5],
                        inside: false,
                    })
                } else {
                    // Wall attachment: a solid side neighbour with the face
                    // pointing into our empty cell.
                    [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]]
                        .iter()
                        .find_map(|d| {
                            let sup = [cell[0] - d[0], cell[1], cell[2] - d[2]];
                            let f = facts.get(game.world.block(sup[0], sup[1], sup[2]));
                            if supported(f) {
                                Some(crate::game::Hit {
                                    pos: sup,
                                    face: *d,
                                    point: [
                                        sup[0] as f64 + 0.5 + d[0] as f64 * 0.5,
                                        sup[1] as f64 + 0.5,
                                        sup[2] as f64 + 0.5 + d[2] as f64 * 0.5,
                                    ],
                                    inside: false,
                                })
                            } else {
                                None
                            }
                        })
                };
                if let Some(hit) = hit
                    && best.as_ref().map_or(true, |(b, _, _)| d2 < *b)
                {
                    best = Some((d2, cell, hit));
                }
            }
        }
    }
    best.map(|(_, cell, hit)| (cell, hit))
}

pub(super) fn tick(game: &mut crate::game::Game<'_>) {
    if !game.native_window() || !game.playing() {
        release(game);
        return;
    }
    sync(game);
    cool(|c| {
        for v in c.values_mut() {
            *v = v.saturating_sub(1);
        }
        c.retain(|_, v| *v > 0);
    });

    let (me, eye) = {
        let Some(p) = game.player.as_ref() else {
            release(game);
            return;
        };
        (feet(p), [p.position.x, p.position.y + 1.62, p.position.z])
    };
    if !with(|s| s.enabled) {
        release(game);
        return;
    }

    // Finish an in-flight placement.
    if with(|s| s.held) > 0 {
        let hit = with(|s| s.hit);
        claim(game, hit);
        let left = with(|s| {
            s.held = s.held.saturating_sub(1);
            s.held
        });
        if left == 0 {
            let target = with(|s| s.target);
            let placed = is_torch_at(game, target);
            cool(|c| {
                c.insert(target, if placed { COOLDOWN_PLACED } else { COOLDOWN_FAILED });
            });
            with(|s| s.hit = None);
            release(game);
        }
        return;
    }

    release(game);

    let Some(slot) = torch_slot(game) else {
        if !with(|s| s.warned) {
            with(|s| s.warned = true);
            say(game, tr("torchplacer.no_torch", &[]));
        }
        return;
    };
    if let Some((cell, hit)) = scan(game, me, eye) {
        game.selected = slot;
        with(|s| {
            s.hit = Some(hit);
            s.held = HOLD_TICKS;
            s.target = cell;
            s.warned = false;
        });
        claim(game, Some(hit));
    }
}

pub(super) fn command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if !game.native_window() || !game.playing() {
        return;
    }
    let mut words = command.split_whitespace();
    let head = words.next().unwrap_or_default();
    if !head.eq_ignore_ascii_case("torch") {
        return;
    }
    *ran = true;
    let args: Vec<&str> = words.collect();
    match args
        .first()
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        None => {
            let v = !with(|s| s.enabled);
            set_enabled(game, v);
            say(
                game,
                tr(if v {
                    "torchplacer.auto.on"
                } else {
                    "torchplacer.auto.off"
                }, &[]),
            );
        }
        Some("on") => {
            set_enabled(game, true);
            say(game, tr("torchplacer.auto.on", &[]));
        }
        Some("off") => {
            set_enabled(game, false);
            say(game, tr("torchplacer.auto.off", &[]));
        }
        Some("status") => {
            let (v, r, t) = with(|s| (s.enabled, s.radius, s.threshold));
            say(
                game,
                tr(
                    if v {
                        "torchplacer.status.on"
                    } else {
                        "torchplacer.status.off"
                    },
                    &[&t, &r],
                ),
            );
        }
        Some("level" | "threshold") => {
            if let Some(n) = args.get(1).and_then(|v| v.parse::<i32>().ok()) {
                let n = n.clamp(THRESHOLD_MIN, THRESHOLD_MAX);
                with(|s| s.threshold = n);
                game.set_setting(ID, "threshold", &n.to_string());
                say(game, tr("torchplacer.threshold", &[&n]));
            } else {
                say(game, tr("torchplacer.threshold.usage", &[]));
            }
        }
        Some("radius") => {
            if let Some(n) = args.get(1).and_then(|v| v.parse::<i32>().ok()) {
                let n = n.clamp(RADIUS_MIN, RADIUS_MAX);
                with(|s| s.radius = n);
                game.set_setting(ID, "radius", &n.to_string());
                say(game, tr("torchplacer.radius", &[&n]));
            } else {
                say(game, tr("torchplacer.radius.usage", &[]));
            }
        }
        Some("clear") => {
            cool(|c| c.clear());
            say(game, tr("torchplacer.cleared", &[]));
        }
        _ => say(game, tr("torchplacer.usage", &[])),
    }
}

/// The /torch usages, pushed so the chat box completes and colours the
/// command like the game's own; `command` above still executes them.
pub(super) fn register_client_commands(commands: &mut Vec<mod_api::Command>) {
    for usage in [
        "torch",
        "torch on",
        "torch off",
        "torch status",
        "torch level <level:integer>",
        "torch threshold <level:integer>",
        "torch radius <radius:integer>",
        "torch clear",
    ] {
        commands.push(mod_api::Command::new(usage));
    }
}
