#!/usr/bin/env python3
"""Nomai Breakout Demo -- End-to-End Verification Loop.

Demonstrates the Nomai verification thesis: behavioral correctness can
be determined from manifest data alone, without pixel peeking.

Architecture:
  Python orchestration drives the Rust engine (via PyO3), loads WASM
  gameplay modules, runs verification against intent specs, and produces
  structured reports.

Flow:
  Phase 1: Buggy    -- create engine, spawn entities, run WITHOUT
                       collision response, verify → failures detected
  Phase 2: Fixed    -- fresh engine, same entities, run WITH
                       manifest-driven collision response, verify → passes
  Phase 3: Regress  -- save passing run as regression test, replay it
  Phase 4: Report   -- structured summary of the full cycle

Design notes:
  - Uses print() for structured demo output (not logging) because this
    is a user-facing CLI demo with formatted tables and progress lines.
  - Phase 2 creates a fresh engine instead of hot-swapping WASM and
    restoring a snapshot. The Python-driven collision response loop
    demonstrates the manifest thesis more directly: the manifest drives
    game rules from Python, which is the project's core value proposition.
    Hot-swap and snapshot/restore are tested separately in milestone tests.
"""

from __future__ import annotations

import json
import logging
import sys
import time
from pathlib import Path

from nomai.breakout_intents import build_breakout_suite
from nomai.engine import NomaiEngine
from nomai.intents import (
    IntentKind,
    IntentSpec,
    VerificationSuite,
    entity_despawned,
    event_occurred,
    tick_reached,
)
from nomai.manifest import EntityEntry, TickManifest
from nomai.verify import RegressionTest, VerificationEngine, VerificationReport

logging.basicConfig(
    level=logging.WARNING,
    format="%(levelname)s: %(message)s",
)
logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Game layout constants
# ---------------------------------------------------------------------------

GAME_WIDTH = 800.0
GAME_HEIGHT = 600.0
BRICK_ROWS = 4
BRICK_COLS = 5
BRICK_WIDTH = 60.0
BRICK_HEIGHT = 20.0
BRICK_SPACING = 10.0
PADDLE_WIDTH = 100.0
PADDLE_HEIGHT = 15.0
BALL_RADIUS = 8.0
WALL_THICKNESS = 20.0

# Ball velocity in rapier units/second. With dt=1/60, ball moves ~3.3
# units/tick horizontally and ~5 vertically. Should reach walls within
# ~120 ticks and hit bricks within ~50 ticks.
BALL_VX = 200.0
BALL_VY = -300.0

SIM_TICKS = 300

WASM_DIR = Path(__file__).parent / "gameplay" / "build"
BUGGY_WASM = WASM_DIR / "breakout_buggy.wasm"
FIXED_WASM = WASM_DIR / "gameplay.wasm"
REGRESSION_DIR = Path(__file__).parent / "tests" / "regression"


# ---------------------------------------------------------------------------
# Demo verification suite
# ---------------------------------------------------------------------------

def build_demo_suite() -> VerificationSuite:
    """Build a verification suite adapted for the current engine.

    Uses the canonical breakout entity/metric/invariant intents from
    ``build_breakout_suite()``, plus demo-adapted behavior intents that
    use ``tick_reached`` triggers (since the physics collision events
    contain entity IDs, not role names, collision triggers don't match
    yet -- full role-enriched collision events are post-MVP).

    The key behavior intent for thesis demonstration:
    ``brick_destroyed_after_physics`` checks that at least one brick
    entity is despawned after the ball has had time to reach the bricks.
    This passes in the fixed run (Python collision response) and fails
    in the buggy run (no collision response).
    """
    canonical = build_breakout_suite()
    # Keep entity, metric, and invariant intents from canonical suite
    adapted: list[IntentSpec] = [
        i for i in canonical.intents
        if i.kind in (IntentKind.ENTITY, IntentKind.METRIC, IntentKind.INVARIANT)
    ]
    # Add demo-adapted behavior intents
    adapted.append(IntentSpec(
        name="brick_destroyed_after_physics",
        kind=IntentKind.BEHAVIOR,
        description=(
            "During the simulation, at least one brick must be "
            "despawned -- proving the collision response works."
        ),
        trigger=tick_reached(1),
        expected=entity_despawned("brick"),
        timeout_ticks=300,
    ))
    adapted.append(IntentSpec(
        name="collision_causes_brick_despawn",
        kind=IntentKind.BEHAVIOR,
        description=(
            "After a physics collision event occurs, at least one "
            "brick must be despawned by the collision response."
        ),
        trigger=event_occurred("collision"),
        expected=entity_despawned("brick"),
        timeout_ticks=200,
    ))
    return VerificationSuite(
        name="breakout_demo",
        description=(
            "Demo-adapted breakout verification suite. Uses canonical "
            "entity/metric/invariant intents with demo-specific behavior "
            "intents adapted for current physics event format."
        ),
        intents=adapted,
    )


