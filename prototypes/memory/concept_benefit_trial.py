"""Independent offline scorer for the bounded synthetic concept pilot."""
import argparse
import hashlib
import itertools
import json
import re
from pathlib import Path
PROTOCOL='memory-concept-benefit-v1'
ROOT=Path(__file__).resolve().parents[2]
def load_fixtures():
    return (json.loads((ROOT/'mecha-core/src/memory/fixtures/concept_benefit_pilot.json').read_text()),
            json.loads((ROOT/'prototypes/memory/fixtures/concept_benefit_pilot_gold.json').read_text()))
def preferred(state, costs):
    feasible = {'repair_shared'}
    if state['shared_ready'] and state['primary_clear']:
        feasible.add('primary')
    if state['shared_ready'] and state['alternate_clear']:
        feasible.add('alternate')
    minimum = min(costs[a] for a in feasible)
    return sorted(a for a in feasible if costs[a] == minimum)

def oracle(case, gold):
    observed = case['observed']
    keys = list(observed)
    worlds = [dict(zip(keys, values)) for values in itertools.product(
        *[(False, True) if observed[k] is None else (observed[k],) for k in keys])]
    optima = [preferred(w, case['costs']) for w in worlds]
    common = set(optima[0]).intersection(*map(set, optima[1:]))
    return dict(expected=sorted(common) or ['insufficient_evidence'],
                completion_count=len(worlds), permitted_optima=optima,
                hidden_preferred=preferred(case['hidden'], case['costs']))
def validate_fixture(fixture, gold):
    def exact(obj, keys):
        if not isinstance(obj, dict) or set(obj) != set(keys):
            raise ValueError('unexpected fixture fields')
    exact(fixture, ('version', 'training', 'counterexample', 'cases'))
    if fixture['version'] != 1 or len(fixture['training']) != 2 or len(fixture['cases']) != 6:
        raise ValueError('invalid fixture cardinality/version')
    ids = set()
    def source(s):
        exact(s, ('id','text'))
        ident(s['id'])
        if not isinstance(s['text'],str) or not 1 <= len(s['text']) <= 8192:
            raise ValueError('invalid original text')
    def ident(value):
        if not isinstance(value,str) or not re.fullmatch(r'[a-z][a-z0-9_]{0,63}',value) or value in ids:
            raise ValueError('invalid/duplicate original ID')
        ids.add(value)
    for s in fixture['training'] + [fixture['counterexample']]: source(s)
    phases=[]
    for c in fixture['cases']:
        exact(c, ('id','phase','question','options','facts'));ident(c['id']);phases.append(c['phase'])
        if not isinstance(c['question'],str) or not 1 <= len(c['question']) <= 2048:
            raise ValueError('invalid question')
        if set(c['options']) != {'primary','alternate','repair_shared','insufficient_evidence'} or len(c['options']) != 4:
            raise ValueError('invalid decision alternatives')
        for s in c['facts']:source(s)
    if sorted(phases) != ['initial']*3+['revised']*3:
        raise ValueError('invalid phases')
    if gold['version'] != 1 or gold['world'] != 'delivery_boolean_v1':raise ValueError('unknown gold')
    if {(c['id'],c['phase']) for c in fixture['cases']} != {(c['id'],c['phase']) for c in gold['cases']} or len(gold['cases']) != 6:
        raise ValueError('gold case mismatch')
    for c in gold['cases']:
        if set(c['costs']) != {'primary','alternate','repair_shared'} or any(type(x) not in (int,float) or x<=0 for x in c['costs'].values()):
            raise ValueError('invalid cost parameters')
        for state in ('observed','hidden'):
            if set(c[state]) != {'shared_ready','primary_clear','alternate_clear'}:raise ValueError('invalid state')
            if any(type(v) is not bool and not (state=='observed' and v is None) for v in c[state].values()):raise ValueError('invalid observation')
        if any(v is not None and c['hidden'][k] != v for k,v in c['observed'].items()):raise ValueError('hidden world contradicts observation')
        allowed={f['id'] for x in fixture['cases'] if x['id']==c['id'] for f in x['facts']}
        if not c['sufficient_premise_sets'] or any(not s or not set(s)<=allowed or len(s)!=len(set(s)) for s in c['sufficient_premise_sets']):
            raise ValueError('invalid frozen premises')
