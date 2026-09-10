"""Freeze small synthetic source worlds; gold remains outside MCP responses."""
import json
from pathlib import Path


def build():
    cases = []
    def case(id, name, question, rows):
        episodes = []
        facts = []
        for i, (source, text, target_fact) in enumerate(rows):
            eid = f"{id}-{i+1}"
            episodes.append(dict(id=eid, kind="episode", source=source,
                                 occurred_at="2026-09-01", text=text))
            if target_fact:
                facts.append(dict(id=eid, statement=target_fact))
        cases.append(dict(id=id, entity=dict(id=f"person:{id}", name=name, aliases=[]),
                          question=question, episodes=episodes, gold=facts))
    case("complement", "Mara Vale", "What should I know about Mara Vale's responsibilities and unresolved commitments?", [
        ("slack", "Mara Vale coordinates the Harbor project. The handoff depends on the Lantern review.", "Mara Vale coordinates Harbor; its handoff depends on the Lantern review."),
        ("bee", "Mara Vale agreed to lead the Lantern review. The review needs the Cedar inventory.", "Mara Vale leads the Lantern review, which needs the Cedar inventory."),
        ("slack", "Mara Vale finished the Cedar inventory and stored it with the operations team.", "Mara Vale finished the Cedar inventory and stored it with operations."),
        ("bee", "Mara Vale cannot authorize the Harbor handoff; the director must approve it.", "Mara Vale cannot authorize the Harbor handoff; the director must approve."),
        ("slack", "Mara Vale will deliver the Juniper checklist after the Lantern review.", "Mara Vale committed to deliver the Juniper checklist after the Lantern review."),
        ("bee", "Mara Vale delegated the Juniper checklist formatting to Oren Pike, retaining responsibility for its contents.", "Mara Vale delegated Juniper formatting to Oren Pike but retains content responsibility."),
    ])
    case("conflict", "Tessa Rowan", "What should I know about Tessa Rowan's current responsibilities and commitments?", [
        ("slack", "The September 1 written roster assigns Tessa Rowan as the sole owner of the Atlas release.", "The September 1 written roster names Tessa Rowan sole Atlas release owner."),
        ("bee", "In the September 1 recorded meeting, Tessa Rowan explicitly declined ownership of the Atlas release.", "Tessa Rowan declined Atlas ownership in the September 1 recording; sources conflict."),
        ("slack", "Tessa Rowan is listed as attending the Beacon review, with a tentative invitation rather than confirmed attendance.", "Tessa Rowan has a tentative Beacon invitation, not confirmed attendance."),
        ("bee", "Tessa Rowan said the Beacon review invitation was tentative and she had not accepted it.", "Tessa Rowan had not accepted the tentative Beacon invitation."),
        ("slack", "Tessa Rowan's Quartz budget request remains awaiting finance approval.", "Tessa Rowan's Quartz budget request awaits finance approval."),
        ("bee", "Tessa Rowan said she would not spend the Quartz budget before finance approval.", "Tessa Rowan committed not to spend Quartz funds before finance approval."),
    ])
    case("temporal", "Ivo Sen", "What should I know about Ivo Sen's current role and commitments, distinguishing earlier plans from later outcomes?", [
        ("slack", "On August 1, Ivo Sen planned to chair the Birch group for the autumn term.", "On August 1 Ivo Sen planned to chair Birch; this was a plan, not current confirmation."),
        ("bee", "On August 20, Ivo Sen withdrew from the Birch chair role; Nell Arco accepted the chair instead.", "Ivo Sen withdrew from Birch chair on August 20; Nell Arco replaced him."),
        ("slack", "Ivo Sen promised on August 5 to deliver the Maple report by August 25.", "Ivo Sen promised on August 5 to deliver Maple by August 25."),
        ("bee", "Ivo Sen confirmed on August 24 that he delivered the Maple report to the archive that morning.", "Ivo Sen delivered Maple to the archive on August 24."),
        ("slack", "Ivo Sen reserved a room for the September 15 Oak workshop; the reservation does not show that the workshop occurred.", "Ivo Sen reserved the September 15 Oak workshop room; occurrence is unconfirmed."),
        ("bee", "Ivo Sen said the Oak workshop still depends on enough registrations and could be cancelled.", "Ivo Sen said Oak depends on registrations and could be cancelled."),
    ])
    case("identity", "Rhea Moss", "What should I know about Rhea Moss, the North lab researcher, and her commitments?", [
        ("slack", "Rhea Moss at North lab maintains the Delta registry. This researcher is person:identity.", "Rhea Moss at North lab maintains Delta."),
        ("bee", "The interviewee Rhea Moss is a South studio designer, person:south-designer, not the North lab researcher. The designer owns the Ember collection.", None),
        ("slack", "North lab researcher Rhea Moss agreed to audit the Delta registry before its next release.", "North lab researcher Rhea Moss agreed to audit Delta before release."),
        ("bee", "South studio designer Rhea Moss promised to unveil the Ember collection in October. This is not the North lab researcher.", None),
        ("slack", "North lab researcher Rhea Moss has not committed to the Pine conference; there is no acceptance from her.", "North lab researcher Rhea Moss has not committed to Pine."),
        ("bee", "The South studio interview contains no evidence about the North lab researcher Rhea Moss or her Pine conference plans.", None),
    ])
    return cases


if __name__ == "__main__":
    Path(__file__).with_name("cases.json").write_text(json.dumps(build(), indent=2) + "\n")
