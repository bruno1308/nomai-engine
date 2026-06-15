#!/usr/bin/env python3
"""Nomai Prism Pop -- Match-3 Game with Manifest Verification.

A match-3 puzzle game ("Prism Pop") built on the Nomai engine.
Python drives the game logic through the engine's ECS command API.
The verification suite (auto-generated from the GDD) validates that
the game conforms to its design spec using manifest data alone.

Run:
  python demo_prism_pop.py

What happens:
  1. Spawns an 8x8 grid of jewel tiles, a score tracker, and a grid entity
  2. Plays through a sequence of swap attempts
  3. Verifies the run against the Prism Pop GDD verification suite
  4. Saves a regression baseline for future runs
  5. Prints a structured report
"""

from __future__ import annotations

import json
import logging
import random
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

from nomai.engine import NomaiEngine
from nomai.intents import VerificationSuite
from nomai.manifest import EntityEntry, TickManifest
from nomai.verify import RegressionTest, VerificationEngine, VerificationReport

logging.basicConfig(
    level=logging.WARNING,
    format="%(levelname)s: %(message)s",
)
logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Constants (from GDD)
# ---------------------------------------------------------------------------

GRID_SIZE = 8
TILE_TYPES = 5
TILE_NAMES = ("ruby", "sapphire", "emerald", "topaz", "amethyst")
WIN_SCORE = 2500
POINTS_3 = 300
POINTS_4 = 500
POINTS_5 = 1000

SUITE_PATH = Path(__file__).parent / "python" / "nomai-sdk" / "specs" / "prism_pop" / "suite.json"
REGRESSION_DIR = Path(__file__).parent / "tests" / "regression"


# ---------------------------------------------------------------------------
# Match-3 Grid Logic
# ---------------------------------------------------------------------------

