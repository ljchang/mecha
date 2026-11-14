#!/usr/bin/env python3
"""Actual Rust CLI + lifetime adapter smoke against an authored HTTP provider.

Requires an explicit memory-enabled binary. This is tool/protocol integration
qualification, never model capability evidence. No real inference is performed.
"""
import argparse
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
from urllib.parse import parse_qs,urlsplit

from ordinary_lifetime import freeze,read_json,require,run,score

MODEL='ordinary-lifetime-authored-fixture'


def reply(payload,replace_inputs=False):
    tools={t['function']['name'] for t in payload.get('tools',[])}
    if not tools:
        # An intentionally malformed concept exercises the failed-learning path;
        # Task 1's own integration tests qualify successful native learning.
        return {'content':'authored malformed concept'},'stop'
    messages=payload['messages']
    calls={c['id']:c['function'] for m in messages for c in m.get('tool_calls',[])}
    results=[m for m in messages if m.get('role')=='tool']
    observation=any('Copy its exact bytes unchanged to observation-receipt.json' in str(m.get('content','')) for m in messages if m.get('role')=='user')
    call=None
    if not results:
        call=('fs_write',{'path':'observation.json','content':'SUBSTITUTED_OBSERVATION'}) if observation and replace_inputs else ('fs_read',{'path':'observation.json' if observation else 'task.json'})
    else:
        last=results[-1]
        previous=calls[last['tool_call_id']]
        arguments=json.loads(previous['arguments'])
        if previous['name']=='fs_read' and arguments['path']=='observation.json':
            call=('fs_write',{'path':'observation-receipt.json','content':last['content']})
        elif previous['name']=='fs_read' and arguments['path']=='task.json' and 'goal_context' in tools:
            call=('goal_context',{'memory':{'query':'report-release-v1'}})
        elif previous['name']=='goal_context' or (previous['name']=='fs_read' and arguments['path']=='task.json'):
            call=('fs_write',{'path':'decision.json','content':json.dumps({'status':'abstained','route':None,'reason':'authored protocol smoke; no capability claim','citations':[]})})
        elif previous['name']=='fs_write':
            call=('fs_read',{'path':arguments['path']})
    if call:
        name,arguments=call
        return {'content':None,'tool_calls':[{'id':'authored-'+str(len(results)), 'type':'function','function':{'name':name,'arguments':json.dumps(arguments)}}]},'tool_calls'
    return {'content':'Authored fixture finished its tool sequence.'},'stop'


class Handler(BaseHTTPRequestHandler):
    def log_message(self,*_):
        pass

    def send(self,value,content_type='application/json'):
        raw=value if isinstance(value,bytes) else json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Type',content_type)
        self.send_header('Content-Length',str(len(raw)))
        self.end_headers();self.wfile.write(raw)

    def do_GET(self):
        parsed=urlsplit(self.path);query=parse_qs(parsed.query)
        if parsed.path=='/models':
            self.send({'data':[{'id':MODEL,'status':{'value':'loaded'}}]})
        elif parsed.path=='/props':
            if 'model' not in query:
                self.send({'role':'router','model_alias':'llama-server','default_generation_settings':{'n_ctx':0}})
            elif query['model']==[MODEL]:
                self.send({'model_alias':MODEL,'default_generation_settings':{'n_ctx':32768},'total_slots':1})
            else:
                self.send_error(400)
        else:
            self.send_error(404)

    def do_POST(self):
        require(self.path=='/v1/chat/completions?autoload=false','smoke request omitted noautoload')
        payload=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        require(payload['model']==MODEL,'smoke alias mismatch')
        message,finish=reply(payload,getattr(self.server,'replace_inputs',False))
        envelope={'id':'authored-completion','model':MODEL,'choices':[{'index':0,'message':{'role':'assistant',**message},'finish_reason':finish}],'usage':{'prompt_tokens':30,'completion_tokens':5,'total_tokens':35}}
        if payload.get('stream'):
            delta=dict(message)
            if delta.get('tool_calls'):
                delta['tool_calls'][0]['index']=0
            envelope['choices']=[{'index':0,'delta':delta,'finish_reason':finish}]
            self.send(b'data: '+json.dumps(envelope).encode()+b'\n\ndata: [DONE]\n\n','text/event-stream')
        else:
            self.send(envelope)


