"""High-level Python wrapper around the Rust NomaiEngine via PyO3.

The native extension module ``nomai._engine`` provides the raw FFI layer.
This module wraps it with typed Python APIs that return proper dataclasses
instead of raw dicts.
"""

from __future__ import annotations

import logging
from typing import Any, Callable

from nomai.manifest import (
    CausalChain,
    EntityEntry,
    TickManifest,
)
from nomai.replay import EngineSnapshot, ReplayLog, ReplayResult

logger = logging.getLogger(__name__)


def _get_native_engine() -> type:
    """Import the native engine, raising a clear error if unavailable."""
    try:
        from nomai._engine import NomaiEngine  # type: ignore[import-not-found]

        return NomaiEngine
    except ImportError as exc:
        raise RuntimeError(
            "Nomai native engine not available. "
            "Build with: cd crates/nomai-python && maturin develop --release"
        ) from exc


class NomaiEngine:
    """High-level wrapper around the Rust NomaiEngine.

    All manifest results are returned as typed Python dataclasses
    (``TickManifest``, ``EntityEntry``, ``CausalChain``).

    Usage::

        engine = NomaiEngine()
        engine.register_component("position")
        engine.register_component("velocity")
        manifest = engine.tick()
        print(manifest.tick, manifest.commands_processed)
    """

    def __init__(
        self,
        *,
        headless: bool = True,
        fixed_dt: float | None = None,
    ) -> None:
        cls = _get_native_engine()
        # The native engine type is dynamically loaded; Any is unavoidable here.
        self._engine: Any = cls(headless=headless, fixed_dt=fixed_dt)

    # -- Simulation control --------------------------------------------------

    def register_component(self, name: str) -> None:
        """Register a component type by name."""
        self._engine.register_component(name)

    def tick(self) -> TickManifest:
        """Run one tick and return the manifest."""
        raw = self._engine.tick()
        return TickManifest.from_dict(raw)

    def run_ticks(self, n: int) -> list[TickManifest]:
        """Run N ticks and return all manifests."""
        raws = self._engine.run_ticks(n)
        return [TickManifest.from_dict(r) for r in raws]

    def run_until(
        self,
        condition: Callable[[TickManifest], bool],
        max_ticks: int = 10_000,
    ) -> list[TickManifest]:
        """Run ticks until condition returns True or max_ticks reached."""
        manifests: list[TickManifest] = []
        for _ in range(max_ticks):
            m = self.tick()
            manifests.append(m)
            if condition(m):
                break
        return manifests

    # -- Manifest queries ----------------------------------------------------

    def last_manifest(self) -> TickManifest | None:
        """Get the manifest for the most recent tick."""
        raw = self._engine.last_manifest()
        if raw is None:
            return None
        return TickManifest.from_dict(raw)

    def manifest_at_tick(self, tick: int) -> TickManifest | None:
        """Get manifest at a specific tick (within history window)."""
        raw = self._engine.manifest_at_tick(tick)
        if raw is None:
            return None
        return TickManifest.from_dict(raw)

    def manifest_history(self) -> list[TickManifest]:
        """Get all manifests in the history window."""
        raws = self._engine.manifest_history()
        return [TickManifest.from_dict(r) for r in raws]

    def entity_index(self) -> list[EntityEntry]:
        """Get all tracked entities."""
        raws = self._engine.entity_index()
        return [EntityEntry.from_dict(r) for r in raws]

    def get_entity(self, entity_id: int) -> EntityEntry | None:
        """Get a single entity's index entry."""
        raw = self._engine.get_entity(entity_id)
        if raw is None:
            return None
        return EntityEntry.from_dict(raw)

    def trace_causality(
        self,
        entity_id: int,
        component: str,
        tick: int,
    ) -> CausalChain | None:
        """Trace the causal chain for a component change."""
        raw = self._engine.trace_causality(entity_id, component, tick)
        if raw is None:
            return None
        return CausalChain.from_dict(raw)

    # -- World manipulation --------------------------------------------------

    def spawn_entity(
        self,
        entity_type: str,
        role: str,
        components: dict[str, Any] | None = None,
    ) -> None:
        """Queue a semantic entity spawn (applied on next tick)."""
        self._engine.spawn_entity(
            entity_type, role, components or {}
        )

    def despawn_entity(self, entity_id: int) -> None:
        """Queue an entity despawn (applied on next tick)."""
        self._engine.despawn_entity(entity_id)

    def set_component(
        self,
        entity_id: int,
        component: str,
        value: Any,
    ) -> None:
        """Queue a component value change (applied on next tick)."""
        self._engine.set_component(entity_id, component, value)

    # -- Physics -------------------------------------------------------------

    def init_physics(self) -> None:
        """Initialize the physics world with zero gravity.

        Must be called before ``register_physics_entity()``. Also
        auto-registers the ``"position"`` and ``"velocity"`` component
        types if they are not already registered.
        """
        self._engine.init_physics()

    def register_physics_entity(
        self,
        entity_id: int,
        x: float,
        y: float,
        dx: float,
        dy: float,
        body_type: str,
        collider_type: str,
        *,
        collider_radius: float | None = None,
        collider_half_width: float | None = None,
        collider_half_height: float | None = None,
        restitution: float = 0.5,
        is_sensor: bool = False,
    ) -> None:
        """Register a physics entity with position, velocity, and body type.

        The entity must already be alive (spawn + tick first).

        Args:
            entity_id: Raw entity ID from ``entity_index()``.
            x, y: Initial position.
            dx, dy: Initial velocity.
            body_type: ``"dynamic"``, ``"kinematic"``, or ``"static"``.
            collider_type: ``"circle"`` or ``"box"``.
            collider_radius: Required when collider_type is ``"circle"``.
            collider_half_width: Required when collider_type is ``"box"``.
            collider_half_height: Required when collider_type is ``"box"``.
            restitution: Bounciness (default 0.5).
            is_sensor: Whether this is a sensor (default False).
        """
        self._engine.register_physics_entity(
            entity_id, x, y, dx, dy,
            body_type, collider_type,
            collider_radius,
            collider_half_width,
            collider_half_height,
            restitution,
            is_sensor,
        )

    # -- WASM ----------------------------------------------------------------

    def load_gameplay_wasm(self, wasm_bytes: bytes) -> None:
        """Load a WASM gameplay module."""
        self._engine.load_gameplay_wasm(wasm_bytes)

    def hot_swap_gameplay_wasm(self, wasm_bytes: bytes) -> None:
        """Hot-swap the current WASM gameplay module."""
        self._engine.hot_swap_gameplay_wasm(wasm_bytes)

    # -- Snapshot/Restore ----------------------------------------------------

    def capture_snapshot(self) -> EngineSnapshot:
        """Capture a snapshot of the current engine state.

        Returns a typed ``EngineSnapshot`` containing the tick counter,
        fixed_dt, BLAKE3 hash, and the full JSON for round-tripping back
        to ``restore_snapshot()``.
        """
        json_str: str = self._engine.capture_snapshot()
        return EngineSnapshot.from_json(json_str)

    def restore_snapshot(self, snapshot: EngineSnapshot) -> None:
        """Restore engine state from a previously captured snapshot.

        After restore the tick counter and world state match the snapshot.
        The manifest pipeline is reset and the command buffer is cleared.

        **Note:** Systems, physics world, and WASM module are NOT restored.
        Re-attach them after calling this method if needed.
        """
        self._engine.restore_snapshot(snapshot.raw_json)

    def state_hash(self) -> str:
        """Get BLAKE3 hex digest of the current engine state.

        Returns a 64-character lowercase hex string. Two engines with
        identical state will produce the same hash.
        """
        result: str = self._engine.state_hash()
        return result

    # -- Replay --------------------------------------------------------------

    def replay(self, log: ReplayLog) -> ReplayResult:
        """Replay a recorded log and return the result.

        Restores the log's initial snapshot, feeds recorded inputs tick-by-tick,
        and verifies state hashes at each checkpoint. Returns a ``ReplayResult``
        indicating whether the replay was deterministic.
        """
        result_json: str = self._engine.replay_log(log.raw_json)
        return ReplayResult.from_json(result_json)

    def set_input(self, inputs: dict[str, object]) -> None:
        """Set the input frame for simulation.

        Each key-value pair is a named input. Values must be JSON-serializable.
        The input frame persists until overwritten by another ``set_input()``
        call (or snapshot restore) and is included in snapshot/replay state
        hashing. Pass an empty dict to clear the current input.
        """
        self._engine.set_input(inputs)

    # -- Info ----------------------------------------------------------------

    @property
    def tick_count(self) -> int:
        """Current tick count."""
        return self._engine.tick_count()

    @property
    def sim_time(self) -> float:
        """Current simulation time."""
        return self._engine.sim_time()

    @property
    def entity_count(self) -> int:
        """Current entity count in the world."""
        return self._engine.entity_count()
