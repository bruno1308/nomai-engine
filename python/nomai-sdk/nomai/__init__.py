"""Nomai SDK -- Python interface for the Nomai Engine.

Provides intent spec DSL, manifest data types, verification engine,
snapshot/replay types, GDD analysis, and engine control for AI-driven
game development.
"""

__version__ = "0.1.0"

# Re-export key types for convenience.
from nomai.manifest import (
    Aggregates,
    CausalChain,
    CausalStep,
    ComponentChange,
    EntityEntry,
    GameEvent,
    TickManifest,
)
from nomai.gdd import (
    BoundsSpec,
    ClarificationQuestion,
    CompletenessChecker,
    DegenerateStateSpec,
    EntitySpec,
    GameDesignSpec,
    IntentGenerator,
    InteractionSpec,
    InvariantSpec,
    PlayAreaSpec,
)
from nomai.gdd_pipeline import (
    PipelineResult,
    load_spec,
    run_pipeline,
    save_spec,
)
from nomai.replay import (
    EngineSnapshot,
    ReplayDivergence,
    ReplayLog,
    ReplayResult,
)

__all__ = [
    "Aggregates",
    "BoundsSpec",
    "CausalChain",
    "CausalStep",
    "ClarificationQuestion",
    "CompletenessChecker",
    "ComponentChange",
    "DegenerateStateSpec",
    "EngineSnapshot",
    "EntityEntry",
    "EntitySpec",
    "GameDesignSpec",
    "GameEvent",
    "IntentGenerator",
    "InteractionSpec",
    "InvariantSpec",
    "NomaiEngine",
    "PipelineResult",
    "PlayAreaSpec",
    "ReplayDivergence",
    "ReplayLog",
    "ReplayResult",
    "TickManifest",
    "load_spec",
    "run_pipeline",
    "save_spec",
]


def __getattr__(name: str) -> object:
    """Lazy import for NomaiEngine to avoid failing when native module is absent."""
    if name == "NomaiEngine":
        from nomai.engine import NomaiEngine

        return NomaiEngine
    msg = f"module 'nomai' has no attribute {name!r}"
    raise AttributeError(msg)
