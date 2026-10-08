#!/usr/bin/env python3
"""Black-box email registration checks against a real Palpo + isolated Postgres.

Prerequisite: postgres container exposing a loopback port, with password
palpo-otp-isolated-test. Creates and drops only its own unique test database.
AgentMail is a local recording transport; no real mail is sent by this suite.
--serve keeps the tested server alive for native Rinx acceptance afterward.
"""
import argparse, concurrent.futures, json, os, pathlib, re, secrets, socket
import subprocess, threading, time, urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def request(url, body=None, token=None, ip='10.77.0.1'):
    headers = {'Content-Type':'application/json', 'X-Forwarded-For':ip}
    if token: headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode(), headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as r: return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


class Fixture:
    def __init__(self, args):
        self.args, self.messages, self.reject = args, [], False
        self.root = args.output.resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        self.db = 'email_otp_' + secrets.token_hex(5)
        self.process = None
        fixture = self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_GET(self): self.respond(200, fixture.messages)
            def do_POST(self):
                assert self.path == '/v0/inboxes/fixture%40agentmail.to/messages/send' or self.path == '/v0/inboxes/fixture@agentmail.to/messages/send', self.path
                assert self.headers['Authorization'] == 'Bearer ' + 'test-key-'*5
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if fixture.reject: return self.respond(503, {'message':'provider temporarily unavailable'})
                fixture.messages.append(body)
                self.respond(200, {'message_id':str(len(fixture.messages)), 'thread_id':'test'})
            def respond(self, code, value):
                b = json.dumps(value).encode()
                self.send_response(code); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(b))); self.end_headers(); self.wfile.write(b)
        self.mail = ThreadingHTTPServer(('127.0.0.1',0), Handler)
        threading.Thread(target=self.mail.serve_forever, daemon=True).start()
        self.url = 'http://127.0.0.1:' + str(free_port())
        self.mail_url = 'http://127.0.0.1:' + str(self.mail.server_port)

    def sql(self, sql, database=None):
        return subprocess.check_output(['docker','exec',self.args.pg_container,'psql','-U','postgres','-d',database or self.db,'-At','-c',sql], text=True).strip()

    def start(self):
        self.sql('CREATE DATABASE '+self.db, 'postgres')
        for name, value in [('key','test-key-'*5),('otp-secret',secrets.token_hex(32))]:
            p = self.root/name; p.write_text(value); p.chmod(0o600)
        config = f'''server_name = "otp.test"
allow_registration = true
registration_token = "invite"
allow_federation = false
trusted_proxies = ["127.0.0.1/32"]
[[listeners]]
address = "{self.url.removeprefix('http://')}"
[db]
url = "postgres://postgres:palpo-otp-isolated-test@127.0.0.1:{self.args.pg_port}/{self.db}"
[well_known]
client = "{self.url}"
[storage]
backend = "fs"
root = "{self.root}/media"
[rc_registration]
per_second = 100.0
burst = 100
[registration_email]
agentmail_inbox = "fixture@agentmail.to"
agentmail_api_key_file = "{self.root}/key"
otp_secret_file = "{self.root}/otp-secret"
agentmail_api_url = "{self.mail_url}/v0/"
'''
        (self.root/'palpo.toml').write_text(config)
        self.log = (self.root/'palpo.log').open('w')
        self.process = subprocess.Popen([str(self.args.binary.resolve()),'--config',str(self.root/'palpo.toml')],cwd=self.root,stdout=self.log,stderr=subprocess.STDOUT)
        deadline=time.monotonic()+180
        while time.monotonic()<deadline:
            if self.process.poll() is not None: raise RuntimeError('Palpo exited: '+str(self.root/'palpo.log'))
            try:
                if request(self.url+'/_matrix/client/versions')[0] == 200: break
            except (OSError,ValueError): pass
            time.sleep(.25)
        else: raise TimeoutError('Palpo startup')
        (self.root/'state.json').write_text(json.dumps({'server':self.url,'mail_capture':self.mail_url,'database':self.db,'pid':self.process.pid}))

    def close(self):
        if self.process:
            self.process.terminate()
            try:self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
            self.log.close()
        self.mail.shutdown(); self.mail.server_close()
        self.sql('DROP DATABASE IF EXISTS '+self.db+' WITH (FORCE)', 'postgres')

    def api(self, route, body=None, **kw): return request(self.url+'/_matrix/client/v3/'+route,body,**kw)
    def issue(self, name, attempt=1, secret=None):
        secret=secret or secrets.token_hex(16)
        body={'email':name+'@example.org','client_secret':secret,'send_attempt':attempt}
        status,result=self.api('register/email/requestToken',body,ip='10.77.1.'+str(len(self.messages)+1))
        assert status==200,(status,result)
        code=re.search(r'\b[0-9]{6}\b',self.messages[-1]['text']).group()
        return {'sid':result['sid'],'client_secret':secret},code,body
    def verify(self, proof, code, **kw): return self.api('register/email/submitToken',dict(proof,token=code),**kw)
    def begin(self, name):
        body={'username':name,'password':'local-test-password'}
        status,result=self.api('register',body)
        assert status==401,(status,result)
        assert result['flows']==[{'stages':['m.login.email.identity','m.login.registration_token']}],result
        return body,result['session']
    def email_stage(self, body, session, proof):
        return self.api('register',dict(body,auth={'type':'m.login.email.identity','session':session,'threepid_creds':proof}))
    def finish(self, body, session, token='invite'):
        return self.api('register',dict(body,auth={'type':'m.login.registration_token','session':session,'token':token}))

    def test(self):
        checks=[]
        def passed(name): checks.append(name); print('PASS',name,flush=True)
        assert request(self.url+'/_matrix/client/unstable/org.palpo.registration')[1]['email_otp'] is True
        assert self.api('register',{'kind':'guest'})[0]==403
        bypass,bypass_session=self.begin('bypass')
        assert self.api('register',dict(bypass,auth={'type':'m.login.dummy','session':bypass_session}))[0]==401
        assert self.finish(bypass,bypass_session)[0]==401
        assert self.sql("SELECT count(*) FROM users WHERE localpart='bypass'")=='0'
        passed('discover required email; guest and UIAA bypasses denied')
        p,code,send=self.issue('happy')
        count=len(self.messages)
        assert self.api('register/email/requestToken',send)[1]['sid']==p['sid']
        assert len(self.messages)==count
        assert self.api('register/email/requestToken',dict(send,send_attempt=2))[0]==429
        body,session=self.begin('happy')
        assert self.email_stage(body,session,p)[0]==403
        assert self.verify(dict(p,client_secret='wrong-secret-123456789'),code)[0]==403
        assert self.verify(p,'000000' if code!='000000' else '000001')[0]==403
        assert self.verify(p,code)[0]==200
        passed('idempotent send; cooldown; wrong secret/code; must verify before registration')
        status,result=self.email_stage(body,session,p)
        assert status==401 and 'm.login.email.identity' in result['completed'],(status,result)
        assert self.finish(body,session,'bad-invite')[0]==403
        status,result=self.finish(body,session)
        assert status==200,(status,result)
        contacts=self.api('account/3pid',token=result['access_token'])[1]
        assert contacts['threepids'][0]['address']=='happy@example.org',contacts
        assert self.verify(p,code)[0]==403
        other,other_session=self.begin('replay')
        assert self.email_stage(other,other_session,p)[0]==403
        passed('email + invitation stages; verified contact stored; consumed proof cannot be replayed')
        p,code,_=self.issue('lockout')
        bad='000000' if code!='000000' else '000001'
        for i in range(5): assert self.verify(p,bad,ip='10.77.2.2')[0]==403
        assert self.verify(p,code,ip='10.77.2.2')[0]==403
        assert self.sql("SELECT failed_attempts FROM registration_email_sessions WHERE sid='"+p['sid']+"'")=='5'
        passed('five wrong guesses are durable; correct code then rejected')
        p,code,_=self.issue('expiry')
        self.sql("UPDATE registration_email_sessions SET expires_at=0 WHERE sid='"+p['sid']+"'")
        assert self.verify(p,code,ip='10.77.2.3')[0]==403
        passed('expired code rejected')
        p,code,_=self.issue('proofexpiry')
        assert self.verify(p,code,ip='10.77.2.30')[0]==200
        self.sql("UPDATE registration_email_sessions SET verified_at=verified_at-1800001 WHERE sid='"+p['sid']+"'")
        body,session=self.begin('proofexpiry')
        assert self.email_stage(body,session,p)[0]==403
        passed('expired verified proof rejected')
        old,old_code,send=self.issue('resend')
        self.sql("UPDATE registration_email_sessions SET created_at=created_at-61000 WHERE sid='"+old['sid']+"'")
        new,new_code,_=self.issue('resend',2,old['client_secret'])
        assert old['sid']!=new['sid']
        assert self.verify(old,old_code,ip='10.77.2.4')[0]==403
        assert self.verify(new,new_code,ip='10.77.2.4')[0]==200
        passed('resend invalidates earlier code')
        body,session=self.begin('binding')
        assert self.email_stage(body,session,new)[0]==401
        assert self.finish(dict(body,username='changed'),session)[0]==403
        assert self.sql("SELECT count(*) FROM users WHERE localpart IN ('binding','changed')")=='0'
        passed('proof bound to intended username; failed registration inserts no account')
        self.reject=True
        status,result=self.api('register/email/requestToken',{'email':'delivery@example.org','client_secret':secrets.token_hex(16),'send_attempt':1},ip='10.77.3.1')
        assert status>=400,(status,result)
        assert self.sql("SELECT count(*) FROM registration_email_sessions WHERE email='delivery@example.org' AND sent_at IS NOT NULL")=='0'
        self.reject=False
        passed('provider failure never returns a successful send or usable proof')
        p,code,_=self.issue('race')
        assert self.verify(p,code,ip='10.77.3.2')[0]==200
        body,session=self.begin('race')
        assert self.email_stage(body,session,p)[0]==401
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            results=list(pool.map(lambda _:self.finish(body,session),range(2)))
        assert sum(status==200 for status,_ in results)==1,results
        assert self.sql("SELECT count(*) FROM users WHERE localpart='race'")=='1'
        passed('concurrent consumption creates exactly one account')
        (self.root/'report.json').write_text(json.dumps({'passed':True,'checks':checks},indent=2)+'\n')


def main():
    p=argparse.ArgumentParser();p.add_argument('--binary',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--pg-container',default='palpo-email-otp-db');p.add_argument('--pg-port',type=int,default=54329);p.add_argument('--serve',action='store_true')
    args=p.parse_args();f=Fixture(args)
    try:
        f.start();f.test()
        print('STATE',str(f.root/'state.json'),flush=True)
        if args.serve:
            while True:time.sleep(1)
    finally:f.close()
if __name__=='__main__':main()
