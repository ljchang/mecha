"""Bounded direct-loopback recorder for ordinary lifetime inference.

Raw client requests, exact forwarded requests, responses (including partial
bodies) and failures survive client disconnects. No redirects or env proxies.
"""
from contextlib import contextmanager
from http.client import HTTPConnection, HTTPException
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hmac
import socket
import threading
import time
from urllib.parse import parse_qsl, urlencode, urlsplit

from ordinary_lifetime import LifetimeError, decode, encode, endpoint_parts, require, write_new

TransportError = LifetimeError
REQUEST_LIMIT = 2 * 1024 * 1024
RESPONSE_LIMIT = 4 * 1024 * 1024


def shutdown(sock):
    try:
        sock.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass


def request_direct(address, method, path, deadline, capture, body=None, phase=None, audit=None):
    remaining = deadline-time.monotonic()
    require(remaining>0, 'transport deadline expired')
    connection=HTTPConnection(*address,timeout=remaining)
    timer=None
    wire=None
    try:
        connection.connect()
        wire=connection.sock
        if phase:
            with phase.condition:
                require(not phase.closed,'phase closed before forwarding')
                phase.sockets.add(wire)
        timer=threading.Timer(max(.001,deadline-time.monotonic()),shutdown,args=(wire,))
        timer.daemon=True
        timer.start()
        headers={'Accept-Encoding':'identity','Connection':'close'}
        if body is not None:
            headers['Content-Type']='application/json'
        if audit is not None:
            audit['dispatch_uncertain']=True
            audit['transmission_started']=True
        connection.request(method,path,body=body,headers=headers)
        if audit is not None:
            audit.update(forwarded=True,dispatch_uncertain=False)
        with connection.getresponse() as response, capture.open('xb') as output:
            status=response.status
            if audit is not None:
                audit['response_started']=True
            content_type=response.getheader('Content-Type','application/octet-stream')
            require(response.getheader('Content-Encoding','identity')=='identity','encoded upstream response refused')
            total=0
            try:
                while True:
                    require(time.monotonic()<deadline,'transport deadline expired')
                    chunk=response.read1(min(65536,RESPONSE_LIMIT+1-total))
                    if not chunk:
                        break
                    output.write(chunk[:RESPONSE_LIMIT-total])
                    total+=len(chunk)
                    require(total<=RESPONSE_LIMIT,'upstream response too large')
                require(response.length in (None,0) and total>0,'incomplete or empty upstream response')
            finally:
                output.flush()
                import os
                os.fsync(output.fileno())
        require(time.monotonic()<deadline,'transport deadline expired')
        if audit is not None:
            audit['response_complete']=True
        return status,content_type
    finally:
        if timer:
            timer.cancel()
        connection.close()
        if phase and wire:
            with phase.condition:
                phase.sockets.discard(wire)


def response_metadata(data, content_type):
    """Parse wire usage without inventing a counter for absent usage."""
    if 'text/event-stream' in content_type:
        chunks=[]
        done=False
        for line in data.decode('utf-8').splitlines():
            if not line.startswith('data:'):
                continue
            value=line[5:].strip()
            if value=='[DONE]':
                done=True
            elif value:
                chunks.append(decode(value))
        require(done,'incomplete SSE response')
    else:
        chunks=[decode(data)]
    aliases={c['model'] for c in chunks if isinstance(c,dict) and isinstance(c.get('model'),str)}
    usage=None
    for chunk in chunks:
        raw=chunk.get('usage') if isinstance(chunk,dict) else None
        if isinstance(raw,dict):
            usage={key:raw.get(source) if type(raw.get(source)) is int and raw[source]>=0 else None for key,source in [('input_tokens','prompt_tokens'),('output_tokens','completion_tokens')]}
    return aliases,usage


class Phase:
    def __init__(self,path,seconds,requests,tokens,learning):
        self.path=path
        self.deadline=time.monotonic()+seconds
        self.max_requests=requests
        self.tokens=tokens
        self.learning=learning
        self.condition=threading.Condition()
        self.active=0
        self.accepted=0
        self.forwarding=False
        self.closed=False
        self.sockets=set()
        self.calls=[]
        self.discovery=[]
        self.output_tokens=0
        self.reserved_tokens=0
        self.unresolved=False
        self.attempted=0


