# Cage — Sandboxed AI agent runtime (Python API)
#
# >>> from cage import Sandbox, Policy
# >>> policy = Policy(fuel=500_000).env("API_KEY", "sk-...").allow_url("https://api.openai.com")
# >>> with Sandbox("agent.wasm", policy) as box:
# ...     msg = box.init({"task": "summarize"})
# ...     while msg:
# ...         print(msg)
# ...         msg = box.tick()
# >>> box.stats()
# {'fuel_consumed': 33480, 'tick_count': 1}

from __future__ import annotations

from typing import Any, Dict, List, Optional

from ._native import Orchestrator, RoundSummary, Policy as _Policy, Sandbox as _Sandbox


__all__ = ["Orchestrator", "Policy", "RoundSummary", "Sandbox"]


class Policy:
    """Sandbox policy: fuel limit, injected environment variables, URL whitelist.

    Usage:
        policy = Policy(fuel=500_000) \\
            .env("API_KEY", "sk-...") \\
            .allow_url("https://api.openai.com/v1")
    """

    def __init__(self, fuel: int = 500_000) -> None:
        self._inner = _Policy(fuel)

    def env(self, key: str, value: str) -> Policy:
        """Inject an environment variable (secret) into the sandbox."""
        self._inner.set_env(key, value)
        return self

    def allow_url(self, prefix: str) -> Policy:
        """Allow HTTP requests to URLs starting with *prefix*."""
        self._inner.allow_url(prefix)
        return self

    def __repr__(self) -> str:
        return repr(self._inner)


class Sandbox:
    """Sandboxed WASM agent runtime.

    Usage:
        with Sandbox("agent.wasm", policy) as box:
            result = box.run({"prompt": "Hello"})
    """

    def __init__(self, wasm_path: str, policy: Policy) -> None:
        self._inner = _Sandbox(wasm_path, policy._inner)

    def init(self, message: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        """Deliver the init message to the agent and return its response."""
        result = self._inner.init(message)
        return dict(result) if result is not None else None

    def tick(self) -> Optional[Dict[str, Any]]:
        """Run one tick cycle.  Returns the agent's message or *None*."""
        result = self._inner.tick()
        return dict(result) if result is not None else None

    def run(
        self,
        init_message: Dict[str, Any],
        max_ticks: int = 100,
    ) -> List[Dict[str, Any]]:
        """Convenience: init + tick until completion or *max_ticks* ticks.

        Returns a list of all AgentMessage dicts produced during the
        execution lifecycle.
        """
        outputs = self._inner.run(init_message, max_ticks)
        return [dict(o) for o in outputs]

    def stats(self) -> Dict[str, Any]:
        """Return runtime statistics (fuel consumed, tick count, …)."""
        return dict(self._inner.stats())

    def __enter__(self) -> Sandbox:
        self._inner.__enter__()
        return self

    def __exit__(self, *exc: Any) -> None:
        self._inner.__exit__(*exc)
