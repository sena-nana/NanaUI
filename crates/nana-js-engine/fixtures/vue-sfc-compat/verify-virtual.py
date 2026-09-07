"""Real V8/Runtime/Scene navigation contract; build the fixture and Agent first."""
import json
from pathlib import Path
import agent_driver

root = Path(__file__).resolve().parents[4]
output = root / "target/virtual-navigation"
output.mkdir(parents=True, exist_ok=True)
commands = [
    {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "click", "agent_id": "jump"}, {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "click", "agent_id": "row-500000"}, {"cmd": "pump"}, {"cmd": "a11y"},
    {"cmd": "screenshot", "path": str(output / "jumped.png")},
    {"cmd": "click", "agent_id": "release"}, {"cmd": "pump"}, {"cmd": "a11y"},
]
raw, by_id = agent_driver.run(commands, js=output / "app.js")
(output / "agent.jsonl").write_text(raw, encoding="utf-8")
replies = [by_id[index] for index in range(len(commands))]
assert len(replies) == len(commands)
assert all(reply["ok"] for reply in replies), replies
for index in [2, 5, 9]:
    assert replies[index]["handled"], (index, replies[index])

def rows(reply):
    return {node["agent_id"]: node for node in reply["nodes"] if node.get("agent_id", "").startswith("row-")}

agent_driver.screenshot_painted(replies[8])
initial, jumped, selected, released = (replies[index] for index in [1, 4, 7, 11])
assert list(rows(initial)) == [f"row-{i}" for i in range(5)]
assert len(rows(jumped)) == 6
assert rows(initial)["row-2"]["id"] == rows(jumped)["row-2"]["id"]
assert rows(jumped)["row-2"]["bounds"]["y"] < 0
for index in range(5):
    box = rows(jumped)[f"row-{500000 + index}"]["bounds"]
    assert box["y"] == 64 + 32 * index and box["width"] > 8 and box["height"] > 8
assert next(node for node in selected["nodes"] if node.get("agent_id") == "jump")["label"] == "Selected 500000"
assert len(rows(released)) == 5 and "row-2" not in rows(released)
report = {"logical_items": 1000000, "mounted_before": 5, "mounted_with_editor": 6,
          "mounted_after_release": 5, "a11y_nodes_with_editor": len(jumped["nodes"]),
          "jump_click_and_release": "passed", "refresh_rate_measured": False}
(output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report))
