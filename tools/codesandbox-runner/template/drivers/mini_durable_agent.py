"""mini-swe-agent with durability: a checkpoint after every step and a commit + push after every step.

Used as ``mini --agent-class mini_durable_agent.DurableAgent`` (the drivers directory is on
``PYTHONPATH``). Based on mini-swe-agent 2.x's ``DefaultAgent``:

* After each batch of actions the agent's messages are written to
  ``$SYMPHONY_STATE_DIR/mini-checkpoint.json`` (atomically), and ``autocommit.sh`` commits and
  pushes whatever the actions changed, so every edit reaches the task branch right away.
* When the sandbox is resumed after an interruption, ``run`` reads that checkpoint and tells the
  model what it already did. The work itself is in the repository (it was pushed), so the model
  continues from the real state instead of starting over.
* Structured events (``@@SYMPHONY {...}`` lines) report the model's text, every command and the
  token usage to Symphony.
"""

import json
import os
import subprocess  # nosec B404 - only used to run the commit/push script below
from pathlib import Path

from minisweagent.agents.default import DefaultAgent

PREFIX = "@@SYMPHONY "
STATE_DIR = Path(os.environ.get("SYMPHONY_STATE_DIR", ".symphony-state"))
CHECKPOINT = STATE_DIR / "mini-checkpoint.json"
AUTOCOMMIT = Path(__file__).with_name("autocommit.sh")
# Overridable for hosts where `bash` on PATH is not the one that should run the script (tests on Windows).
BASH = os.environ.get("SYMPHONY_BASH", "bash")
MAX_TEXT = 4000
MAX_DIGEST_ACTIONS = 15


def emit(event: dict) -> None:
    print(PREFIX + json.dumps(event, ensure_ascii=False), flush=True)


def _clip(text: str, limit: int = MAX_TEXT) -> str:
    return text if len(text) <= limit else text[:limit] + "…"


def _text_of(content) -> str:
    """Message content is a string, or a list of parts with a ``text`` field."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(part.get("text", "") for part in content if isinstance(part, dict))
    return ""


def resume_note(messages: list[dict]) -> str:
    """What an interrupted earlier run did, from its checkpoint (commands only, newest last)."""
    commands: list[str] = []
    for message in messages:
        for action in message.get("extra", {}).get("actions", []) or []:
            command = action.get("command") if isinstance(action, dict) else None
            if command:
                commands.append(_clip(" ".join(str(command).split()), 200))
    if not commands:
        return ""
    recent = commands[-MAX_DIGEST_ACTIONS:]
    lines = "\n".join(f"- {command}" for command in recent)
    return (
        "## Resuming an interrupted run\n\n"
        "An earlier run of this task was interrupted and this sandbox was restored. Everything it "
        "committed is in the repository (check `git log` and `git status`). Its most recent "
        f"commands were:\n{lines}\n\n"
        "Continue from the current state of the repository; do not start over."
    )


class DurableAgent(DefaultAgent):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._input_tokens = 0
        self._output_tokens = 0
        self._previous: list[dict] = self._load_checkpoint()

    # -- durability -----------------------------------------------------------------------
    def _load_checkpoint(self) -> list[dict]:
        try:
            return json.loads(CHECKPOINT.read_text()).get("messages", [])
        except (OSError, ValueError):
            return []

    def _save_checkpoint(self) -> None:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        temporary = CHECKPOINT.with_suffix(".tmp")
        temporary.write_text(json.dumps({"messages": self._previous + self.messages}))
        os.replace(temporary, CHECKPOINT)

    def _commit_and_push(self) -> None:
        issue = os.environ.get("SYMPHONY_ISSUE", "task")
        # A fixed argument list, no shell: only the commit message varies, and git receives it as data.
        result = subprocess.run(  # nosec B603
            [BASH, AUTOCOMMIT.as_posix(), f"{issue}: step {self.n_calls}"],
            check=False,
            text=True,
            stdout=None,  # the script prints the `pushed` event itself
            stderr=subprocess.PIPE,
        )
        if result.returncode != 0:
            emit({"type": "output", "text": _clip("commit/push failed: " + (result.stderr or "").strip())})

    # -- agent loop -------------------------------------------------------------------------
    def run(self, task: str = "", **kwargs) -> dict:
        note = resume_note(self._previous)
        return super().run(f"{task}\n\n{note}" if note else task, **kwargs)

    def execute_actions(self, message: dict) -> list[dict]:
        observations: list[dict] = []
        try:
            observations = super().execute_actions(message)
            return observations
        finally:
            # Also when an action ends the run (the submit command raises inside `super()`): edits made
            # by the other actions of the same step still have to reach the branch.
            self._report(message, observations)
            self._save_checkpoint()
            self._commit_and_push()

    def _report(self, message: dict, observations: list[dict]) -> None:
        extra = message.get("extra", {})
        usage = (extra.get("response") or {}).get("usage") or {}
        if isinstance(usage, dict):
            self._input_tokens += int(usage.get("prompt_tokens") or usage.get("input_tokens") or 0)
            self._output_tokens += int(usage.get("completion_tokens") or usage.get("output_tokens") or 0)
        text = _text_of(message.get("content")).strip()
        if text:
            emit({"type": "output", "text": _clip(text)})
        actions = extra.get("actions", []) or []
        for index, action in enumerate(actions):
            observation = _text_of(observations[index].get("content")) if index < len(observations) else ""
            emit(
                {
                    "type": "tool",
                    "callId": f"{os.environ.get('SYMPHONY_TURN_ID', 'turn')}-{self.n_calls}-{index}",
                    "name": "bash",
                    "kind": "command",
                    "status": "succeeded",
                    "input": action if isinstance(action, dict) else {"command": str(action)},
                    "output": _clip(observation),
                }
            )
        emit({"type": "usage", "inputTokens": self._input_tokens, "outputTokens": self._output_tokens})