def planned_stages(fixture):
    plans=[dict(id='induction',kind='induction',phase='initial',case_id=None,arm=None)]
    for phase in ('initial','revised'):
        if phase=='revised':plans.append(dict(id='rumination',kind='rumination',phase='revised',case_id=None,arm=None))
        for c in fixture['cases']:
            if c['phase']==phase:
                plans.extend(dict(id=f"{phase}.{c['id']}.{arm}",kind='decision',phase=phase,case_id=c['id'],arm=arm)
                             for arm in ('baseline','concept'))
    return plans
def decode(text):
    def unique(pairs):
        result={}
        for k,v in pairs:
            if k in result: raise ValueError('duplicate JSON field')
            result[k]=v
        return result
    return json.loads(text,object_pairs_hook=unique,parse_constant=lambda x: (_ for _ in ()).throw(ValueError('nonfinite JSON')))

def artifact(root, row, prefix):
    name=row.get(prefix+'_path');expected=row.get(prefix+'_sha256')
    if not isinstance(name,str) or not isinstance(expected,str): raise ValueError('missing artifact')
    path=(root/name).resolve()
    if Path(name).is_absolute() or not path.is_relative_to(root.resolve()): raise ValueError('artifact escapes root')
    data=path.read_bytes()
    if hashlib.sha256(data).hexdigest()!=expected: raise ValueError('artifact hash mismatch')
    return data.decode('utf8')

def decision_contract(answer, row, case, fixture):
    if not isinstance(answer,dict) or set(answer)!= {'request_key','choice','because','reason','question'}:return False
    if not isinstance(row.get('request_key'),str) or answer['request_key']!=row['request_key']:return False
    if answer['choice'] not in case['options']:return False
    reason=answer['reason'];question=answer['question'];because=answer['because']
    if not isinstance(reason,str) or not reason.strip() or len(reason)>512:return False
    if question is not None and (not isinstance(question,str) or len(question)>160):return False
    if answer['choice']=='insufficient_evidence' and (question is None or not question.strip()):return False
    allowed={s['id'] for s in fixture['training']+case['facts']}
    if case['phase']=='revised':allowed.add(fixture['counterexample']['id'])
    if not isinstance(because,list) or len(because)>12 or any(not isinstance(x,str) for x in because):return False
    return len(because)==len(set(because)) and set(because)<=allowed

def wire_json(value):
    return json.dumps(value,separators=(',',':'),ensure_ascii=False)

def digest(value):
    return hashlib.sha256(wire_json(value).encode('utf8')).hexdigest()

def check_packet(packet, row, case, fixture, bindings):
    if not isinstance(packet,dict) or set(packet)!={'protocol','request_key','evidence','question','options','provisional_concept'}:
        raise ValueError('invalid decision packet fields')
    if packet['protocol']!='memory-concept-decision-v1':raise ValueError('unknown decision packet protocol')
    if packet['question']!=case['question'] or packet['options']!=case['options']:
        raise ValueError('current decision differs from frozen fixture')
    if row['arm']=='baseline' and packet['provisional_concept'] is not None:
        raise ValueError('baseline contains a concept')
    if row['arm']=='concept' and not isinstance(packet['provisional_concept'],dict):
        raise ValueError('treatment concept is absent')
    def bound(source):
        reference=bindings[source['id']]
        if not isinstance(reference,dict) or set(reference)!={'object','revision'} or any(not isinstance(v,str) or not v for v in reference.values()):
            raise ValueError('invalid original revision binding')
        return dict(source,reference=reference)
    evidence=dict(episodes=[bound(x) for x in fixture['training']+([fixture['counterexample']] if case['phase']=='revised' else [])],
                  facts=[bound(x) for x in case['facts']])
    originals=evidence['episodes']+evidence['facts']
    if len({(x['reference']['object'],x['reference']['revision']) for x in originals})!=len(originals):raise ValueError('original revision bindings are not independent')
    if packet['evidence']!=evidence:raise ValueError('original evidence differs from frozen fixture/bindings')
    key_free={k:v for k,v in packet.items() if k!='request_key'}
    key=hashlib.sha256((row['id']+':'+wire_json(key_free)).encode('utf8')).hexdigest()
    if packet['request_key']!=key or row['request_key']!=key:raise ValueError('request identity mismatch')
    return digest(evidence)

