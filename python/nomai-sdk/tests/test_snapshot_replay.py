"""Tests for snapshot/restore, state hashing, input frames, and replay.

These tests require the native engine extension (``nomai._engine``), which
must be built with ``maturin develop`` before running::

    cd crates/nomai-python && maturin develop --release

Tests that need the native engine use ``pytest.importorskip`` so they are
cleanly skipped when the extension is not available. Pure-Python dataclass
tests run unconditionally.
"""

from __future__ import annotations

import json
import logging
import re

import pytest

from nomai.replay import (
    EngineSnapshot,
    ReplayDivergence,
    ReplayLog,
    ReplayResult,
)

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Pure-Python dataclass tests (no native engine required)
# ---------------------------------------------------------------------------


class TestEngineSnapshotDataclass:
    """Unit tests for EngineSnapshot parsing and serialization."""

    SAMPLE_JSON = json.dumps({
        "world": {"entities": [], "components": {}, "allocator": {}},
        "tick_counter": 42,
        "fixed_dt": 0.016666666666666666,
        "current_input": {"inputs": {}},
        "hash": "a" * 64,
    })

    def test_from_json_parses_fields(self) -> None:
        """from_json extracts tick_counter, fixed_dt, and hash."""
        snap = EngineSnapshot.from_json(self.SAMPLE_JSON)
        assert snap.tick_counter == 42
        assert abs(snap.fixed_dt - 1.0 / 60.0) < 1e-10
        assert snap.hash == "a" * 64

    def test_from_json_preserves_raw_json(self) -> None:
        """from_json stores the original JSON string for round-trip."""
        snap = EngineSnapshot.from_json(self.SAMPLE_JSON)
        assert snap.raw_json == self.SAMPLE_JSON

    def test_to_dict_excludes_raw_json(self) -> None:
        """to_dict returns a summary without raw_json."""
        snap = EngineSnapshot.from_json(self.SAMPLE_JSON)
        d = snap.to_dict()
        assert "raw_json" not in d
        assert d["tick_counter"] == 42
        assert d["hash"] == "a" * 64

    def test_from_json_invalid_raises(self) -> None:
        """from_json raises ValueError on malformed JSON."""
        with pytest.raises(ValueError, match="invalid snapshot JSON"):
            EngineSnapshot.from_json("not valid json")

    def test_from_json_missing_field_raises(self) -> None:
        """from_json raises ValueError when required fields are missing."""
        with pytest.raises(ValueError, match="missing required field"):
            EngineSnapshot.from_json("{}")


class TestReplayDivergenceDataclass:
    """Unit tests for ReplayDivergence."""

    def test_from_dict_parses_fields(self) -> None:
        div = ReplayDivergence.from_dict({
            "tick": 100,
            "expected_hash": "abc123",
            "actual_hash": "def456",
        })
        assert div.tick == 100
        assert div.expected_hash == "abc123"
        assert div.actual_hash == "def456"

    def test_round_trip(self) -> None:
        div = ReplayDivergence(tick=5, expected_hash="aaa", actual_hash="bbb")
        d = div.to_dict()
        restored = ReplayDivergence.from_dict(d)
        assert restored == div


class TestReplayResultDataclass:
    """Unit tests for ReplayResult."""

    def test_from_dict_completed_no_divergence(self) -> None:
        result = ReplayResult.from_dict({
            "completed": True,
            "ticks_replayed": 100,
            "first_divergence": None,
        })
        assert result.completed is True
        assert result.ticks_replayed == 100
        assert result.first_divergence is None

    def test_from_dict_with_divergence(self) -> None:
        result = ReplayResult.from_dict({
            "completed": False,
            "ticks_replayed": 50,
            "first_divergence": {
                "tick": 50,
                "expected_hash": "exp",
                "actual_hash": "act",
            },
        })
        assert result.completed is False
        assert result.ticks_replayed == 50
        assert result.first_divergence is not None
        assert result.first_divergence.tick == 50

    def test_from_json_string(self) -> None:
        j = json.dumps({
            "completed": True,
            "ticks_replayed": 10,
            "first_divergence": None,
        })
        result = ReplayResult.from_json(j)
        assert result.completed is True
        assert result.ticks_replayed == 10

    def test_to_dict_round_trip(self) -> None:
        div = ReplayDivergence(tick=7, expected_hash="e", actual_hash="a")
        result = ReplayResult(
            completed=False, ticks_replayed=7, first_divergence=div
        )
        d = result.to_dict()
        restored = ReplayResult.from_dict(d)
        assert restored == result


