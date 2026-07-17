# Cage — type definitions for IDE autocomplete and static analysis

from __future__ import annotations

from typing import Any, Dict, List, Optional, Protocol, TypedDict, Union


class AgentMessage(TypedDict):
    """A message produced by the agent and delivered via `cage_send`."""

    kind: str
    """Message kind, e.g. ``"init_complete"``, ``"tick_complete"``."""

    payload: Dict[str, Any]
    """Arbitrary JSON payload the agent attached."""


class SandboxStats(TypedDict):
    """Runtime statistics returned by :meth:`cage.Sandbox.stats`."""

    fuel_consumed: int
    """Total fuel (wasm instructions) burned so far."""

    tick_count: int
    """Number of tick cycles executed."""


class HasCageSandbox(Protocol):
    """Protocol for objects that wrap a Cage :class:`cage.Sandbox`."""

    def init(self, message: Dict[str, Any]) -> Optional[AgentMessage]:
        ...

    def tick(self) -> Optional[AgentMessage]:
        ...

    def run(self, init_message: Dict[str, Any]) -> List[AgentMessage]:
        ...

    def stats(self) -> SandboxStats:
        ...


__all__ = [
    "AgentMessage",
    "SandboxStats",
    "HasCageSandbox",
]