class BoundServer(ThreadingHTTPServer):
    """Bind an accepted socket before a request thread can parse or dispatch it."""
    daemon_threads=True

    def __init__(self,*args,**kwargs):
        self.phases={}
        super().__init__(*args,**kwargs)

    def get_request(self):
        with self.recorder.phase_lock:
            request,address=super().get_request()
            phase=self.recorder.current
            if phase is not None and phase.closed:
                phase=None
            self.phases[request]=phase
            if phase is not None:
                with phase.condition:
                    phase.accepted+=1
                    phase.sockets.add(request)
            return request,address

    def release(self,request):
        with self.recorder.phase_lock:
            phase=self.phases.pop(request,None)
            if phase is not None:
                with phase.condition:
                    phase.sockets.discard(request)
                    phase.accepted-=1
                    phase.condition.notify_all()

    def process_request(self,request,client_address):
        try:
            super().process_request(request,client_address)
        except Exception:
            self.release(request)
            raise

    def process_request_thread(self,request,client_address):
        try:
            super().process_request_thread(request,client_address)
        finally:
            self.release(request)


class RecordingProxy:
    def __init__(self,endpoint,model,token):
        self.address=endpoint_parts(endpoint)
        self.model=model
        self.token=token
        self.current=None
        self.phase_lock=threading.RLock()
        self.server=BoundServer(('127.0.0.1',0),Handler)
        self.server.daemon_threads=True
        self.server.recorder=self
        self.port=self.server.server_address[1]
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self,*_):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    @contextmanager
    def phase(self,path,seconds,requests,tokens,learning=False):
        with self.phase_lock:
            require(self.current is None,'previous transport is not quiescent')
            phase=Phase(path,seconds,requests,tokens,learning)
            self.current=phase
        try:
            yield phase
        finally:
            with self.phase_lock:
                with phase.condition:
                    phase.closed=True
                    for sock in tuple(phase.sockets):
                        shutdown(sock)
                    phase.condition.notify_all()
            with phase.condition:
                quiet=phase.condition.wait_for(lambda:phase.active==0 and phase.accepted==0,timeout=4)
                unresolved=any((c.get('forwarded') or c.get('transmission_started') or c.get('dispatch_uncertain')) and not c.get('upstream_complete') for c in phase.calls)
                write_new(path/'transport.json',encode({'calls':phase.calls,'discovery':phase.discovery,'active':phase.active,'accepted':phase.accepted,'local_quiescent':quiet,'upstream_quiescent':not unresolved}))
                require(quiet and not unresolved,'upstream completion is unproven; stop all dispatch and retain indeterminate slot')
            with self.phase_lock:
                self.current=None