class TestReplayLogDataclass:
    """Unit tests for ReplayLog."""

    SAMPLE_LOG_JSON = json.dumps({
        "initial_snapshot": {
            "world": {"entities": [], "components": {}, "allocator": {}},
            "tick_counter": 0,
            "fixed_dt": 0.016666666666666666,
            "current_input": {"inputs": {}},
            "hash": "b" * 64,
        },
        "gameplay_module_hash": None,
        "total_ticks": 50,
        "entries": [],
    })

    def test_from_json_parses_total_ticks(self) -> None:
        log = ReplayLog.from_json(self.SAMPLE_LOG_JSON)
        assert log.total_ticks == 50

    def test_from_json_preserves_raw_json(self) -> None:
        log = ReplayLog.from_json(self.SAMPLE_LOG_JSON)
        assert log.raw_json == self.SAMPLE_LOG_JSON

    def test_to_dict_summary(self) -> None:
        log = ReplayLog.from_json(self.SAMPLE_LOG_JSON)
        d = log.to_dict()
        assert d["total_ticks"] == 50
        assert "raw_json" not in d


# ---------------------------------------------------------------------------
# Integration tests (require native engine)
# ---------------------------------------------------------------------------


def _skip_if_no_native() -> None:
    """Skip the test if the native engine is not available."""
    pytest.importorskip(
        "nomai._engine",
        reason="Native engine not built -- run: cd crates/nomai-python && maturin develop",
    )


def _make_engine() -> "NomaiEngine":
    """Create a fresh headless engine with common components registered."""
    from nomai.engine import NomaiEngine

    engine = NomaiEngine(headless=True)
    engine.register_component("position")
    engine.register_component("velocity")
    engine.register_component("counter")
    return engine


class TestSnapshotIntegration:
    """Integration tests for snapshot/restore via the Python SDK."""

    @pytest.fixture(autouse=True)
    def _require_native(self) -> None:
        _skip_if_no_native()

    def test_capture_snapshot_returns_typed_object(self) -> None:
        """capture_snapshot returns an EngineSnapshot with correct fields."""
        engine = _make_engine()
        engine.tick()

        snap = engine.capture_snapshot()
        assert isinstance(snap, EngineSnapshot)
        assert snap.tick_counter == 1
        assert abs(snap.fixed_dt - 1.0 / 60.0) < 1e-10
        assert len(snap.hash) == 64
        assert len(snap.raw_json) > 0

    def test_snapshot_restore_resets_tick_count(self) -> None:
        """Restore resets the tick counter to the snapshot's value."""
        engine = _make_engine()

        # Run 5 ticks, capture.
        engine.run_ticks(5)
        assert engine.tick_count == 5
        snap = engine.capture_snapshot()

        # Run 10 more ticks.
        engine.run_ticks(10)
        assert engine.tick_count == 15

        # Restore should rewind to tick 5.
        engine.restore_snapshot(snap)
        assert engine.tick_count == 5

    def test_state_hash_returns_hex_string(self) -> None:
        """state_hash returns a 64-character lowercase hex string."""
        engine = _make_engine()
        h = engine.state_hash()
        assert len(h) == 64
        assert re.fullmatch(r"[0-9a-f]{64}", h), (
            f"state_hash should be 64 hex chars, got: {h!r}"
        )

    def test_state_hash_changes_after_tick(self) -> None:
        """State hash changes after running a tick."""
        engine = _make_engine()
        h1 = engine.state_hash()
        engine.tick()
        h2 = engine.state_hash()
        assert h1 != h2, "state hash should change after a tick"

    def test_snapshot_json_roundtrip(self) -> None:
        """Snapshot JSON can be parsed and restored."""
        engine = _make_engine()
        engine.spawn_entity("unit", "test", {"position": {"x": 1.0, "y": 2.0}})
        engine.tick()

        snap = engine.capture_snapshot()

        # Parse the raw JSON to verify it is valid JSON.
        parsed = json.loads(snap.raw_json)
        assert parsed["tick_counter"] == 1

        # Re-wrap from JSON and restore.
        snap2 = EngineSnapshot.from_json(snap.raw_json)
        assert snap2.tick_counter == snap.tick_counter
        assert snap2.hash == snap.hash

        # Run more ticks then restore from the re-parsed snapshot.
        engine.run_ticks(5)
        assert engine.tick_count == 6
        engine.restore_snapshot(snap2)
        assert engine.tick_count == 1

    def test_restore_then_run_produces_same_hash(self) -> None:
        """Restore + same ticks produces the same state hash (determinism)."""
        engine = _make_engine()
        engine.spawn_entity("unit", "test", {"counter": 0})
        engine.tick()  # tick 0: apply spawn

        snap = engine.capture_snapshot()

        # Run 10 ticks and capture the hash.
        engine.run_ticks(10)
        hash_a = engine.state_hash()

        # Restore and run the same 10 ticks.
        engine.restore_snapshot(snap)
        engine.run_ticks(10)
        hash_b = engine.state_hash()

        assert hash_a == hash_b, (
            "Restoring a snapshot and running the same ticks should produce "
            "an identical state hash (determinism guarantee)"
        )

    def test_snapshot_capture_at_tick_zero(self) -> None:
        """Snapshot at tick 0 (before any ticks) is valid."""
        engine = _make_engine()
        snap = engine.capture_snapshot()
        assert snap.tick_counter == 0

        # Should be restorable.
        engine.tick()
        engine.restore_snapshot(snap)
        assert engine.tick_count == 0