@dataclass
class Match3Grid:
    """Pure-logic match-3 grid.

    Holds the grid state and implements swap validation, match detection,
    gravity fill, and scoring. All game rules from the GDD live here.
    """
    grid: list[list[int]] = field(default_factory=list)
    score: int = 0
    _rng: random.Random = field(default_factory=lambda: random.Random(42))

    def __post_init__(self) -> None:
        if not self.grid:
            self.grid = self._init_grid()

    def _init_grid(self) -> list[list[int]]:
        """Fill grid with random tiles, ensuring no initial matches."""
        g: list[list[int]] = [[0] * GRID_SIZE for _ in range(GRID_SIZE)]
        for r in range(GRID_SIZE):
            for c in range(GRID_SIZE):
                tile = self._rng.randint(0, TILE_TYPES - 1)
                while self._creates_match(g, r, c, tile):
                    tile = (tile + 1) % TILE_TYPES
                g[r][c] = tile
        return g

    @staticmethod
    def _creates_match(
        g: list[list[int]], row: int, col: int, tile: int,
    ) -> bool:
        if col >= 2 and g[row][col - 1] == tile and g[row][col - 2] == tile:
            return True
        if row >= 2 and g[row - 1][col] == tile and g[row - 2][col] == tile:
            return True
        return False

    def try_swap(
        self, r1: int, c1: int, r2: int, c2: int,
    ) -> tuple[bool, int, list[tuple[int, int]]]:
        """Attempt a swap. Returns (accepted, points, destroyed_cells).

        If the swap forms a match, resolves the full cascade (match ->
        destroy -> gravity fill -> repeat) and returns the total points
        and all cells that were destroyed. If no match, swaps back.
        """
        if not self._adjacent(r1, c1, r2, c2):
            return False, 0, []

        # Swap
        self.grid[r1][c1], self.grid[r2][c2] = (
            self.grid[r2][c2], self.grid[r1][c1]
        )

        if self._has_match_at(r1, c1) or self._has_match_at(r2, c2):
            total_points = 0
            all_destroyed: list[tuple[int, int]] = []
            cascade = 0

            while cascade < 20:
                matched = self._find_matches()
                if not matched:
                    break
                points = self._score_matched(matched)
                total_points += points
                all_destroyed.extend(matched)

                for mr, mc in matched:
                    self.grid[mr][mc] = -1

                self._apply_gravity()
                cascade += 1

            self.score += total_points
            return True, total_points, all_destroyed

        # No match -- swap back
        self.grid[r1][c1], self.grid[r2][c2] = (
            self.grid[r2][c2], self.grid[r1][c1]
        )
        return False, 0, []

    @staticmethod
    def _adjacent(r1: int, c1: int, r2: int, c2: int) -> bool:
        return (abs(r1 - r2) + abs(c1 - c2)) == 1

    def _has_match_at(self, row: int, col: int) -> bool:
        tile = self.grid[row][col]
        if tile < 0:
            return False
        # Horizontal
        h = 1
        c = col - 1
        while c >= 0 and self.grid[row][c] == tile:
            h += 1
            c -= 1
        c = col + 1
        while c < GRID_SIZE and self.grid[row][c] == tile:
            h += 1
            c += 1
        if h >= 3:
            return True
        # Vertical
        v = 1
        r = row - 1
        while r >= 0 and self.grid[r][col] == tile:
            v += 1
            r -= 1
        r = row + 1
        while r < GRID_SIZE and self.grid[r][col] == tile:
            v += 1
            r += 1
        if v >= 3:
            return True
        return False

    def _find_matches(self) -> set[tuple[int, int]]:
        """Find all cells that are part of a match of 3+."""
        matched: set[tuple[int, int]] = set()

        # Horizontal runs
        for r in range(GRID_SIZE):
            start = 0
            for c in range(1, GRID_SIZE):
                if (self.grid[r][c] == self.grid[r][start]
                        and self.grid[r][c] >= 0):
                    continue
                if c - start >= 3:
                    for k in range(start, c):
                        matched.add((r, k))
                start = c
            if GRID_SIZE - start >= 3 and self.grid[r][start] >= 0:
                for k in range(start, GRID_SIZE):
                    matched.add((r, k))

        # Vertical runs
        for c in range(GRID_SIZE):
            start = 0
            for r in range(1, GRID_SIZE):
                if (self.grid[r][c] == self.grid[start][c]
                        and self.grid[r][c] >= 0):
                    continue
                if r - start >= 3:
                    for k in range(start, r):
                        matched.add((k, c))
                start = r
            if GRID_SIZE - start >= 3 and self.grid[start][c] >= 0:
                for k in range(start, GRID_SIZE):
                    matched.add((k, c))

        return matched

    @staticmethod
    def _score_matched(matched: set[tuple[int, int]]) -> int:
        """Score based on number of matched tiles."""
        n = len(matched)
        if n >= 5:
            return POINTS_5
        if n >= 4:
            return POINTS_4
        if n >= 3:
            return POINTS_3
        return 0

    def _apply_gravity(self) -> None:
        """Tiles fall down; empty cells at top filled with new tiles."""
        for c in range(GRID_SIZE):
            write = GRID_SIZE - 1
            for r in range(GRID_SIZE - 1, -1, -1):
                if self.grid[r][c] >= 0:
                    self.grid[write][c] = self.grid[r][c]
                    if write != r:
                        self.grid[r][c] = -1
                    write -= 1
            for r in range(write, -1, -1):
                self.grid[r][c] = self._rng.randint(0, TILE_TYPES - 1)

    def count_valid_moves(self) -> int:
        """Count how many valid swaps exist on the board."""
        count = 0
        for r in range(GRID_SIZE):
            for c in range(GRID_SIZE):
                if c + 1 < GRID_SIZE:
                    self.grid[r][c], self.grid[r][c + 1] = (
                        self.grid[r][c + 1], self.grid[r][c]
                    )
                    if self._has_match_at(r, c) or self._has_match_at(r, c + 1):
                        count += 1
                    self.grid[r][c], self.grid[r][c + 1] = (
                        self.grid[r][c + 1], self.grid[r][c]
                    )
                if r + 1 < GRID_SIZE:
                    self.grid[r][c], self.grid[r + 1][c] = (
                        self.grid[r + 1][c], self.grid[r][c]
                    )
                    if self._has_match_at(r, c) or self._has_match_at(r + 1, c):
                        count += 1
                    self.grid[r][c], self.grid[r + 1][c] = (
                        self.grid[r + 1][c], self.grid[r][c]
                    )
        return count


# ---------------------------------------------------------------------------
# Engine setup
# ---------------------------------------------------------------------------