def cost_summary(rows):
    result=dict(known_input_tokens=0,known_output_tokens=0,unknown_usage_slots=0,
                known_elapsed_ms=0,unknown_elapsed_slots=0)
    for row in rows:
        usage=row.get('usage')
        if isinstance(usage,dict) and all(type(usage.get(k)) is int and usage[k]>=0 for k in ('input_tokens','output_tokens')):
            result['known_input_tokens']+=usage['input_tokens'];result['known_output_tokens']+=usage['output_tokens']
        else:result['unknown_usage_slots']+=1
        elapsed=row.get('elapsed_ms')
        if type(elapsed) is int and elapsed>=0:result['known_elapsed_ms']+=elapsed
        else:result['unknown_elapsed_slots']+=1
    return result

def decision_summary(rows):
    return dict(denominator=len(rows),raw_correct=sum(r['raw_choice_correct'] for r in rows),
                contract_valid=sum(r['contract_valid'] for r in rows),
                accepted_correct=sum(r['accepted_correct'] for r in rows),
                premise_coverage=sum(r['premise_coverage'] for r in rows),
                accepted_grounded_correct=sum(r['accepted_grounded_correct'] for r in rows),
                false_certainty=sum(r['false_certainty'] for r in rows),
                hidden_optimum_match=sum(r['hidden_optimum_match'] for r in rows))

