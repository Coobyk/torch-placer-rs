// Torch Placer — a reach-only helper mod for Horizon.
//
// Every `interval` ticks it looks for a nearby cell whose light level is at
// or below `threshold`, that is empty, that the player is not standing in,
// and that has a solid block under it. It selects a torch from the hotbar
// and places it on that block's top face through the normal use action, so
// vanilla placement rules, item consumption and reach checks still apply.
// It never walks anywhere: torches appear around wherever you are.

use std::collections::HashMap;
use std::sync::Mutex;

pub const ID: &str = "torchplacer";
pub const VERSION: &str = "0.1.2";
pub const DESCRIPTION: &str = "Places torches in nearby dark spots (reach only)";

/// 4.5 blocks — vanilla reach for placing a block.
const REACH2: f64 = 20.25;

struct S {
    enabled: bool,
    radius: i32,
    threshold: i32,
    interval: u32,
    ticks: u32,
    /// Ticks left holding right-click for the placement in flight.
    held: u8,
    hit: Option<crate::game::Hit>,
    target: [i32; 3],
    warned: bool,
}

static ST: Mutex<Option<S>> = Mutex::new(None);
fn with<R>(f: impl FnOnce(&mut S) -> R) -> R {
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(S {
            enabled: true,
            radius: 5,
            threshold: 8,
            interval: 10,
            ticks: 0,
            held: 0,
            hit: None,
            target: [0; 3],
            warned: false,
        });
    }
    f(g.as_mut().unwrap())
}

/// Cells we placed a torch in (or failed to) recently, with ticks remaining.
static COOL: Mutex<Option<HashMap<[i32; 3], u32>>> = Mutex::new(None);
fn cool<R>(f: impl FnOnce(&mut HashMap<[i32; 3], u32>) -> R) -> R {
    let mut g = COOL.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    f(g.as_mut().unwrap())
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

fn say(game: &mut crate::game::Game<'_>, m: &str) {
    game.native_chat(&format!("torch: {m}"));
}

fn onoff(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// Read persisted settings. Missing keys leave the defaults in place.
fn sync(game: &crate::game::Game<'_>) {
    if let Some(v) = game.native_setting(ID, "enabled") {
        with(|s| s.enabled = v == "true");
    }
    if let Some(v) = game.native_setting(ID, "radius") {
        if let Ok(n) = v.parse::<i32>() {
            with(|s| s.radius = n.clamp(2, 16));
        }
    }
    if let Some(v) = game.native_setting(ID, "threshold") {
        if let Ok(n) = v.parse::<i32>() {
            with(|s| s.threshold = n.clamp(0, 15));
        }
    }
}

fn set_enabled(game: &mut crate::game::Game<'_>, v: bool) {
    with(|s| s.enabled = v);
    game.native_set_setting(ID, "enabled", if v { "true" } else { "false" });
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
                .filter(|s| s.item.as_ref().contains("torch"))
                .map(|_| i)
        })
}