def create_engine_with_grid() -> tuple[
    NomaiEngine, Match3Grid, dict[str, list[int]], dict[tuple[int, int], int]
]:
    """Create engine, spawn match-3 entities from Python.

    Returns (engine, grid_logic, roles, cell_entity_map).
    """
    engine = NomaiEngine(fixed_dt=1.0 / 60.0)
    engine.register_component("position")
    engine.register_component("size")
    engine.register_component("tile_type")
    engine.register_component("score_value")
    engine.register_component("state")
    engine.register_component("identity")

    grid_logic = Match3Grid()

    # Spawn grid entity
    engine.spawn_entity("container", "grid", {
        "position": {"x": 0, "y": 0},
        "size": {"w": GRID_SIZE, "h": GRID_SIZE},
    })

    # Spawn score entity
    engine.spawn_entity("ui", "score", {
        "position": {"x": 0, "y": 0},
        "score_value": {"points": 0},
    })

    # Spawn 64 tile entities
    for r in range(GRID_SIZE):
        for c in range(GRID_SIZE):
            tile = grid_logic.grid[r][c]
            engine.spawn_entity("tile", "tile", {
                "position": {"x": c, "y": r},
                "size": {"w": 1, "h": 1},
                "tile_type": {"type_id": tile, "name": TILE_NAMES[tile]},
            })

    # Apply spawns
    engine.tick()

    # Build role -> [entity_id] and cell -> entity_id maps
    index = engine.entity_index()
    roles: dict[str, list[int]] = {}
    for entry in index:
        roles.setdefault(entry.role, []).append(entry.entity_id)

    tile_ids = sorted(roles.get("tile", []))
    cell_map: dict[tuple[int, int], int] = {}
    i = 0
    for r in range(GRID_SIZE):
        for c in range(GRID_SIZE):
            if i < len(tile_ids):
                cell_map[(r, c)] = tile_ids[i]
                i += 1

    return engine, grid_logic, roles, cell_map


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
# Swap sequence
# ---------------------------------------------------------------------------

def generate_swap_sequence() -> list[tuple[int, int, int, int]]:
    """Generate swap sequence covering the whole grid.

    Tries every adjacent horizontal and vertical pair. The game logic
    rejects swaps that don't form matches, so this is a brute-force
    "player" that explores all possibilities.
    """
    swaps: list[tuple[int, int, int, int]] = []
    for r in range(GRID_SIZE):
        for c in range(GRID_SIZE - 1):
            swaps.append((r, c, r, c + 1))
    for r in range(GRID_SIZE - 1):
        for c in range(GRID_SIZE):
            swaps.append((r, c, r + 1, c))
    return swaps


# ---------------------------------------------------------------------------
# Verification suite
# ---------------------------------------------------------------------------

def load_suite() -> VerificationSuite:
    """Load the auto-generated verification suite from the GDD spec."""
    if SUITE_PATH.exists():
        suite = VerificationSuite.load(SUITE_PATH)

        # Remove shard-related intents -- shards are a particle effect
        # that we haven't implemented yet (core mechanics only)
        shard_intents = {
            "shard_exists", "shard_x_bounds", "shard_y_bounds",
            "shard_speed_dx", "shard_speed_dy",
        }
        suite.intents = [
            i for i in suite.intents if i.name not in shard_intents
        ]
        return suite

    print("  WARNING: Suite file not found at", SUITE_PATH)
    print("  Run: /parse-gdd match3-gdd.md  to generate it")
    sys.exit(1)


# ---------------------------------------------------------------------------
# Simulation runner
# ---------------------------------------------------------------------------