# ---------------------------------------------------------------------------
# Engine setup helper
# ---------------------------------------------------------------------------

def _brick_positions() -> list[tuple[float, float]]:
    """Compute brick center positions from the grid layout."""
    positions: list[tuple[float, float]] = []
    start_x = (GAME_WIDTH - (BRICK_COLS * (BRICK_WIDTH + BRICK_SPACING))) / 2
    for row in range(BRICK_ROWS):
        for col in range(BRICK_COLS):
            x = start_x + col * (BRICK_WIDTH + BRICK_SPACING) + BRICK_WIDTH / 2
            y = 60.0 + row * (BRICK_HEIGHT + BRICK_SPACING)
            positions.append((x, y))
    return positions


def create_engine_with_entities(
    wasm_path: Path,
) -> tuple[NomaiEngine, dict[str, list[int]]]:
    """Create a fresh engine, spawn breakout entities, register physics.

    Returns the engine and a mapping of role -> [entity_id, ...].
    """
    engine = NomaiEngine(headless=True, fixed_dt=1.0 / 60.0)
    engine.register_component("position")
    engine.register_component("velocity")
    engine.register_component("size")
    engine.register_component("score")
    engine.register_component("game_state")
    engine.register_component("identity")
    engine.init_physics()

    # Spawn entities via command buffer
    engine.spawn_entity("character", "paddle", {
        "position": {"x": GAME_WIDTH / 2, "y": GAME_HEIGHT - 40},
        "size": {"w": PADDLE_WIDTH, "h": PADDLE_HEIGHT},
    })
    engine.spawn_entity("projectile", "ball", {
        "position": {"x": GAME_WIDTH / 2, "y": GAME_HEIGHT / 2},
        "velocity": {"dx": BALL_VX, "dy": BALL_VY},
    })
    brick_pos = _brick_positions()
    for bx, by in brick_pos:
        engine.spawn_entity("destructible", "brick", {
            "position": {"x": bx, "y": by},
            "size": {"w": BRICK_WIDTH, "h": BRICK_HEIGHT},
        })
    engine.spawn_entity("boundary", "wall_top", {
        "position": {"x": GAME_WIDTH / 2, "y": -WALL_THICKNESS / 2},
    })
    engine.spawn_entity("boundary", "wall_left", {
        "position": {"x": -WALL_THICKNESS / 2, "y": GAME_HEIGHT / 2},
    })
    engine.spawn_entity("boundary", "wall_right", {
        "position": {"x": GAME_WIDTH + WALL_THICKNESS / 2, "y": GAME_HEIGHT / 2},
    })

    # Apply spawns
    engine.tick()

    # Build role -> [entity_id] mapping
    index = engine.entity_index()
    roles: dict[str, list[int]] = {}
    for entry in index:
        roles.setdefault(entry.role, []).append(entry.entity_id)

    # Register physics bodies
    engine.register_physics_entity(
        roles["paddle"][0], GAME_WIDTH / 2, GAME_HEIGHT - 40, 0, 0,
        "kinematic", "box",
        collider_half_width=PADDLE_WIDTH / 2,
        collider_half_height=PADDLE_HEIGHT / 2,
        restitution=1.0,
    )
    engine.register_physics_entity(
        roles["ball"][0], GAME_WIDTH / 2, GAME_HEIGHT / 2, BALL_VX, BALL_VY,
        "dynamic", "circle",
        collider_radius=BALL_RADIUS,
        restitution=1.0,
    )
    brick_ids = sorted(roles.get("brick", []))
    for i, brick_id in enumerate(brick_ids):
        bx, by = brick_pos[i]
        engine.register_physics_entity(
            brick_id, bx, by, 0, 0,
            "static", "box",
            collider_half_width=BRICK_WIDTH / 2,
            collider_half_height=BRICK_HEIGHT / 2,
            restitution=1.0,
        )
    for wall_role, wx, wy, hw, hh in [
        ("wall_top", GAME_WIDTH / 2, -WALL_THICKNESS / 2,
         GAME_WIDTH / 2, WALL_THICKNESS / 2),
        ("wall_left", -WALL_THICKNESS / 2, GAME_HEIGHT / 2,
         WALL_THICKNESS / 2, GAME_HEIGHT / 2),
        ("wall_right", GAME_WIDTH + WALL_THICKNESS / 2, GAME_HEIGHT / 2,
         WALL_THICKNESS / 2, GAME_HEIGHT / 2),
    ]:
        engine.register_physics_entity(
            roles[wall_role][0], wx, wy, 0, 0,
            "static", "box",
            collider_half_width=hw, collider_half_height=hh,
            restitution=1.0,
        )

    # Load WASM
    wasm_bytes = wasm_path.read_bytes()
    engine.load_gameplay_wasm(wasm_bytes)

    return engine, roles


