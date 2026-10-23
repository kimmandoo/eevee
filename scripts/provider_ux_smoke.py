#!/usr/bin/env python3
"""Actual unified connection UI and native routing; disposable PTYs and loopback only.

Native OAuth reviews are inspected/cancelled before network authorization. Actual
issuer login/entitlement is not inferred from local transport fixtures.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tempfile
import traceback

from connection_smoke_helpers import FIELDS, fill_text, focus_field
from native_connection_smoke import MODEL, Peer, until
from response_smoke import SurfaceJourney
from terminal_smoke import confirm_exit
from workbench_smoke import wait_for

ESC='\x1b'
LABELS={
    'en': {'account':'Sign in with ChatGPT','api':'Use an API key','custom':'Custom / local endpoint',
           'browser':'Browser login','device':'Device login','review':'Review connection',
           'advanced':'Advanced settings','warning':'NOT encrypted/keychain','token':'Access token'},
    'ko': {'account':'ChatGPT로 로그인','api':'API 키 사용','custom':'사용자 지정 / 로컬 연결',
           'browser':'브라우저 로그인','device':'기기 로그인','review':'연결 검토',
           'advanced':'고급 설정','warning':'암호화/키체인 아님','token':'액세스 토큰'},
}
DRAFT='Keep 요청界e\u0301 future draft'
ANSWER='LOCAL_UNIFIED_OPENAI_ROUTE_CONFIRMED'


def choose(session,label):
    wait_for(session,label)
    for _ in range(12):
        if any('>' in line and label in line for line in session.display().splitlines()):
            session.send('\r');return
        session.send(ESC+'[B')
    raise AssertionError('Action not reachable: '+label+'\n'+session.display())


def saved_draft(journey):
    return [(task['id'],owner,draft['text'],draft['cursor'])
            for task in journey.snapshot()['tasks'] for owner,state in task['partner_drafts'].items()
            for draft in state.get('drafts',[]) if draft['field']=='Request']


def run(binary,output,root,size,language):
    output.mkdir(parents=True)
    labels=LABELS[language];cases=[]
    with Peer('responses','sk-fixture-unified-openai-932a8f',False) as peer:
        journey=SurfaceJourney(binary,output,root,size,size[0]==60,'openai','responses',False,True,peer,language=language)
        session=journey.session
        try:
            wait_for(session,'Storage: saved')
            journey.write_request(DRAFT)
            session.send(ESC+'[D'+ESC+'[D'+'\x13')
            until(session,lambda: any(row[2]==DRAFT and row[3]<len(DRAFT.encode()) for row in saved_draft(journey)),'saved future caret')
            before=saved_draft(journey)
            # A fresh missing connection must recover directly to setup, not
            # lose the draft in a failing model request or diagnostics detour.
            session.send('\r')
            wait_for(session,labels['account'])
            assert not peer.snapshot()
            journey.workspace()
            assert saved_draft(journey)==before
            session.command('connect')
            wait_for(session,'OpenAI')
            journey.capture('one-openai-family-root')
            rows=[line for line in session.display().splitlines() if re.search(r'OpenAI\s*\|',line)]
            assert len(rows)==1,'OpenAI family is not a single visible row'
            assert not any('Codex · native' in line or 'Codex · CLI agent |' in line or 'OpenAI · API |' in line for line in session.display().splitlines())
            session.paste('private-navigation-paste')
            journey.workspace()
            assert saved_draft(journey)==before,'connection navigation paste reached the Request'
            cases.append('single-family-root-and-noneditor-private-paste-isolation')

            session.command('connect openai');choose(session,labels['account'])
            wait_for(session,labels['browser']);wait_for(session,labels['device'])
            assert labels['token']+':' not in session.display(),'raw token is the first login UX'
            journey.capture('native-account-methods-without-cli')
            for method in ('browser','device'):
                choose(session,labels[method]);wait_for(session,labels['review'])
                journey.capture(method+'-exact-neutral-auth-review')
                session.send('\r')  # Neutral Back must not start OAuth.
                wait_for(session,labels['browser'])
                assert not peer.snapshot(),'neutral OAuth review caused fixture traffic'
            journey.workspace();assert saved_draft(journey)==before
            cases.append('native-browser-and-device-reviews-no-cli-neutral-no-network')

            session.command('connect openai');choose(session,labels['api'])
            wait_for(session,FIELDS[language]['key']+':')
            journey.private_paste(peer.secret)
            assert peer.secret not in session.display()
            focus_field(session,'storage',language);session.send(' ')
            focus_field(session,'save',language)
            assert labels['warning'] in session.display(),'compact plaintext warning was not visible'
            journey.capture('masked-api-primary-and-complete-storage-disclosure')
            focus_field(session,'advanced',language);session.send('\r')
            fill_text(session,'endpoint',peer.base,language)
            fill_text(session,'model','模型界',language)
            session.send(ESC+'[H'+ESC+'[C'+'X')
            assert '模X型界' in session.display(),'private CJK caret changed the wrong character'
            fill_text(session,'model',MODEL,language)
            focus_field(session,'save',language)
            session.resize((59,17));session.send('\r')
            assert not peer.snapshot(),'below-minimum setup granted metadata or inference'
            assert not (journey.home/'.eevee-connections'/'openai.json').exists()
            session.resize(size);session.pump(.2)
            assert labels['warning'] in session.display()
            session.send('\r');journey.choose_saved_model(1)
            assert not peer.snapshot('POST') and saved_draft(journey)==before
            stored=json.loads((journey.home/'.eevee-connections'/'openai.json').read_text())
            assert stored['base_url']==peer.base and stored['protocol']=='responses'
            assert not (journey.home/'.eevee-connections'/'chatgpt.json').exists()
            cases.append('api-method-routes-responses-exact-source-cjk-resize-guard-no-inference')

            peer.plans.put(('complete',ANSWER))
            session.send('\r')
            until(session,lambda:len(peer.snapshot('POST'))==1,'explicit Request sent once')
            wait_for(session,ANSWER)
            until(session,lambda:not journey.snapshot()['claims'] and 'Storage: saved' in session.display(),'request settled')
            row=peer.snapshot('POST')[0]
            assert row['path'].endswith('/responses')
            journey.assert_payload(row,[('user',DRAFT)],'unified-api-exact-request')
            assert journey.snapshot()['executions']==0
            journey.capture('actual-openai-api-reply-without-route-fallback')
            cases.append('explicit-request-uses-selected-responses-route-and-exact-draft')

            journey.workspace();session.command('connect openai');choose(session,labels['custom'])
            wait_for(session,FIELDS[language]['endpoint']+':')
            wait_for(session,FIELDS[language]['protocol']+':')
            journey.capture('custom-endpoint-auth-protocol-explicit')
            session.send(ESC);journey.workspace()
            assert len(peer.snapshot('POST'))==1 and len(peer.snapshot('GET'))==1,'cancelled method switched/fetched/replayed'
            cases.append('custom-method-discloses-endpoint-protocol-and-cancel-keeps-account')
            session.command('quit');confirm_exit(session);assert session.wait_exit()==0
            peer.assert_clean()
            return {'status':'passed','language':language,'size':list(size),'cases':cases}
        finally:session.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args();binary=args.binary.resolve();output=args.output.resolve();output.mkdir(parents=True,exist_ok=False)
    report={'status':'failed','sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'journeys':[],
            'limits':'Actual disposable PTYs and loopback API; native OAuth network not authorized, no live account/model/billing or physical terminal claim.'}
    try:
        for language in ('en','ko'):
            for size in ((120,36),(60,18)):
                with tempfile.TemporaryDirectory(prefix='ev-provider-ux-') as temporary:
                    try:report['journeys'].append(run(binary,output/f'{language}-{size[0]}',Path(temporary),size,language))
                    except Exception as error:
                        report['journeys'].append({'status':'failed','language':language,'size':list(size),'error':str(error),'traceback':traceback.format_exc()})
        if all(row['status']=='passed' for row in report['journeys']):report['status']='passed'
    finally:(output/'results.json').write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps({'status':report['status'],'results':str(output/'results.json')}));return 0 if report['status']=='passed' else 1

if __name__=='__main__':raise SystemExit(main())