def run_simulation(
    engine: NomaiEngine,
    grid_logic: Match3Grid,
    roles: dict[str, list[int]],
    cell_map: dict[tuple[int, int], int],
    swaps: list[tuple[int, int, int, int]],
) -> tuple[list[TickManifest], int, int, int]:
    """Run swaps through the engine, return manifests and stats."""
    manifests: list[TickManifest] = []
    accepted = 0
    rejected = 0
    total_points = 0

    score_ids = roles.get("score", [])
    grid_ids = roles.get("grid", [])
    score_eid = score_ids[0] if score_ids else 0
    grid_eid = grid_ids[0] if grid_ids else 0

    for i, (r1, c1, r2, c2) in enumerate(swaps):
        ok, points, destroyed = grid_logic.try_swap(r1, c1, r2, c2)

        if ok:
            accepted += 1
            total_points += points

            # Push updated score to ECS
            engine.set_component(score_eid, "score_value", {
                "points": grid_logic.score,
            })

            # Push updated tile_type for all cells
            for r in range(GRID_SIZE):
                for c in range(GRID_SIZE):
                    tile = grid_logic.grid[r][c]
                    eid = cell_map.get((r, c))
                    if eid is not None:
                        name = TILE_NAMES[tile] if 0 <= tile < TILE_TYPES else "empty"
                        engine.set_component(eid, "tile_type", {
                            "type_id": tile,
                            "name": name,
                        })
                        engine.set_component(eid, "position", {
                            "x": c, "y": r,
                        })

            # Push grid state
            engine.set_component(grid_eid, "state", {
                "valid_moves_count": grid_logic.count_valid_moves(),
                "cascade_depth": 0,
                "phase": 0,
            })

            engine.set_input({
                "event": "swap_accepted",
                "r1": r1, "c1": c1, "r2": r2, "c2": c2,
                "points": points,
            })
        else:
            rejected += 1
            engine.set_input({
                "event": "swap_rejected",
                "r1": r1, "c1": c1, "r2": r2, "c2": c2,
            })

        m = engine.tick()
        manifests.append(m)
        engine.set_input({})

        if (i + 1) % 40 == 0:
            print(f"    Swaps: {i + 1}/{len(swaps)}, "
                  f"accepted: {accepted}, score: {grid_logic.score}")

        if grid_logic.score >= WIN_SCORE:
            print(f"    WIN at swap {i + 1}! Score: {grid_logic.score}")
            break

    return manifests, accepted, rejected, total_points


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    """Run Prism Pop and verify it against the GDD spec."""
    print("=" * 70)
    print("  NOMAI PRISM POP")
    print("  Match-3 puzzle game with manifest-based verification")
    print("=" * 70)

    # --- Play the game ---------------------------------------------------
    print("\n  Setting up...")
    engine, grid_logic, roles, cell_map = create_engine_with_grid()
    print(f"  Entities: {engine.entity_count}")
    print(f"  Valid moves on initial board: {grid_logic.count_valid_moves()}")

    swaps = generate_swap_sequence()
    print(f"\n  Playing {len(swaps)} swap attempts...")
    t0 = time.monotonic()
    manifests, accepted, rejected, points = run_simulation(
        engine, grid_logic, roles, cell_map, swaps,
    )
    elapsed = (time.monotonic() - t0) * 1000
    print(f"\n  Game finished in {elapsed:.1f}ms ({len(manifests)} ticks)")
    print(f"  Swaps: {accepted} accepted, {rejected} rejected")
    print(f"  Final score: {grid_logic.score}")

    # --- Verify against GDD spec -----------------------------------------
    print("\n" + "-" * 70)
    print("  VERIFICATION (from GDD spec)")
    print("-" * 70)

    suite = load_suite()
    verifier = VerificationEngine()
    entity_dict = build_entity_index_dict(engine.entity_index())
    report = verifier.verify(suite, manifests, entity_dict)

    print(report.summary())

    if not report.all_passed:
        print("\n  Issues found:")
        print(report.diagnosis())

    # --- Regression baseline ---------------------------------------------
    print("\n" + "-" * 70)
    print("  REGRESSION BASELINE")
    print("-" * 70)

    reg_test = RegressionTest.create(
        name="prism_pop_baseline",
        suite=suite,
        manifests=manifests,
        report=report,
    )
    reg_path = REGRESSION_DIR / "prism_pop_baseline.json"
    reg_test.save(reg_path)
    print(f"  Saved to {reg_path}")

    loaded = RegressionTest.load(reg_path)
    replay_report = verifier.verify(loaded.suite, loaded.manifests, entity_dict)
    regression_ok = (
        replay_report.passed == loaded.expected_pass_count
        and replay_report.failed == loaded.expected_fail_count
    )
    print(f"  Replay: {'PASS' if regression_ok else 'FAIL'} "
          f"({replay_report.passed}P/{replay_report.failed}F)")

    # --- JSON report -----------------------------------------------------
    report_data = {
        "game": "prism_pop",
        "score": grid_logic.score,
        "swaps_accepted": accepted,
        "swaps_rejected": rejected,
        "ticks": len(manifests),
        "verification": report.to_dict(),
        "regression": "PASS" if regression_ok else "FAIL",
    }
    report_path = Path(__file__).parent / "demo_prism_pop_report.json"
    report_path.write_text(json.dumps(report_data, indent=2), encoding="utf-8")
    print(f"  JSON report: {report_path}")

    # --- Exit / Window ----------------------------------------------------
    print("\n" + "=" * 70)
    if report.all_passed and regression_ok:
        print("  All verifications passed.")
    else:
        print(f"  {report.failed} verification failure(s).")
    print("=" * 70)

    # Open a window to visualise the final game state.
    # The window blocks until closed; manifests remain accessible after.
    if "--headless" not in sys.argv:
        print("\n  Opening Prism Pop window (close to exit)...")
        engine.run(title="Prism Pop", width=800, height=600)

    return 0 if report.all_passed and regression_ok else 1


if __name__ == "__main__":
    sys.exit(main())