class Handler(BaseHTTPRequestHandler):
    def __init__(self,request,client_address,server):
        with server.recorder.phase_lock:
            self.phase=server.phases[request]
        super().__init__(request,client_address,server)

    def setup(self):
        super().setup()
        self.connection.settimeout(3)

    def log_message(self,*_):
        pass

    def respond(self,status,content_type,data):
        try:
            self.send_response(status)
            self.send_header('Content-Type',content_type)
            self.send_header('Content-Length',str(len(data)))
            self.send_header('Connection','close')
            self.end_headers()
            self.wfile.write(data)
        except OSError:
            pass

    def do_GET(self):
        self.dispatch(False)

    def do_POST(self):
        self.dispatch(True)

    def dispatch(self,inference):
        recorder=self.server.recorder
        phase=self.phase
        if phase is None:
            self.respond(403,'application/json',b'{"error":"no active slot"}')
            return
        start=time.monotonic()
        with phase.condition:
            if inference:
                phase.attempted+=1
                sequence=phase.attempted
                prefix=phase.path/f'{sequence:03d}'
                record={'attempt':sequence,'forwarded':False,'usage':None,'client_path':self.path}
                phase.calls.append(record)
            else:
                prefix=phase.path/f'discovery-{len(phase.discovery)+1:03d}'
                record={'client_path':self.path,'forwarded':False}
                phase.discovery.append(record)
            permitted=(not phase.closed and not phase.unresolved and not phase.forwarding and time.monotonic()<phase.deadline
                       and (phase.attempted<=phase.max_requests if inference else len(phase.discovery)<=4))
            if not permitted:
                record['error']='request/time/concurrency budget exhausted'
                write_new(prefix.with_suffix('.json'),encode(record))
                self.respond(429,'application/json',b'{"error":"budget exhausted"}')
                return
            phase.active+=1
            phase.forwarding=True
        released=False
        def release_forwarding():
            nonlocal released
            if not released:
                with phase.condition:
                    phase.forwarding=False
                    released=True
        try:
            if inference:
                require(self.path in ('/v1/chat/completions','/v1/chat/completions?autoload=false'),'unexpected inference path')
                auth=self.headers.get_all('Authorization',[])
                require((phase.learning and not auth) or (len(auth)==1 and hmac.compare_digest(auth[0],'Bearer '+recorder.token)),'authorization mismatch')
                lengths=self.headers.get_all('Content-Length',[])
                require(len(lengths)==1 and len(lengths[0])<=10 and lengths[0].isascii() and lengths[0].isdigit() and not self.headers.get('Transfer-Encoding'),'invalid request framing')
                count=int(lengths[0])
                require(0<count<=REQUEST_LIMIT,'request too large')
                body=self.rfile.read(count)
                write_new(prefix.with_suffix('.request'),body)
                require(len(body)==count,'incomplete request body')
                payload=decode(body)
                require(isinstance(payload,dict) and payload.get('model')==recorder.model,'request model alias mismatch')
                require('autoload' not in payload or payload['autoload'] is False,'autoload refused')
                requested=payload.get('max_tokens')
                require(type(requested) is int and 0<requested<=phase.tokens,'invalid token ceiling')
                remain=phase.tokens-phase.reserved_tokens
                require(remain>0,'cumulative output token ceiling reached')
                payload['max_tokens']=min(requested,remain)
                record['reserved_output_tokens']=payload['max_tokens']
                phase.reserved_tokens+=payload['max_tokens']
                forwarded=encode(payload)
                write_new(prefix.with_suffix('.forwarded'),forwarded)
                upstream='/v1/chat/completions?autoload=false'
            else:
                parsed=urlsplit(self.path)
                require(parsed.path in ('/models','/props') and not (parsed.fragment or parsed.scheme or parsed.netloc),'unexpected discovery path')
                pairs=parse_qsl(parsed.query,keep_blank_values=True,strict_parsing=True)
                query=dict(pairs)
                require(len(query)==len(pairs),'duplicate discovery query key')
                require(set(query)<=({'autoload','model'} if parsed.path=='/props' else {'autoload'}),'unexpected discovery query key')
                require('autoload' not in query or query['autoload']=='false','discovery autoload refused')
                require('model' not in query or query['model']==recorder.model,'discovery model alias mismatch')
                forwarded=None
                upstream=parsed.path+'?'+urlencode({'autoload':'false',**({'model':recorder.model} if 'model' in query else {})})
            record.update(upstream_path=upstream)
            response_path=prefix.with_suffix('.response')
            status,content_type=request_direct(recorder.address,'POST' if inference else 'GET',upstream,phase.deadline,response_path,forwarded,phase,record)
            record.update(status=status,content_type=content_type)
            data=response_path.read_bytes()
            require(status==200,f'upstream HTTP {status}; redirects and retries refused')
            if inference:
                aliases,usage=response_metadata(data,content_type)
                record.update(response_aliases=sorted(aliases),usage=usage,upstream_complete=True)
                require(aliases=={recorder.model},'response model alias mismatch or absent')
                if usage and usage['output_tokens'] is not None:
                    phase.output_tokens+=usage['output_tokens']
                    phase.reserved_tokens-=payload['max_tokens']-usage['output_tokens']
                    require(usage['output_tokens']<=payload['max_tokens'] and phase.output_tokens<=phase.tokens,'upstream violated token ceiling')
            record['acknowledged']=True
            release_forwarding()
            self.respond(status,content_type,data)
        except (LifetimeError,OSError,ValueError,HTTPException) as error:
            record['error']=f'{type(error).__name__}: {error}'[:1000]
            if inference:
                phase.unresolved=(record.get('transmission_started',False) and not record.get('upstream_complete',False))
                if not record.get('transmission_started'):
                    phase.reserved_tokens-=record.get('reserved_output_tokens',0)
            release_forwarding()
            self.respond(502,'application/json',b'{"error":"recording transport refused request"}')
        finally:
            release_forwarding()
            record['elapsed_ms']=int((time.monotonic()-start)*1000)
            try:
                write_new(prefix.with_suffix('.json'),encode(record))
            finally:
                with phase.condition:
                    phase.active-=1
                    phase.condition.notify_all()