class TestInputFrameIntegration:
    """Integration tests for set_input via the Python SDK."""

    @pytest.fixture(autouse=True)
    def _require_native(self) -> None:
        _skip_if_no_native()

    def test_set_input_changes_state_hash(self) -> None:
        """Setting an input frame changes the state hash."""
        engine = _make_engine()
        h1 = engine.state_hash()
        engine.set_input({"move_x": 1.0, "move_y": -1.0})
        h2 = engine.state_hash()
        assert h1 != h2, "state hash should change after set_input"

    def test_set_input_with_complex_values(self) -> None:
        """set_input handles various JSON-serializable value types."""
        engine = _make_engine()
        # Should not raise.
        engine.set_input({
            "int_val": 42,
            "float_val": 3.14,
            "str_val": "hello",
            "bool_val": True,
            "list_val": [1, 2, 3],
            "dict_val": {"nested": "value"},
            "null_val": None,
        })
        # Verify the engine is still operational.
        engine.tick()
        assert engine.tick_count == 1

    def test_empty_input_is_valid(self) -> None:
        """An empty input dict is valid."""
        engine = _make_engine()
        engine.set_input({})
        engine.tick()
        assert engine.tick_count == 1


class TestReplayIntegration:
    """Integration tests for replay via the Python SDK.

    Since ``ReplayRecorder`` lives in Rust and is not directly exposed,
    we build a minimal ``ReplayLog`` JSON by hand using snapshots and
    state hashes captured from the Python API.
    """

    @pytest.fixture(autouse=True)
    def _require_native(self) -> None:
        _skip_if_no_native()

    def test_replay_deterministic_no_inputs(self) -> None:
        """Replay with no inputs or checkpoints completes successfully."""
        engine = _make_engine()
        engine.spawn_entity("unit", "test", {"counter": 0})
        engine.tick()  # tick 0: apply spawn

        snap = engine.capture_snapshot()

        # Build a minimal replay log: 5 ticks, no inputs, no checkpoints.
        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 5,
            "entries": [],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        result = engine.replay(log)
        assert result.completed is True
        assert result.ticks_replayed == 5
        assert result.first_divergence is None

    def test_replay_with_checkpoint_passes(self) -> None:
        """Replay with a valid checkpoint produces no divergence."""
        engine = _make_engine()
        engine.spawn_entity("unit", "test", {"counter": 0})
        engine.tick()  # tick 0: apply spawn

        snap = engine.capture_snapshot()
        start_tick = snap.tick_counter  # should be 1

        # Record the state hash at the start (before first replayed tick).
        # The checkpoint is checked BEFORE executing the tick, after input is set.
        # With no input at this tick, the hash should match.
        start_hash = engine.state_hash()

        # Build replay log with a single checkpoint at the start tick.
        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 3,
            "entries": [
                {
                    "Checkpoint": {
                        "tick": start_tick,
                        "state_hash": start_hash,
                    }
                }
            ],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        result = engine.replay(log)
        assert result.completed is True
        assert result.first_divergence is None

    def test_replay_with_wrong_checkpoint_detects_divergence(self) -> None:
        """Replay with a deliberately wrong checkpoint detects divergence."""
        engine = _make_engine()
        engine.spawn_entity("unit", "test", {"counter": 0})
        engine.tick()

        snap = engine.capture_snapshot()
        start_tick = snap.tick_counter

        # Use a bogus hash for the checkpoint.
        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 3,
            "entries": [
                {
                    "Checkpoint": {
                        "tick": start_tick,
                        "state_hash": "0" * 64,
                    }
                }
            ],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        result = engine.replay(log)
        assert result.completed is False
        assert result.first_divergence is not None
        assert result.first_divergence.tick == start_tick
        assert result.first_divergence.expected_hash == "0" * 64
        assert result.first_divergence.actual_hash != "0" * 64

    def test_replay_zero_ticks(self) -> None:
        """Replay with zero ticks completes immediately."""
        engine = _make_engine()
        snap = engine.capture_snapshot()

        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 0,
            "entries": [],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        result = engine.replay(log)
        assert result.completed is True
        assert result.ticks_replayed == 0
        assert result.first_divergence is None

    def test_replay_result_is_json_serializable(self) -> None:
        """ReplayResult round-trips through JSON cleanly."""
        engine = _make_engine()
        snap = engine.capture_snapshot()

        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 3,
            "entries": [],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        result = engine.replay(log)
        d = result.to_dict()
        json_str = json.dumps(d)
        roundtrip = json.loads(json_str)
        assert roundtrip["completed"] is True
        assert roundtrip["ticks_replayed"] == 3