fn is_torch_at(game: &crate::game::Game<'_>, p: [i32; 3]) -> bool {
    game.res
        .facts()
        .get(game.world.block(p[0], p[1], p[2]))
        .name
        .contains("torch")
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
    let (x0, y0, z0) = (p[0] as f64, p[1] as f64, p[2] as f64);
    px + 0.3 > x0
        && px - 0.3 < x0 + 1.0
        && py + 1.8 > y0
        && py < y0 + 1.0
        && pz + 0.3 > z0
        && pz - 0.3 < z0 + 1.0
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
    // Horizon stores SKY light at its full daytime value even at night; the
    // effective level (what mob spawning uses) subtracts sky darken. Without
    // this, every outdoor cell reads 15 and nothing is ever "dark" at night.
    let darken = game.sky_darken();
    let mut best: Option<(f64, [i32; 3], crate::game::Hit)> = None;
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            for dy in -3..=4 {
                let cell = [me[0] + dx, me[1] + dy, me[2] + dz];
                let d2 = dist2(eye, center(cell));
                if d2 > REACH2 {
                    continue;
                }
                let (block, sky) = game.native_light(cell);
                let level = (block as i32).max(sky as i32 - darken);
                if level > threshold {
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
                let hit = if bf.full_collision && !bf.name.contains("torch") {
                    Some(crate::game::Hit {
                        pos: below,
                        face: [0, 1, 0],
                        point: [cell[0] as f64 + 0.5, below[1] as f64 + 1.0, cell[2] as f64 + 0.5],
                    })
                } else {
                    // Wall attachment: a solid side neighbour with the face
                    // pointing into our empty cell.
                    [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]]
                        .iter()
                        .find_map(|d| {
                            let sup = [cell[0] - d[0], cell[1], cell[2] - d[2]];
                            let f = facts.get(game.world.block(sup[0], sup[1], sup[2]));
                            if f.full_collision && !f.name.contains("torch") {
                                Some(crate::game::Hit {
                                    pos: sup,
                                    face: *d,
                                    point: [
                                        sup[0] as f64 + 0.5 + d[0] as f64 * 0.5,
                                        sup[1] as f64 + 0.5,
                                        sup[2] as f64 + 0.5 + d[2] as f64 * 0.5,
                                    ],
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
    if !game.native_on(ID) || !game.native_window() {
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
    if !with(|s| s.enabled) || !game.native_playing() {
        release(game);
        return;
    }

    // Finish an in-flight placement. `use_delay` is 4 ticks, so holding
    // right for 6 gives the use action a chance to land.
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
                c.insert(target, if placed { 200 } else { 60 });
            });
            with(|s| s.hit = None);
            release(game);
        }
        return;
    }

    release(game);
    let due = with(|s| {
        s.ticks += 1;
        if s.ticks >= s.interval {
            s.ticks = 0;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }

    let Some(slot) = torch_slot(game) else {
        if !with(|s| s.warned) {
            with(|s| s.warned = true);
            say(game, "no torch in the hotbar — put one in a slot 1-9");
        }
        return;
    };
    if let Some((cell, hit)) = scan(game, me, eye) {
        game.selected = slot;
        with(|s| {
            s.hit = Some(hit);
            s.held = 6;
            s.target = cell;
            s.warned = false;
        });
        claim(game, Some(hit));
    }
}

pub(super) fn command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if !game.native_on(ID) || !game.native_window() {
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
            say(game, &format!("auto-torch {}", onoff(v)));
        }
        Some("on") => {
            set_enabled(game, true);
            say(game, "auto-torch on");
        }
        Some("off") => {
            set_enabled(game, false);
            say(game, "auto-torch off");
        }
        Some("status") => {
            let (v, r, t, i) = with(|s| (s.enabled, s.radius, s.threshold, s.interval));
            say(
                game,
                &format!("auto-torch {} — light <= {t}, radius {r}, every {i} ticks", onoff(v)),
            );
        }
        Some("level" | "threshold") => {
            if let Some(n) = args.get(1).and_then(|v| v.parse::<i32>().ok()) {
                let n = n.clamp(0, 15);
                with(|s| s.threshold = n);
                game.native_set_setting(ID, "threshold", &n.to_string());
                say(game, &format!("light threshold {n}"));
            } else {
                say(game, "/torch level <0-15>");
            }
        }
        Some("radius") => {
            if let Some(n) = args.get(1).and_then(|v| v.parse::<i32>().ok()) {
                let n = n.clamp(2, 16);
                with(|s| s.radius = n);
                game.native_set_setting(ID, "radius", &n.to_string());
                say(game, &format!("radius {n} (actual reach is ~4.5 blocks)"));
            } else {
                say(game, "/torch radius <2-16>");
            }
        }
        Some("now") => {
            with(|s| s.ticks = s.interval);
            say(game, "scanning now");
        }
        Some("clear") => {
            cool(|c| c.clear());
            say(game, "placement history cleared");
        }
        _ => say(game, "/torch [on|off|status|level <n>|radius <n>|now|clear]"),
    }
}

pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.into(),
        title: format!("Torch Placer {VERSION}"),
        description: DESCRIPTION.into(),
        keys: Vec::new(),
    });
}

pub(super) fn page(game: &crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    sync(game);
    let (v, t, r) = with(|s| (s.enabled, s.threshold, s.radius));
    buttons.push(format!("Auto-torch: {}", onoff(v)));
    buttons.push(format!("Light level <= {t}"));
    buttons.push(format!("Radius: {r}"));
}

pub(super) fn click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    match button {
        0 => {
            let v = !with(|s| s.enabled);
            set_enabled(game, v);
        }
        1 => {
            let next = match with(|s| s.threshold) {
                0 => 4,
                4 => 7,
                7 => 10,
                10 => 15,
                _ => 0,
            };
            with(|s| s.threshold = next);
            game.native_set_setting(ID, "threshold", &next.to_string());
        }
        2 => {
            let next = {
                let r = with(|s| s.radius);
                if r < 16 { (r + 3).min(16) } else { 3 }
            };
            with(|s| s.radius = next);
            game.native_set_setting(ID, "radius", &next.to_string());
        }
        _ => {}
    }
}
