#!/usr/bin/env python3
"""Actual native-owner receipt -> old Node database -> Rust CLI adoption.
Requires an isolated coordinator_migration test artifact, never production state.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile

SEED = r'''
import {pathToFileURL} from 'node:url';
import {readFileSync} from 'node:fs';
const {Store}=await import(pathToFileURL(process.argv[1]));
const store=new Store(process.argv[2]);
const {nativeReceipt:n,authority}=JSON.parse(readFileSync(process.argv[3]));
const fleet=n.serverEngagementId, project=n.projects[0], agent=n.agents[0];
store.state.projects={};store.state.requests={};
const d=structuredClone(agent.definition); delete d.sourceEventId;
store.state.fleets[fleet]={id:fleet,ownerMxid:authority.owner,registrationGeneration:n.registrationGeneration,representativeMxid:`@${fleet}_representative:example.test`,state:'ready',transport:{generation:1}};
store.state.projects[project.grant.projectId]={id:project.grant.projectId,requestId:'action_original',fleetId:fleet,ownerMxid:project.grant.owner,...project.definition,createdAt:10};
store.state.actionInbox={records:{action_original:{id:'action_original',requestId:'original_project',kind:'project',ownerMxid:project.grant.owner,payload:{fleetId:fleet,name:project.definition.name},state:'approved',revision:2,execution:'done',result:{projectId:project.grant.projectId},decision:{by:'@original_admin:example.test',at:10,reason:'Original decision'}}},notices:{},rooms:{}};
store.state.requests[`${fleet}:${agent.request.id}`]={id:`${fleet}:${agent.request.id}`,requestId:agent.request.id,fleetId:fleet,projectId:project.grant.projectId,requesterMxid:agent.request.requester,payload:d,sourceEventId:agent.definition.sourceEventId,state:'active',createdAt:20,provider:{engagementId:agent.agentAllocationId,state:'active',allocatedTokens:agent.allocatedTokens,ready:false}};
store.state.unknownExtension={keep:'原有记录'};
store.save();store.close();
'''

def run(command, env=None, ok=True):
    result=subprocess.run(command,env=env,capture_output=True,text=True,timeout=30)
    if (result.returncode==0)!=ok:
        raise AssertionError(f'Unexpected exit {result.returncode}: {result.stderr[:1000]}')
    return result

def read(path):
    with sqlite3.connect(f'{path.as_uri()}?mode=ro',uri=True) as db:
        return json.loads(db.execute('SELECT body FROM state').fetchone()[0])

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--node',required=True);parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--native-fixture',type=Path,required=True)
    args=parser.parse_args();binary=str(args.binary.resolve());fixture=args.native_fixture.resolve()
    native=json.loads(fixture.read_text());n=native['nativeReceipt']
    assert n['version']==1 and native['authority']['server']=='example.test'
    with tempfile.TemporaryDirectory(prefix='palpo-native-adoption-') as temp:
        temp=Path(temp);db=temp/'source.sqlite';plan_path=temp/'plan.json';auth=temp/'authority.json'
        env={k:v for k,v in os.environ.items() if not k.startswith('PALPO_')}
        env.update(PALPO_SERVER_NAME='example.test',PALPO_URL='http://127.0.0.1:9',PALPO_ADMIN_DATABASE=str(db))
        store=Path(__file__).resolve().parents[3]/'crates/operations/tests/fixtures/legacy_node/lib/store.mjs'
        run([args.node,'--input-type=module','-e',SEED,str(store),str(db),str(fixture)])
        original=read(db)
        auth.write_text(json.dumps({'engagements':{n['serverEngagementId']:native['authority']},'resources':{},'projects':{}}))
        run([binary,'import-authority',str(auth)],env)
        inventory=json.loads(run([binary,'migration-inventory'],env).stdout)
        plan_path.write_text(json.dumps({'version':1,'id':'handoff','inventory':inventory,'nativeAuditDigest':n['sourceDigest']}))
        run([binary,'handoff-store',str(plan_path)],env)
        plan={'version':1,'id':'adopt_native','inventory':json.loads(run([binary,'migration-inventory'],env).stdout),'nativeReceipt':n,'pendingAgents':{}}
        plan_path.write_text(json.dumps(plan))
        receipt=json.loads(run([binary,'adopt-legacy',str(plan_path)],env).stdout)
        migrated=read(db);w=migrated['rustWorkflows']
        for key in ['fleets','projects','requests','actionInbox','unknownExtension','audit']:
            assert migrated.get(key)==original.get(key),key
        assert not w['outbox']
        assert w['actions']['action_original']['decision']['by']=='@original_admin:example.test'
        agents=[a for a in w['actions'].values() if a['request']['kind']=='agent']
        assert len(agents)==1
        agent=agents[0];assert agent['state']=='approved' and agent['execution']=='unknown'
        assert w['observations'][agent['id']]['engagementId']==n['agents'][0]['agentAllocationId']
        assert w['legacySources'][agent['id']]['native']['originalDecisions']==n['agents'][0]['originalDecisions']
        assert json.loads(run([binary,'adopt-legacy',str(plan_path)],env).stdout)==receipt
        # Back up all post-migration state and receipts. Reopening this copy
        # preserves the writer fence and replays without a second allocation.
        restored=temp/'restored.sqlite'
        with sqlite3.connect(db) as source,sqlite3.connect(restored) as target:source.backup(target)
        env['PALPO_ADMIN_DATABASE']=str(restored)
        assert json.loads(run([binary,'adopt-legacy',str(plan_path)],env).stdout)==receipt
        assert read(restored)==migrated
        plan['nativeReceipt']['agents'][0]['allocatedTokens']+=1
        plan_path.write_text(json.dumps(plan));run([binary,'adopt-legacy',str(plan_path)],env,ok=False)
        assert read(restored)==migrated
        print('PASS: native CLI receipt, Node source preservation, Rust adoption, original verdict/agent, replay, restore and conflict refusal')

if __name__=='__main__':main()
