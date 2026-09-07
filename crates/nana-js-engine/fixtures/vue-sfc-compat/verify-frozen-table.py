"""Real V8/Runtime/Scene navigation contract; build the fixture and Agent first."""
import json
from pathlib import Path
import agent_driver

root = Path(__file__).resolve().parents[4]
output = root / "target/virtual-table"
output.mkdir(parents=True, exist_ok=True)
commands = [
    {"cmd":"pump"}, {"cmd":"a11y"},
    {"cmd":"click", "agent_id":"jump"}, {"cmd":"pump"}, {"cmd":"a11y"},
]
for cell in ["cell-0-0", "cell-0-8000", "cell-500000-0", "cell-500000-8000"]:
    commands.extend([{"cmd":"click", "agent_id":cell}, {"cmd":"pump"}, {"cmd":"a11y"}])
commands.extend([
    {"cmd":"screenshot", "path":str(output / "jumped.png")},
    {"cmd":"click", "agent_id":"release"}, {"cmd":"pump"}, {"cmd":"a11y"},
])
raw, by_id = agent_driver.run(commands, js=output / "app.js")
(output / "agent.jsonl").write_text(raw, encoding="utf-8")
replies = [by_id[index] for index in range(len(commands))]
assert len(replies) == len(commands)
assert all(reply["ok"] for reply in replies), replies

def cells(reply):
    return {n["agent_id"]: n for n in reply["nodes"] if n.get("agent_id", "").startswith("cell-")}
initial, jumped, released = replies[1], replies[4], replies[-1]
assert len(cells(initial)) == 20
assert len(cells(jumped)) == 30
assert cells(initial)["cell-2-2"]["id"] == cells(jumped)["cell-2-2"]["id"]
expected = {"cell-0-0":(0,64), "cell-0-8000":(80,64), "cell-500000-0":(0,96), "cell-500000-8000":(80,96)}
for key, (x,y) in expected.items():
    box = cells(jumped)[key]["bounds"]
    assert (box["x"],box["y"]) == (x,y), (key,box)
for index, label in zip([5,8,11,14], ["0/0","0/8000","500000/0","500000/8000"]):
    assert replies[index]["handled"]
    assert next(n for n in replies[index+2]["nodes"] if n.get("agent_id") == "jump")["label"] == label
assert len(cells(released)) == 20 and "cell-2-2" not in cells(released)
report = {"logical_rows":1000000,"logical_columns":10000,"mounted_before":20,
          "mounted_with_retained_axes":30,"mounted_after_release":20,
          "a11y_nodes_with_retained_axes":len(jumped["nodes"]),
          "frozen_corner_header_column_body_clicks":"passed","refresh_rate_measured":False}
(output / "report.json").write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
print(json.dumps(report))