def build_entity_index_dict(
    entries: list[EntityEntry],
) -> dict[str, dict[str, str]]:
    """Convert entity index entries to the dict format verify() expects."""
    result: dict[str, dict[str, str]] = {}
    for entry in entries:
        if entry.alive and entry.role not in result:
            result[entry.role] = {
                "entity_type": entry.entity_type,
                "role": entry.role,
                "tier": entry.tier,
            }
    return result


# ---------------------------------------------------------------------------
# Phase 1: Buggy run
# ---------------------------------------------------------------------------

def run_buggy() -> tuple[VerificationReport, list[TickManifest]]:
    """Run simulation without collision response (buggy behavior)."""
    print("\n" + "=" * 70)
    print("PHASE 1: BUGGY RUN (no collision response)")
    print("=" * 70)

    engine, roles = create_engine_with_entities(BUGGY_WASM)
    print(f"  Engine created: {engine.entity_count} entities, "
          f"buggy WASM loaded ({BUGGY_WASM.stat().st_size} bytes)")

    # Run simulation -- no collision handling
    print(f"  Running {SIM_TICKS} ticks...")
    t0 = time.monotonic()
    manifests = engine.run_ticks(SIM_TICKS)
    elapsed = (time.monotonic() - t0) * 1000
    print(f"  Simulation: {elapsed:.1f}ms ({elapsed / SIM_TICKS:.2f}ms/tick)")

    collision_count = sum(
        1 for m in manifests for e in m.events if e.event_type == "collision"
    )
    print(f"  Physics collisions detected: {collision_count}")

    # Verify
    suite = build_demo_suite()
    verifier = VerificationEngine()
    entity_dict = build_entity_index_dict(engine.entity_index())
    report = verifier.verify(suite, manifests, entity_dict)

    print(f"\n  --- Verification Result ---")
    print(report.summary())
    if not report.all_passed:
        print(f"\n  --- Diagnosis ---")
        print(report.diagnosis())

    return report, manifests


# ---------------------------------------------------------------------------
# Phase 2: Fixed run
# ---------------------------------------------------------------------------

def run_fixed() -> tuple[VerificationReport, list[TickManifest], dict[str, dict[str, str]]]:
    """Run simulation WITH manifest-driven collision response.

    After each tick, Python inspects the manifest for ball-brick
    collisions and despawns the hit brick. This demonstrates the
    manifest-driven game rule loop that the verification thesis enables.
    """
    print("\n" + "=" * 70)
    print("PHASE 2: FIXED RUN (manifest-driven collision response)")
    print("=" * 70)

    engine, roles = create_engine_with_entities(FIXED_WASM)
    print(f"  Engine created: {engine.entity_count} entities, "
          f"fixed WASM loaded ({FIXED_WASM.stat().st_size} bytes)")

    # Run simulation with collision response
    print(f"  Running {SIM_TICKS} ticks with collision response...")
    t0 = time.monotonic()
    manifests: list[TickManifest] = []
    ball_ids = set(roles.get("ball", []))
    brick_ids_alive = set(roles.get("brick", []))
    bricks_destroyed = 0

    for _ in range(SIM_TICKS):
        m = engine.tick()
        manifests.append(m)

        # Inspect manifest for ball-brick collisions
        for event in m.events:
            if event.event_type != "collision":
                continue
            involved = set(event.involved_entities)
            ball_hit = involved & ball_ids
            brick_hit = involved & brick_ids_alive
            if ball_hit and brick_hit:
                for brick_id in brick_hit:
                    engine.despawn_entity(brick_id)
                    brick_ids_alive.discard(brick_id)
                    bricks_destroyed += 1

    elapsed = (time.monotonic() - t0) * 1000
    print(f"  Simulation: {elapsed:.1f}ms ({elapsed / SIM_TICKS:.2f}ms/tick)")
    print(f"  Bricks destroyed: {bricks_destroyed}, "
          f"remaining: {len(brick_ids_alive)}")

    # Verify
    suite = build_demo_suite()
    verifier = VerificationEngine()
    entity_dict = build_entity_index_dict(engine.entity_index())
    report = verifier.verify(suite, manifests, entity_dict)

    print(f"\n  --- Verification Result ---")
    print(report.summary())
    if not report.all_passed:
        print(f"\n  --- Remaining Failures ---")
        for f in report.failures():
            print(f"  [{f.intent_name}] {f.failure_reason}")

    return report, manifests, entity_dict


# ---------------------------------------------------------------------------
# Phase 3: Regression test
# ---------------------------------------------------------------------------

