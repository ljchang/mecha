"""Accounting for the paired, source-selected observation diagnostic."""
import json

def summarize(rows, final, exit_code, model, experiment="observe"):
    reconsider = experiment == "observe-reconsider"
    protocol = "experience-observe-v3" if reconsider else "experience-observe-v2"
    cases = tuple(f'{w}-{c}' for w in ('elm','fern','grove','heather') for c in (1,2,3))
    policies = ('control','observe')
    seen, queries = set(), {}
    def valid_call(call):
        reply = (call or {}).get('reply') or {}
        return bool(call) and not call.get('error') and not reply.get('error') and reply.get('model') == model
    for row in rows:
        key = (row['case'],row['policy'])
        if key in seen or key[0] not in cases or key[1] not in policies:
            raise ValueError('duplicate or unknown observation row')
        seen.add(key)
        query = (row.get('query_call'),row.get('request'))
        if key[0] in queries and queries[key[0]] != query:
            raise ValueError('paired observation query differs')
        queries[key[0]] = query
    arms = {}
    for policy in policies:
        selected = [r for r in rows if r['policy'] == policy]
        valid = [r for r in selected if not r.get('error') and r.get('result') is not None
                 and valid_call(r.get('query_call')) and valid_call(r.get('call'))
                 and ((r.get('review') or {}).get('retry') is None or valid_call(r['review']['retry']))]
        withheld = lambda r: (r.get('assessment') or {}).get('admissible') is False
        count = dict(planned=12,recorded=len(selected),missing=12-len(selected),
                     host_valid=len(valid),failed=len(selected)-len(valid),
                     diagnostic_units=sum((r.get('acquisition') or {}).get('units',0) for r in selected),
                     host_withheld=sum(withheld(r) for r in valid))
        for status in ('observed','unavailable','declined','control'):
            count[status] = sum((r.get('acquisition') or {}).get('status') == status for r in selected)
        for metric in ('optimal','evidence_complete','citations_complete','appropriate_abstention'):
            count[metric] = sum(r['result'].get(metric) is True and
                (metric != 'appropriate_abstention' or not withheld(r)) for r in valid)
        count['grounded_optimal'] = sum(all(r['result'].get(m) is True for m in
            ('optimal','evidence_complete','citations_complete')) for r in valid)
        count['justified_decisions'] = count['grounded_optimal'] + count['appropriate_abstention']
        # Available cases stay in this denominator even if the model declines to query.
        count['available_case_goal_progress'] = sum(r['case'].split('-')[0] in ('elm','grove') and
            all(r['result'].get(m) is True for m in ('optimal','evidence_complete','citations_complete')) for r in valid)
        count['available_case_count'] = 6
        if reconsider:
            count['reconsiderations'] = sum((r.get('review') or {}).get('retry') is not None for r in selected)
            initial = [r for r in selected if (r.get('review') or {}).get('before') is not None and valid_call(r.get('call'))]
            count['initial_assessed'] = len(initial)
            count['initial_host_withheld'] = sum(not r['review']['before']['admissible'] for r in initial)
            choices = [(r, json.loads(r['call']['reply']['text'])['choice']) for r in initial]
            count['initial_grounded_optimal'] = sum(choice != 'abstain' and r['review']['before']['admissible'] and not r['review']['before']['findings'] for r,choice in choices)
            count['initial_appropriate_abstention'] = sum(choice == 'abstain' and bool(r['review']['before']['missing']) for r,choice in choices)
        arms[policy] = count
    calls = [q[0] for q in queries.values() if q[0] is not None]
    calls += [r['call'] for r in rows if r.get('call') is not None]
    calls += [r['review']['retry'] for r in rows if (r.get('review') or {}).get('retry') is not None]
    complete = (exit_code == 0 and len(seen) == 24 and isinstance(final,dict)
        and final.get('protocol') == protocol and final.get('readback_verified') is True
        and final.get('rows') == rows)
    result = dict(protocol='experience-trial-v1',experiment=experiment,complete=complete,exit_code=exit_code,
        arms=arms,planned_rows=24,planned_calls=36,max_calls=60 if reconsider else 36,recorded_calls=len(calls),
        recorded_query_calls=len(queries),elapsed_ms=sum(c.get('millis',0) for c in calls),
        interpretation='One shared query and two checked final decisions per case. Source-selected synthetic evidence, announced corrections, one seed. Diagnostic units are extra work outside the task energy budget. Host withholding is not model abstention. Structured support does not certify free-text explanations or generalization.')
    if reconsider:
        result['interpretation'] += ' One optional reconsideration per decision; initial/final counts share the same draft, with extra inference cost. Failed revisions remain errors.'
    for direction in ('input','output'):
        values = [(c.get('reply') or {}).get(direction+'_tokens') for c in calls]
        known = [v for v in values if type(v) is int and v >= 0]
        result['actual_'+direction+'_tokens'] = sum(known) if known else None
        result['actual_'+direction+'_usage_unknown'] = len(calls)-len(known)
    return result
