"""mini-swe-agent baseline installed under a uv-managed Python 3.11.

Harbor's stock installer runs `uv tool install mini-swe-agent` against the task
image's system Python. Observed 2026-09-09: the current litellm release imports
`typing.NotRequired` (3.11+), so every Terminal-Bench image on Python 3.10 crashed
at import (ImportError) before the agent ran. Passing `--python 3.11` makes uv
fetch its own interpreter; nothing else about the baseline changes.

    harbor run -a mini_swe_agent_py311:MiniSweAgentPy311 -m zai/glm-5.3-flash ...
"""

from __future__ import annotations

from harbor.agents.installed.mini_swe_agent import MiniSweAgent
from harbor.environments.base import BaseEnvironment

MINI_SWE_PYTHON = "3.11"


class MiniSweAgentPy311(MiniSweAgent):
    @staticmethod
    def name() -> str:
        return "mini-swe-agent-py311"

    async def install(self, environment: BaseEnvironment) -> None:
        await self.ensure_system_dependencies(
            environment,
            ("curl", "bash", "build_tools", "git", "python3", "python_pip"),
        )
        version_spec = f"=={self._version}" if self._version else ""
        await self.exec_as_agent(
            environment,
            command=(
                "set -euo pipefail; "
                "if ! command -v uv >/dev/null 2>&1; then"
                "  curl -LsSf https://astral.sh/uv/install.sh | sh;"
                " fi && "
                'if ! grep -q \'export PATH="$HOME/.local/bin:$PATH"\' "$HOME/.bashrc" 2>/dev/null; then'
                '  echo \'export PATH="$HOME/.local/bin:$PATH"\' >> "$HOME/.bashrc";'
                " fi && "
                'if [ -f "$HOME/.local/bin/env" ]; then source "$HOME/.local/bin/env"; fi && '
                'export PATH="$HOME/.local/bin:$PATH" && '
                f"uv tool install --python {MINI_SWE_PYTHON} mini-swe-agent{version_spec} "
                "--with litellm --with orjson --with fastapi && "
                "mini-swe-agent --help"
            ),
        )
