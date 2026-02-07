"""Nomai SDK -- Python interface for the Nomai Engine.

Provides intent spec DSL, manifest data types, verification engine,
and engine control for AI-driven game development.
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

__all__ = [
    "Aggregates",
    "CausalChain",
    "CausalStep",
    "ComponentChange",
    "EntityEntry",
    "GameEvent",
    "NomaiEngine",
    "TickManifest",
]


def __getattr__(name: str) -> object:
    """Lazy import for NomaiEngine to avoid failing when native module is absent."""
    if name == "NomaiEngine":
        from nomai.engine import NomaiEngine

        return NomaiEngine
    msg = f"module 'nomai' has no attribute {name!r}"
    raise AttributeError(msg)