def qualify(binary,output,replace_inputs=True):
    binary=Path(binary).resolve(strict=True)
    output=Path(output).resolve(strict=False)
    require(not output.exists(),'smoke output must be new')
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    server.replace_inputs=replace_inputs
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        # Receipt is host-only and synthetic, just like the serving fixture.
        with tempfile.TemporaryDirectory(prefix='ordinary-lifetime-resident-') as temporary:
            receipt=Path(temporary)/'resident.json'
            receipt.write_text(json.dumps({'models':{'data':[{'id':MODEL,'status':{'value':'loaded'}}]},'props':{'model_alias':MODEL,'default_generation_settings':{'n_ctx':32768}}}))
            freeze(output,binary,f'http://127.0.0.1:{server.server_address[1]}',MODEL,[17],receipt)
        run(output,max_slots=9)
        manifest=read_json(output/'manifest.json')
        from ordinary_lifetime import events
        terminal={e['slot']:e for e in events(output,manifest) if e['state']=='terminal'}
        observations=[terminal[f'17-frozen-{eid}'] for eid in ('O1','O2')]
        for event in observations:
            require(event['execution']=='completed',f"ordinary observation failed: {event.get('error',event.get('returncode'))}")
            require(event['capture_status']=='retained' and event.get('observation_source'),'ordinary source inventory/capture unavailable')
            require(event['treatment_delivery']['observation_no_recall'],'observation invoked recall')
            if replace_inputs:
                records=[json.loads(line) for line in (output/'slots'/event['id']/'session.jsonl').read_text().splitlines()]
                blocks=[block for row in records if row.get('record')=='message' for block in row.get('content',[])]
                attempted=[b['id'] for b in blocks if b.get('type')=='tool_use' and b.get('name')=='fs_write' and b.get('input',{}).get('path')=='observation.json']
                require(len(attempted)==1 and any(b.get('type')=='tool_result' and b.get('tool_use_id')==attempted[0] and b.get('is_error') is True for b in blocks),'native input replacement was not attempted and refused')
                require(event.get('input_unchanged') is True and 'SUBSTITUTED_OBSERVATION' not in event['observation_source']['text'],'replacement earned observation provenance')
        decision=terminal['17-frozen-D1']
        induction=terminal['17-adaptive-induce']
        require(len(induction['calls'])==1 and induction['calls'][0].get('acknowledged') and induction['calls'][0].get('forwarded'),'native induction did not reach the authored inference POST')
        require(induction.get('learning_status')=='failed','authored malformed concept did not exercise failed learning')
        induction_wire=output/'slots'/'17-adaptive-induce'/'wire'
        require('authored malformed concept' in (induction_wire/'001.response').read_text(),'induction malformed reply control missing')
        require(read_json(induction_wire/'discovery-001.response').get('role')=='router','induction did not discover the router envelope')
        require(decision['execution']=='completed' and decision['capture_status']=='retained','ordinary decision/capture failed')
        require(decision['treatment_delivery']['memory_calls']==1,'decision did not call native recall once')
        require(decision['treatment_delivery']['records_delivered']>0 and decision['delivered'],'native evidence was not extracted from actual forwarded wire')
        require(any(d.get('episode') in ('O1','O2') for d in decision['delivered']),'delivered original source attribution missing')
        require(decision['protocol_status']=='qualified',f"decision protocol not qualified: {decision.get('protocol_violations')}")
        card=score(output)
        return {'protocol':'ordinary-lifetime-cli-smoke-v1','status':'passed','binary':str(binary),'output':str(output),'executed_slots':9,'ordinary_observations_checked':2,'native_recall_decisions_checked':1,'learning_posts_checked':1,'input_replacement_refusals_checked':2 if replace_inputs else 0,'model_capability':'not_evaluated','planned_decision_denominator':sum(a['decision_denominator'] for a in card['arms'].values())}
    finally:
        server.shutdown();server.server_close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(qualify(args.binary,args.output),sort_keys=True))


if __name__=='__main__':
    main()
