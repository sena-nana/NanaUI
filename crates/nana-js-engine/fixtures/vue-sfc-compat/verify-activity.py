"""Native focus -> million-row navigation -> input -> blur, without explicit pins."""
import json
from pathlib import Path
import agent_driver

root = Path(__file__).resolve().parents[4]
output = root / "target/virtual-activity"
commands = [
    {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "click", "agent_id": "jump"}, {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "type", "text": "edited"}, {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "screenshot", "path": str(output / "jumped.png")},
    {"cmd": "click", "agent_id": "release"}, {"cmd": "pump"}, {"cmd": "a11y"},
]
raw, by_id = agent_driver.run(commands, js=output / "app.js")
(output / "agent.jsonl").write_text(raw, encoding="utf-8")
replies = [by_id[index] for index in range(len(commands))]
assert len(replies) == len(commands) and all(reply["ok"] for reply in replies), replies

def rows(reply):
    return {node["agent_id"]: node for node in reply["nodes"] if node.get("agent_id", "").startswith("row-")}

initial, jumped, edited, released = [rows(replies[index]) for index in [1, 4, 7, 11]]
assert [len(initial), len(jumped), len(released)] == [5, 6, 5]
assert initial["row-2"]["id"] == jumped["row-2"]["id"] == edited["row-2"]["id"]
assert jumped["row-2"]["focused"] and jumped["row-2"]["bounds"]["y"] < 0
assert "edited" in edited["row-2"]["value"], edited["row-2"]
assert "row-2" not in released
report = {"logical_items": 1000000, "mounted_before": 5, "mounted_while_focused": 6,
          "mounted_after_blur": 5, "offscreen_input_preserved": True,
          "explicit_retained_keys": False, "refresh_rate_measured": False}
(output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report))
