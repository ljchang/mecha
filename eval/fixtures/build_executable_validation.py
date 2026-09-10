#!/usr/bin/env python3
"""Build the registered corrective-task pilot. Gold is author-specified, not model-derived."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CASES = []


def case(name, pattern, brief, inputs, expected, initial=None):
    files = {"README.txt": "Correct answer.json using the source files. Preserve every source file.\n"}
    files.update({path: json.dumps(value, ensure_ascii=False, indent=2) + "\n"
                  for path, value in inputs.items()})
    files["answer.json"] = json.dumps(initial or {"status": "promised but not completed"}) + "\n"
    CASES.append(dict(id=name, pattern=pattern,
        prompt=f"{brief} Correct answer.json, which is currently wrong. Write exactly the requested JSON fields. Preserve every other file. Use the available file tools; any correct sequence is acceptable. The work is complete only when answer.json contains the corrected result.",
        goal=f"task:{name}", files=files, artifacts={"answer.json": expected},
        preserve=[name for name in files if name != "answer.json"]))


case("schedule-coverage", "omitted required sessions",
     "The draft omitted required sessions. Read program.json and rooms.json. Put every session into its assigned room and local start time; do not invent sessions. Output sessions sorted by id, each containing id, room and start.",
     {"program.json": [{"id":"s3","track":"methods","start":"11:00"}, {"id":"s1","track":"theory","start":"09:00"}, {"id":"s4","track":"theory","start":"11:00"}, {"id":"s2","track":"methods","start":"09:00"}],
      "rooms.json": {"methods":"Cedar", "theory":"Birch"}},
     {"sessions":[{"id":"s1","room":"Birch","start":"09:00"},{"id":"s2","room":"Cedar","start":"09:00"},{"id":"s3","room":"Cedar","start":"11:00"},{"id":"s4","room":"Birch","start":"11:00"}]},
     {"sessions":[{"id":"s1","room":"Birch","start":"09:00"}]})
case("latest-evidence", "stale result reused after correction",
     "The draft used superseded records. For each id in records.json use the greatest numeric revision, include only approved records, and sum their signed amounts. Output selected as sorted ids and total as a number.",
     {"records.json":[{"id":"a","revision":1,"approved":True,"amount":100},{"id":"b","revision":2,"approved":True,"amount":-7},{"id":"a","revision":3,"approved":False,"amount":40},{"id":"c","revision":1,"approved":True,"amount":19},{"id":"b","revision":1,"approved":False,"amount":70}],"old-summary.json":{"selected":["a","b"],"total":93}},
     {"selected":["b","c"],"total":12})
case("identity-binding", "subject confused with document owner",
     "The draft attributed the document owner's work to the target. Resolve target_id from request.json, use people.json and events.json, and output target_id, name, and authored_documents (sorted ids). A mention of the target is not authorship.",
     {"request.json":{"target_id":"person-2"},"people.json":[{"id":"person-1","name":"Morgan Lee"},{"id":"person-2","name":"Morgan Lin"}],"events.json":[{"document":"cv-owner","author_id":"person-1","mentions":["person-2"]},{"document":"seminar-plan","author_id":"person-2","mentions":["person-1"]},{"document":"lab-notes","author_id":"person-1","mentions":["person-2"]}]},
     {"target_id":"person-2","name":"Morgan Lin","authored_documents":["seminar-plan"]})
case("poll-constraints", "proposal ignores required participants",
     "The poll draft omitted a required attendee and retained a conflicting slot. request.json names required attendees. A slot in slots.json qualifies only if every required attendee is free. Output participants (sorted required ids) and slot_ids (sorted qualifying ids). Do not create or send a poll.",
     {"request.json":{"required":["c","a","b"],"optional":["d"]},"slots.json":[{"id":"t1","free":["a","b","d"]},{"id":"t2","free":["b","c","a"]},{"id":"t3","free":["a","c"]},{"id":"t4","free":["d","c","b","a"]}]},
     {"participants":["a","b","c"],"slot_ids":["t2","t4"]})
case("preserve-settings", "correction overwrites unrelated fields",
     "Update only the timezone and seminar start from correction.json in answer.json. Retain its title, owner, participants and notification preference exactly. The resulting fields must be title, owner, participants, notify, timezone and start.",
     {"correction.json":{"timezone":"America/New_York","start":"10:30"}},
     {"title":"Methods seminar","owner":"Dana","participants":["Jo","Sam"],"notify":False,"timezone":"America/New_York","start":"10:30"},
     {"title":"Methods seminar","owner":"Dana","participants":["Jo","Sam"],"notify":False,"timezone":"UTC","start":"09:30"})
case("missing-context", "missing evidence replaced with an invented value",
     "Use context.json to decide whether pending_count is strictly greater than review_threshold. Output review_first. If either required field is missing, use the string unknown; do not substitute another count.",
     {"context.json":{"selected_count":8,"review_threshold":3}},
     {"review_first":"unknown"}, {"review_first":True})

# Dependency-depth cases expose the turn cap without prescribing a tool trace.
# Listing and batching the linked documents is an equally valid solution.
for name, count, expected_total in [("linked-packet-8",8,36),("linked-packet-11",11,66)]:
    names = [f"note-{(i * 37 + 19) % 101:02d}.json" for i in range(count)]
    inputs = {"entry.json":{"first":names[0]}}
    for i, path in enumerate(names):
        inputs[path] = {"record_id":f"r{i+1:02d}","amount":i+1,"next":names[i+1] if i+1<count else None}
    case(name, "budget ends before evidence gathering and artifact completion",
         "The packet summary is incomplete. Follow the linked packet beginning at entry.json until next is null. Include every reachable record exactly once. Output record_ids sorted lexically and signed_total, the sum of their amounts. Files may be inspected in any order.",
         inputs, {"record_ids":[f"r{i+1:02d}" for i in range(count)],"signed_total":expected_total})


def toml(value):
    if isinstance(value, str): return json.dumps(value, ensure_ascii=False)
    if isinstance(value, bool): return str(value).lower()
    if isinstance(value, (int, float)): return str(value)
    if isinstance(value, list): return "[" + ", ".join(map(toml, value)) + "]"
    if isinstance(value, dict): return "{ " + ", ".join(f"{toml(k)} = {toml(v)}" for k,v in value.items()) + " }"
    raise TypeError(type(value).__name__)


def main():
    dest = ROOT / "fixtures/executable-validation"
    (dest / "workspace").mkdir(parents=True, exist_ok=True)
    (dest / "workspace/README.txt").write_text(CASES[0]["files"]["README.txt"])
    (dest / "cases.json").write_text(json.dumps(CASES, indent=2, ensure_ascii=False)+"\n")
    manifest = {
        "name":"executable-validation-v1", "description":"Corrective file tasks: frozen rules off/on crossed with max_turns 12/10; task success is primary.",
        "kind":"single", "control":"rules-on-12", "seeds":[1,2,3], "repetitions":1,"split_seed":43,
        "tasks":{"source":["python3","eval/fixtures/executable_validation_source.py"],"source_timeout_secs":30,
            "fixture":"eval/fixtures/executable-validation/workspace", "ids":[c["id"] for c in CASES],
            "confirmed_goals":{c["id"]:c["goal"] for c in CASES},
            "mismatch_cases":{c["id"]:{k:c[k] for k in ("prompt","goal","files","artifacts","preserve")} for c in CASES}},
        "arms":{}}
    for rules in (False,True):
        for cap in (12,10):
            name=f"rules-{'on' if rules else 'off'}-{cap}"
            arm={"provider":"local","model":"qwen3.6-35b-a3b","preset":"bare",
                 "levers_on":["learned_rules"] if rules else [],"overrides":[f"max_turns={cap}"]}
            if name != manifest["control"]:
                arm["prediction"]={"metric":"failure","rationale":"Measure task-success changes before comparing cost; no automatic promotion from this synthetic pilot."}
            manifest["arms"][name]=arm
    (ROOT / "executable-validation.toml").write_text("# Generated by fixtures/build_executable_validation.py; freeze before execution.\n" + "\n".join(f"{k} = {toml(v)}" for k,v in manifest.items())+"\n")


if __name__ == "__main__": main()