class TestErrorPaths:
    """Integration tests for error paths surfaced from FFI."""

    @pytest.fixture(autouse=True)
    def _require_native(self) -> None:
        _skip_if_no_native()

    def test_restore_tampered_snapshot_raises(self) -> None:
        """Restoring a snapshot with a tampered hash raises RuntimeError."""
        engine = _make_engine()
        engine.tick()
        snap = engine.capture_snapshot()

        # Tamper with the hash in the raw JSON.
        data = json.loads(snap.raw_json)
        data["hash"] = "0" * 64
        tampered = EngineSnapshot.from_json(json.dumps(data))

        with pytest.raises(RuntimeError, match="hash mismatch"):
            engine.restore_snapshot(tampered)

    def test_replay_duplicate_input_entries_raises(self) -> None:
        """Replay log with duplicate Input entries at the same tick raises."""
        engine = _make_engine()
        snap = engine.capture_snapshot()

        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 3,
            "entries": [
                {"Input": {"tick": 0, "input": {"inputs": {"a": 1}}}},
                {"Input": {"tick": 0, "input": {"inputs": {"b": 2}}}},
            ],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        with pytest.raises(RuntimeError, match="duplicate Input"):
            engine.replay(log)

    def test_replay_duplicate_checkpoint_entries_raises(self) -> None:
        """Replay log with duplicate Checkpoint entries at same tick raises."""
        engine = _make_engine()
        snap = engine.capture_snapshot()

        log_data = {
            "initial_snapshot": json.loads(snap.raw_json),
            "gameplay_module_hash": None,
            "total_ticks": 3,
            "entries": [
                {"Checkpoint": {"tick": 0, "state_hash": "a" * 64}},
                {"Checkpoint": {"tick": 0, "state_hash": "b" * 64}},
            ],
        }
        log = ReplayLog.from_json(json.dumps(log_data))

        with pytest.raises(RuntimeError, match="duplicate Checkpoint"):
            engine.replay(log)

    def test_replay_malformed_log_json_raises(self) -> None:
        """Replay with invalid JSON raises ValueError."""
        engine = _make_engine()
        # Construct a ReplayLog with garbage raw_json manually.
        # We need to bypass from_json since it would fail to parse.
        log = ReplayLog(total_ticks=1, raw_json="not valid json")

        with pytest.raises(ValueError, match="invalid replay log JSON"):
            engine.replay(log)

    def test_restore_malformed_snapshot_json_raises(self) -> None:
        """Restore with invalid JSON raises ValueError."""
        engine = _make_engine()
        snap = EngineSnapshot(
            tick_counter=0, fixed_dt=1.0 / 60.0, hash="x" * 64,
            raw_json="not valid json",
        )

        with pytest.raises(ValueError, match="invalid snapshot JSON"):
            engine.restore_snapshot(snap)
