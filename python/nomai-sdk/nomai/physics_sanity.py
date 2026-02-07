"""Automatic physics sanity checks derived from entity configuration.

The :class:`PhysicsSanityChecker` inspects collision events and velocity
changes in tick manifests to detect physics responses that violate basic
physical invariants.  These checks run automatically when a physics
registry is provided to :meth:`~nomai.verify.VerificationEngine.verify`.

All result types reuse :class:`~nomai.verify.IntentResult` so they
integrate seamlessly into the verification report.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

from nomai.manifest import TickManifest
from nomai.verify import IntentResult

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class PhysicsEntityInfo:
    """Configuration for a single physics entity.

    Attributes:
        entity_id: The ECS entity ID.
        body_type: One of ``"dynamic"``, ``"static"``, ``"kinematic"``.
        restitution: Coefficient of restitution (0.0 = perfectly inelastic,
            1.0 = perfectly elastic).
        collider_shape: Collider geometry, e.g. ``"circle"``, ``"box"``.
    """
    entity_id: int
    body_type: str
    restitution: float
    collider_shape: str


class PhysicsSanityChecker:
    """Automatic physics sanity checker.

    Given a registry of physics entity configurations, inspects tick
    manifests for violations of basic physical invariants such as
    missing bounce responses after collisions.

    Usage::

        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)
        results = checker.check_collision_responses(manifests)
    """

    def __init__(self, registry: dict[int, PhysicsEntityInfo]) -> None:
        self.registry = registry

    def check_collision_responses(
        self,
        manifests: list[TickManifest],
    ) -> list[IntentResult]:
        """Verify that collisions produce correct physics responses.

        For each collision event involving a dynamic entity with
        ``restitution > 0``, checks that a velocity sign flip occurs
        within 3 ticks.  If no sign flip is found, the check fails
        with a diagnostic message.

        Args:
            manifests: Ordered list of tick manifests to scan.

        Returns:
            A list of :class:`IntentResult` objects, one per failed check.
            Passing checks are not included (no news is good news).
        """
        results: list[IntentResult] = []

        for i, manifest in enumerate(manifests):
            for event in manifest.events:
                if event.event_type != "collision":
                    continue

                # Find dynamic entities involved in this collision
                for eid in event.involved_entities:
                    info = self.registry.get(eid)
                    if info is None or info.body_type != "dynamic":
                        continue
                    if info.restitution <= 0:
                        continue

                    # Check that velocity changed sign within next 3 ticks
                    found_sign_flip = False
                    for j in range(i, min(i + 4, len(manifests))):
                        for change in manifests[j].component_changes:
                            if change.entity_id != eid:
                                continue
                            if change.component_type_name != "velocity":
                                continue
                            # Check for sign flip in any axis
                            old = change.old_value
                            new = change.new_value
                            if isinstance(old, dict) and isinstance(new, dict):
                                for axis in ("dx", "dy"):
                                    ov = old.get(axis, 0)
                                    nv = new.get(axis, 0)
                                    if isinstance(ov, (int, float)) and isinstance(nv, (int, float)):
                                        if ov * nv < 0:
                                            found_sign_flip = True

                    if not found_sign_flip:
                        results.append(IntentResult(
                            intent_name=f"physics_sanity:bounce_response(entity_{eid})",
                            passed=False,
                            trigger_tick=manifest.tick,
                            failure_reason=(
                                f"Dynamic entity {eid} (restitution={info.restitution}) "
                                f"was in a collision at tick {manifest.tick} but no velocity "
                                f"sign flip was detected within 3 ticks"
                            ),
                            suggestion=(
                                "Check that the colliding entity's collider persists long enough "
                                "for rapier's solver to resolve the bounce. Use deferred_unregister "
                                "instead of unregister_entity for entities involved in collisions."
                            ),
                        ))

        return results

    def check_static_immobility(
        self,
        manifests: list[TickManifest],
    ) -> list[IntentResult]:
        """Verify that static bodies do not move.

        Scans all tick manifests for position or velocity changes on
        entities registered as ``"static"``. Any such change is reported
        as a failure (static bodies should never move unless explicitly
        synced by the host).

        Args:
            manifests: Ordered list of tick manifests to scan.

        Returns:
            A list of :class:`IntentResult` objects, one per violation.
        """
        static_ids = {
            eid for eid, info in self.registry.items()
            if info.body_type == "static"
        }
        if not static_ids:
            return []

        results: list[IntentResult] = []
        for manifest in manifests:
            for change in manifest.component_changes:
                if change.entity_id not in static_ids:
                    continue
                if change.component_type_name not in ("position", "velocity"):
                    continue
                # Allow initial sets (old_value is None)
                if change.old_value is None:
                    continue
                if change.old_value != change.new_value:
                    results.append(IntentResult(
                        intent_name=f"physics_sanity:static_immobility(entity_{change.entity_id})",
                        passed=False,
                        trigger_tick=manifest.tick,
                        failure_reason=(
                            f"Static entity {change.entity_id} had {change.component_type_name} "
                            f"change at tick {manifest.tick}: "
                            f"{change.old_value} -> {change.new_value}"
                        ),
                        suggestion=(
                            "Static bodies should not move. Check if an external force "
                            "or sync operation is modifying this entity unexpectedly."
                        ),
                    ))
        return results

    def check_no_tunneling(
        self,
        manifests: list[TickManifest],
        dt: float = 1.0 / 60.0,
    ) -> list[IntentResult]:
        """Verify that dynamic bodies do not tunnel through geometry.

        For each dynamic entity, checks that position changes between
        ticks do not exceed ``velocity * dt * 2``. Larger jumps suggest
        the physics solver failed to detect a collision (tunneling).

        Args:
            manifests: Ordered list of tick manifests to scan.
            dt: Fixed timestep in seconds (default 1/60).

        Returns:
            A list of :class:`IntentResult` objects, one per violation.
        """
        dynamic_ids = {
            eid for eid, info in self.registry.items()
            if info.body_type == "dynamic"
        }
        if not dynamic_ids:
            return []

        results: list[IntentResult] = []
        # Track last known velocity per entity for tunneling check
        last_velocity: dict[int, dict[str, float]] = {}

        for manifest in manifests:
            for change in manifest.component_changes:
                if change.entity_id not in dynamic_ids:
                    continue

                if change.component_type_name == "velocity":
                    if isinstance(change.new_value, dict):
                        last_velocity[change.entity_id] = {
                            k: float(v) for k, v in change.new_value.items()
                            if isinstance(v, (int, float))
                        }

                if change.component_type_name == "position":
                    old_pos = change.old_value
                    new_pos = change.new_value
                    if not isinstance(old_pos, dict) or not isinstance(new_pos, dict):
                        continue
                    if old_pos is None:
                        continue

                    vel = last_velocity.get(change.entity_id, {})
                    for axis, vel_axis in [("x", "dx"), ("y", "dy")]:
                        old_v = old_pos.get(axis)
                        new_v = new_pos.get(axis)
                        if not isinstance(old_v, (int, float)) or not isinstance(new_v, (int, float)):
                            continue
                        speed = abs(vel.get(vel_axis, 0.0))
                        max_displacement = speed * dt * 2.0
                        actual = abs(float(new_v) - float(old_v))
                        if max_displacement > 0 and actual > max_displacement:
                            results.append(IntentResult(
                                intent_name=f"physics_sanity:no_tunneling(entity_{change.entity_id})",
                                passed=False,
                                trigger_tick=manifest.tick,
                                failure_reason=(
                                    f"Dynamic entity {change.entity_id} moved {actual:.1f} on "
                                    f"{axis}-axis at tick {manifest.tick}, but max expected "
                                    f"displacement is {max_displacement:.1f} "
                                    f"(speed={speed:.1f}, dt={dt})"
                                ),
                                suggestion=(
                                    "Large position jumps may indicate tunneling through "
                                    "collision geometry. Consider enabling CCD (continuous "
                                    "collision detection) or reducing the timestep."
                                ),
                            ))
        return results