def run_regression(
    fixed_report: VerificationReport,
    fixed_manifests: list[TickManifest],
    entity_dict: dict[str, dict[str, str]],
) -> bool:
    """Save and replay a regression test from the fixed run.

    Returns True if regression replay matches expected counts.
    """
    print("\n" + "=" * 70)
    print("PHASE 3: REGRESSION TEST")
    print("=" * 70)

    suite = build_demo_suite()
    reg_test = RegressionTest.create(
        name="breakout_fixed_baseline",
        suite=suite,
        manifests=fixed_manifests,
        report=fixed_report,
    )

    reg_path = REGRESSION_DIR / "breakout_fixed_baseline.json"
    reg_test.save(reg_path)
    print(f"  Saved regression test to {reg_path}")

    # Replay: re-verify saved manifests and compare pass/fail counts.
    # We pass entity_dict so entity intents can resolve roles.
    loaded = RegressionTest.load(reg_path)
    verifier = VerificationEngine()
    replay_report = verifier.verify(
        loaded.suite, loaded.manifests, entity_dict
    )
    passed = (
        replay_report.passed == loaded.expected_pass_count
        and replay_report.failed == loaded.expected_fail_count
    )
    status = "PASS" if passed else "FAIL"
    print(f"  Regression replay: {status}")
    print(f"    Expected: {loaded.expected_pass_count} pass / "
          f"{loaded.expected_fail_count} fail")
    print(f"    Actual:   {replay_report.passed} pass / "
          f"{replay_report.failed} fail")
    return passed


# ---------------------------------------------------------------------------
# Phase 4: Report
# ---------------------------------------------------------------------------

def print_final_report(
    buggy_report: VerificationReport,
    fixed_report: VerificationReport,
) -> None:
    """Print structured summary comparing buggy vs fixed."""
    print("\n" + "=" * 70)
    print("PHASE 4: FINAL REPORT")
    print("=" * 70)

    print(f"\n  {'Intent':<35} {'Buggy':>8} {'Fixed':>8}")
    print(f"  {'-' * 35} {'-' * 8} {'-' * 8}")

    buggy_map = {r.intent_name: r.passed for r in buggy_report.results}
    fixed_map = {r.intent_name: r.passed for r in fixed_report.results}

    for name in buggy_map:
        b = "PASS" if buggy_map.get(name) else "FAIL"
        f = "PASS" if fixed_map.get(name) else "FAIL"
        marker = " <-- FIXED" if b == "FAIL" and f == "PASS" else ""
        print(f"  {name:<35} {b:>8} {f:>8}{marker}")

    bp, bf = buggy_report.passed, buggy_report.failed
    fp, ff = fixed_report.passed, fixed_report.failed
    print(f"\n  {'TOTALS':<35} {bp}P/{bf}F    {fp}P/{ff}F")

    improvement = fp - bp
    if improvement > 0:
        print(f"\n  Verification improvement: +{improvement} intents now passing")
        print("  THESIS VALIDATED: Manifest-based verification detected buggy "
              "behavior")
        print("  and confirmed the fix, without pixel peeking.")
    elif fixed_report.all_passed:
        print("\n  All intents pass in both runs.")
    else:
        print(f"\n  Fixed run still has {ff} failure(s) -- "
              "some intents require post-MVP features.")

    # Save JSON report
    report_data = {
        "demo": "breakout_verification",
        "thesis": "manifest-based verification without pixel peeking",
        "buggy_run": buggy_report.to_dict(),
        "fixed_run": fixed_report.to_dict(),
        "improvement": improvement,
        "conclusion": (
            "VALIDATED" if improvement > 0
            else "PARTIAL" if not fixed_report.all_passed
            else "CLEAN"
        ),
    }
    report_path = Path(__file__).parent / "demo_report.json"
    report_path.write_text(json.dumps(report_data, indent=2), encoding="utf-8")
    print(f"\n  JSON report: {report_path}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    """Run the complete end-to-end verification demo."""
    print("=" * 70)
    print("  NOMAI BREAKOUT -- End-to-End Verification Demo")
    print("  Proving: AI can verify game behavior from manifest alone")
    print("=" * 70)

    for path, label in [(BUGGY_WASM, "buggy"), (FIXED_WASM, "fixed")]:
        if not path.exists():
            print(f"ERROR: {label} WASM not found at {path}")
            print("Run: cd gameplay && npm run build:all")
            return 1

    buggy_report, _buggy_manifests = run_buggy()
    fixed_report, fixed_manifests, entity_dict = run_fixed()
    regression_ok = run_regression(fixed_report, fixed_manifests, entity_dict)
    print_final_report(buggy_report, fixed_report)

    print("\n" + "=" * 70)
    print("  Demo complete.")
    print("=" * 70)

    if not fixed_report.all_passed:
        print("  EXIT: Fixed run has failures.")
        return 1
    if not regression_ok:
        print("  EXIT: Regression test failed.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