def score_report(report, root, fixture, gold):
    root=Path(root);validate_fixture(fixture,gold)
    if report.get('version')!=1 or report.get('protocol')!=PROTOCOL or report.get('planned_calls')!=14:
        raise ValueError('unsupported report protocol/planned count')
    frozen=(root/'fixture.json').read_bytes()
    if hashlib.sha256(frozen).hexdigest()!=report.get('fixture_sha256') or decode(frozen.decode('utf8'))!=fixture:
        raise ValueError('frozen public fixture mismatch')
    plans=planned_stages(fixture);expected={p['id']:p for p in plans};rows={}
    for row in report['stages']:
        if row['id'] not in expected or row['id'] in rows:raise ValueError('duplicate/unplanned stage')
        if any(row.get(k)!=v for k,v in expected[row['id']].items()):raise ValueError('stage identity mismatch')
        if row.get('status') not in ('withheld','started','accepted','failed'):raise ValueError('unknown stage status')
        rows[row['id']]=row
    stages=[];decisions=[];stage_audit=[];packets={}
    for plan in plans:
        row=rows.get(plan['id'],dict(plan,status='missing'))
        stages.append(row)
        audit=dict(id=plan['id'],status=row['status'])
        for prefix in ('request','response'):
            try:
                artifact(root,row,prefix)
                audit[prefix+'_integrity']=True
            except (OSError,ValueError,TypeError) as exc:
                audit[prefix+'_integrity']=False;audit[prefix+'_error']=str(exc)
        stage_audit.append(audit)
        if plan['kind']!='decision':continue
        case=next(c for c in fixture['cases'] if c['id']==plan['case_id'])
        g=next(c for c in gold['cases'] if c['id']==plan['case_id']);target=oracle(g,gold)
        answer=None;integrity=False;error=None;contract=False
        try:
            request=decode(artifact(root,row,'request'))
            packet=decode(request['messages'][0]['content'][0]['text'])
            packets[row['id']]=check_packet(packet,row,case,fixture,report.get('source_bindings',{}))
            answer=decode(artifact(root,row,'response'));integrity=True
            contract=decision_contract(answer,row,case,fixture)
        except (OSError,ValueError,KeyError,IndexError,TypeError) as exc:error=str(exc)
        choice=answer.get('choice') if isinstance(answer,dict) and integrity else None
        because=answer.get('because') if isinstance(answer,dict) and integrity else None
        raw=choice in target['expected']
        coverage=isinstance(because,list) and all(isinstance(x,str) for x in because) and any(set(s)<=set(because) for s in g['sufficient_premise_sets'])
        accepted=bool(integrity and contract and row.get('status')=='accepted' and row.get('admission')=='accepted')
        decisions.append(dict(plan,status=row['status'],artifact_integrity=integrity,artifact_error=error,
            contract_valid=contract,host_accepted=row.get('admission')=='accepted',raw_choice=choice,
            raw_choice_correct=raw,accepted_correct=bool(accepted and raw),premise_coverage=coverage,
            accepted_grounded_correct=bool(accepted and raw and coverage),
            false_certainty=bool(isinstance(choice,str) and choice in case['options'] and choice!='insufficient_evidence' and target['expected']==['insufficient_evidence']),
            hidden_optimum_match=choice in target['hidden_preferred'],oracle=target,prose_support='unreviewed'))
    pair_rows={}
    for pair in report.get('pairs',[]):
        key=(pair['phase'],pair['case_id'])
        if key in pair_rows or not any(c['phase']==key[0] and c['id']==key[1] for c in fixture['cases']):
            raise ValueError('duplicate/unplanned evidence pair')
        pair_rows[key]=pair
    pairs=[]
    for case in fixture['cases']:
        pair=pair_rows.get((case['phase'],case['id']),{})
        baseline=f"{case['phase']}.{case['id']}.baseline";concept=f"{case['phase']}.{case['id']}.concept"
        valid=bool(pair.get('baseline_stage')==baseline and pair.get('concept_stage')==concept and
                   baseline in packets and concept in packets and packets[baseline]==packets[concept]==pair.get('evidence_sha256'))
        scored={r['arm']:r for r in decisions if r['case_id']==case['id']}
        complete=bool(valid and all(scored[a]['contract_valid'] and scored[a]['artifact_integrity'] and
                      scored[a]['host_accepted'] and scored[a]['status']=='accepted' for a in ('baseline','concept')))
        pairs.append(dict(phase=case['phase'],case_id=case['id'],evidence_equal_and_bound=valid,complete=complete,
                          accepted_correct_delta=(int(scored['concept']['accepted_correct'])-int(scored['baseline']['accepted_correct'])) if complete else None))
    outcome=report.get('revision_outcome')
    if outcome not in ('revised','unchanged','abstained',None):raise ValueError('unknown revision outcome')
    return dict(protocol=PROTOCOL,stages=stages,decisions=decisions,stage_audit=stage_audit,pairs=pairs,pair_denominator=6,
        by_phase_arm={f'{phase}.{arm}':decision_summary([r for r in decisions if r['phase']==phase and r['arm']==arm])
                      for phase in ('initial','revised') for arm in ('baseline','concept')},
        costs_by_group={group:cost_summary([r for r in stages if (r['kind'] if r['kind']!='decision' else r['phase']+'.'+r['arm'])==group])
                        for group in ('induction','rumination','initial.baseline','initial.concept','revised.baseline','revised.concept')},
        metrics=dict(stage_denominator=14,decision_denominator=12,
            raw_correct=sum(r['raw_choice_correct'] for r in decisions),
            accepted_correct=sum(r['accepted_correct'] for r in decisions),
            accepted_grounded_correct=sum(r['accepted_grounded_correct'] for r in decisions),
            false_certainty=sum(r['false_certainty'] for r in decisions)),
        costs=cost_summary(stages),actual_calls=report.get('actual_calls'),
        follow_up=('revised_concept_utility' if outcome=='revised' else 'unchanged_concept_follow_up' if outcome in ('unchanged','abstained') else 'withheld'),
        revision_benefit_identified=False,prose_support='unreviewed',limitations=gold['limitations'])

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report',type=Path);parser.add_argument('--output',type=Path)
    args=parser.parse_args();fixture,gold=load_fixtures()
    result=score_report(decode(args.report.read_text()),args.report.parent,fixture,gold)
    text=json.dumps(result,indent=2,ensure_ascii=False)+'\n'
    if args.output:args.output.write_text(text,encoding='utf8')
    else:print(text,end='')

if __name__=='__main__':main()
