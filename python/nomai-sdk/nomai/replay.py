"""Replay and snapshot data types for the Nomai SDK.

Mirrors the Rust engine's ``EngineSnapshot``, ``ReplayLog``, ``ReplayResult``,
and ``ReplayDivergence`` types. All types are JSON-serializable dataclasses.

The engine returns snapshot/replay data as JSON strings. These types parse the
JSON into typed Python objects for safe, ergonomic access.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from typing import Self

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# EngineSnapshot
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class EngineSnapshot:
    """A captured engine state snapshot.

    Wraps the full JSON string so it can be passed back to the engine for
    restore, while also providing typed access to key fields.

    Fields:
        tick_counter: Number of ticks executed when the snapshot was captured.
        fixed_dt: Fixed time step in seconds per tick.
        hash: BLAKE3 hex digest (64 chars) of the serialized engine state.
        raw_json: The full JSON string for round-tripping back to the engine.
    """

    tick_counter: int
    fixed_dt: float
    hash: str
    raw_json: str

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Create from JSON string returned by the native engine.

        Args:
            json_str: The JSON string produced by ``NomaiEngine.capture_snapshot()``.

        Returns:
            A typed ``EngineSnapshot`` instance.

        Raises:
            ValueError: If the JSON is missing required fields or is not valid JSON.
        """
        try:
            data = json.loads(json_str)
        except json.JSONDecodeError as exc:
            raise ValueError(f"invalid snapshot JSON: {exc}") from exc
        try:
            return cls(
                tick_counter=int(data["tick_counter"]),
                fixed_dt=float(data["fixed_dt"]),
                hash=str(data["hash"]),
                raw_json=json_str,
            )
        except KeyError as exc:
            raise ValueError(f"snapshot JSON missing required field: {exc}") from exc

    def to_dict(self) -> dict[str, object]:
        """Serialize to a summary dict (excludes raw_json for readability)."""
        return {
            "tick_counter": self.tick_counter,
            "fixed_dt": self.fixed_dt,
            "hash": self.hash,
        }


# ---------------------------------------------------------------------------
# ReplayDivergence
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ReplayDivergence:
    """Details about a determinism failure during replay.

    Fields:
        tick: The tick at which the divergence was detected.
        expected_hash: The state hash recorded in the replay log at this tick.
        actual_hash: The state hash computed during replay at this tick.
    """

    tick: int
    expected_hash: str
    actual_hash: str

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        return cls(
            tick=int(data["tick"]),  # type: ignore[arg-type]
            expected_hash=str(data["expected_hash"]),
            actual_hash=str(data["actual_hash"]),
        )

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "tick": self.tick,
            "expected_hash": self.expected_hash,
            "actual_hash": self.actual_hash,
        }


# ---------------------------------------------------------------------------
# ReplayResult
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ReplayResult:
    """The outcome of replaying a ``ReplayLog``.

    Fields:
        completed: Whether the replay ran to completion without divergence.
        ticks_replayed: The total number of ticks replayed.
        first_divergence: The first checkpoint mismatch, or ``None`` if all matched.
    """

    completed: bool
    ticks_replayed: int
    first_divergence: ReplayDivergence | None

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        div_raw = data.get("first_divergence")
        divergence: ReplayDivergence | None = None
        if div_raw is not None and isinstance(div_raw, dict):
            divergence = ReplayDivergence.from_dict(div_raw)
        return cls(
            completed=bool(data["completed"]),
            ticks_replayed=int(data["ticks_replayed"]),  # type: ignore[arg-type]
            first_divergence=divergence,
        )

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Parse from a JSON string returned by the native engine.

        Args:
            json_str: The JSON string produced by ``NomaiEngine.replay_log()``.
        """
        return cls.from_dict(json.loads(json_str))

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        result: dict[str, object] = {
            "completed": self.completed,
            "ticks_replayed": self.ticks_replayed,
            "first_divergence": (
                self.first_divergence.to_dict() if self.first_divergence else None
            ),
        }
        return result


# ---------------------------------------------------------------------------
# ReplayLog
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ReplayLog:
    """A recorded replay log (opaque JSON blob for the engine).

    The replay log contains the initial snapshot, input entries, checkpoint
    hashes, and tick count. It is passed to the engine as-is (via
    ``raw_json``) for replay execution.

    Fields:
        total_ticks: Number of ticks recorded in the log.
        raw_json: The full JSON string for round-tripping back to the engine.
    """

    total_ticks: int
    raw_json: str

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Create from a JSON string.

        Args:
            json_str: A JSON string representing a ``ReplayLog``.

        Returns:
            A typed ``ReplayLog`` instance.
        """
        data = json.loads(json_str)
        return cls(
            total_ticks=int(data["total_ticks"]),
            raw_json=json_str,
        )

    def to_dict(self) -> dict[str, object]:
        """Serialize to a summary dict (excludes raw_json for readability)."""
        return {
            "total_ticks": self.total_ticks,
        }
