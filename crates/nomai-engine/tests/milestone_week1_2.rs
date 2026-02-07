//! Week 1-2 Milestone Test: 10K entities, 5 systems, 1000 ticks, deterministic.
//!
//! This test validates that the hardened ECS + tick loop produces identical
//! results across runs. It uses blake3 to hash the final world state and
//! verifies the hash matches on a second run.

use nomai_ecs::prelude::*;
use nomai_engine::tick::{TickConfig, TickLoop};

// -- Component types --------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Position { x: f64, y: f64 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Velocity { dx: f64, dy: f64 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Health(u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Score(i64);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Lifetime(u32);

// -- Systems ----------------------------------------------------------------

fn movement_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (pos, vel)) in world.query::<(&Position, &Velocity)>() {
        cmds.set_component(
            entity, "position",
            serde_json::json!({"x": pos.x + vel.dx, "y": pos.y + vel.dy}),
            SystemId(1), CausalReason::SystemInternal("movement".to_owned()),
        );
    }
}

fn damage_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (health,)) in world.query::<(&Health,)>() {
        if health.0 > 0 {
            cmds.set_component(
                entity, "health", serde_json::json!(health.0.saturating_sub(1)),
                SystemId(2), CausalReason::GameRule("tick_damage".to_owned()),
            );
        }
    }
}

fn scoring_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (score, _health)) in world.query::<(&Score, &Health)>() {
        cmds.set_component(
            entity, "score", serde_json::json!(score.0 + 1),
            SystemId(3), CausalReason::GameRule("score_increment".to_owned()),
        );
    }
}

fn lifetime_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (lifetime,)) in world.query::<(&Lifetime,)>() {
        if lifetime.0 == 0 {
            cmds.despawn(entity, SystemId(4),
                CausalReason::GameRule("lifetime_expired".to_owned()));
        } else {
            cmds.set_component(
                entity, "lifetime", serde_json::json!(lifetime.0 - 1),
                SystemId(4), CausalReason::Timer("countdown".to_owned()),
            );
        }
    }
}

fn velocity_decay_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (vel,)) in world.query::<(&Velocity,)>() {
        cmds.set_component(
            entity, "velocity",
            serde_json::json!({"dx": vel.dx * 0.999, "dy": vel.dy * 0.999}),
            SystemId(5), CausalReason::SystemInternal("friction".to_owned()),
        );
    }
}

// -- World builder ----------------------------------------------------------

fn build_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Health>("health");
    world.register_component::<Score>("score");
    world.register_component::<Lifetime>("lifetime");

    // 4000 entities: Position + Velocity (movers)
    for i in 0..4000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: i as f64, y: (i as f64) * 0.5 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: -0.5 });
        world.spawn_bundle(b);
    }

    // 3000 entities: Position + Health + Score (scorers)
    for i in 0..3000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: -(i as f64), y: i as f64 });
        b.add(world.registry(), Health(1000));
        b.add(world.registry(), Score(0));
        world.spawn_bundle(b);
    }

    // 2000 entities: Position + Velocity + Lifetime (temporary movers)
    for i in 0..2000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: i as f64 * 2.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 0.5, dy: 0.5 });
        b.add(world.registry(), Lifetime(500 + (i % 500))); // expire between tick 500-999
        world.spawn_bundle(b);
    }

    // 1000 entities: Health only (static targets)
    for i in 0..1000u32 {
        world.spawn_with(Health(500 + i));
    }

    world
}

/// Hash the world state deterministically using blake3.
fn hash_world(world: &World) -> String {
    let mut hasher = blake3::Hasher::new();

    // Hash entity count.
    hasher.update(&(world.entity_count() as u64).to_le_bytes());

    // Hash all positions (sorted by entity for determinism).
    let mut positions: Vec<(u64, f64, f64)> = world.query::<(&Position,)>()
        .map(|(e, (p,))| (e.to_raw(), p.x, p.y))
        .collect();
    positions.sort_by_key(|(id, _, _)| *id);
    for (id, x, y) in &positions {
        hasher.update(&id.to_le_bytes());
        hasher.update(&x.to_le_bytes());
        hasher.update(&y.to_le_bytes());
    }

    // Hash all healths.
    let mut healths: Vec<(u64, u32)> = world.query::<(&Health,)>()
        .map(|(e, (h,))| (e.to_raw(), h.0))
        .collect();
    healths.sort_by_key(|(id, _)| *id);
    for (id, h) in &healths {
        hasher.update(&id.to_le_bytes());
        hasher.update(&h.to_le_bytes());
    }

    // Hash all scores.
    let mut scores: Vec<(u64, i64)> = world.query::<(&Score,)>()
        .map(|(e, (s,))| (e.to_raw(), s.0))
        .collect();
    scores.sort_by_key(|(id, _)| *id);
    for (id, s) in &scores {
        hasher.update(&id.to_le_bytes());
        hasher.update(&s.to_le_bytes());
    }

    hasher.finalize().to_hex().to_string()
}

fn run_simulation() -> (String, u64, usize) {
    let world = build_world();
    let config = TickConfig { fixed_dt: 1.0 / 60.0, headless: true };
    let mut tick_loop = TickLoop::new(world, config);

    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("damage", damage_system);
    tick_loop.add_system("scoring", scoring_system);
    tick_loop.add_system("lifetime", lifetime_system);
    tick_loop.add_system("velocity_decay", velocity_decay_system);

    let total_cmds = tick_loop.run_ticks(1000);
    let hash = hash_world(tick_loop.world());
    let final_count = tick_loop.world().entity_count();

    (hash, total_cmds, final_count)
}

#[test]
fn milestone_10k_entities_5_systems_1000_ticks_deterministic() {
    let (hash1, cmds1, count1) = run_simulation();
    let (hash2, cmds2, count2) = run_simulation();

    // Determinism: hashes must match.
    assert_eq!(hash1, hash2, "world state hash diverged between runs");

    // Determinism: same command count.
    assert_eq!(cmds1, cmds2, "total command count diverged");

    // Determinism: same final entity count.
    assert_eq!(count1, count2, "final entity count diverged");

    // Sanity checks:
    // Started with 10K entities.
    // 2000 have Lifetime(500-999), so all should be despawned by tick 1000.
    // Remaining: 4000 + 3000 + 1000 = 8000.
    assert_eq!(count1, 8000,
        "expected 8000 surviving entities (2000 with lifetime should be despawned)");

    // Commands should be substantial: at least 1M+ across 1000 ticks.
    assert!(cmds1 > 1_000_000,
        "expected >1M commands across 1000 ticks with 10K entities, got {cmds1}");

    println!("Milestone PASS: hash={hash1}, commands={cmds1}, entities={count1}");
}
