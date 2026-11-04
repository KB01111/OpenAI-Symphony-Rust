"""Drives mini_durable_agent.DurableAgent with mini-swe-agent's scripted test model and its real
local environment (no LLM). Used by test/mini-agent.test.ts.

usage: run_durable_agent.py <script.json>   (run inside the repository, drivers on PYTHONPATH)
"""

import json
import sys
from pathlib import Path

import mini_durable_agent
from minisweagent.environments.local import LocalEnvironment
from minisweagent.models.test_models import DeterministicModel, make_output

steps = json.loads(Path(sys.argv[1]).read_text())
model = DeterministicModel(outputs=[make_output(s["text"], [{"command": c} for c in s["commands"]]) for s in steps])
agent = mini_durable_agent.DurableAgent(
    model,
    LocalEnvironment(),
    system_template="You are a test agent.",
    instance_template="Task: {{task}}",
    cost_limit=0,
    output_path=Path(sys.argv[1]).with_suffix(".traj.json"),
)
result = agent.run("fix the bug")
print(json.dumps({"exit_status": result.get("exit_status")}))
