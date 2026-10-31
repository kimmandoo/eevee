#!/usr/bin/env python3
"""Real isolated PTY/file/SQLite formation acceptance; no SQL writes or model calls."""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import sys
from terminal_smoke import Session, confirm_exit, terminal_environment
from native_connection_smoke import until
from workbench_smoke import wait_for

ESC='\x1b'
RIGHT,DOWN,UP=ESC+'[C',ESC+'[B',ESC+'[A'
DRAFT='Keep 界e\u0301 suffix'


def run(binary,output,root,size,):
    output.mkdir(parents=True)
    root.mkdir()
    project=root/'project 空 格';project.mkdir()
    home=root/'home';home.mkdir()
    tools=root/'tools';tools.mkdir()
    git=shutil.which('git');assert git
    if os.name!='nt':
        (tools/'git').symlink_to(git)
        (tools/'python3').symlink_to(Path(sys.executable).resolve())
    env=terminal_environment('mono')
    for key in list(env):
        if key.startswith(('EV_','EEVEE_','CODEX_','OPENAI_','ANTHROPIC_','CLAUDE_','GEMINI_','GOOGLE_','AWS_','OPENCODE_')) or key.lower().endswith('_proxy'):env.pop(key,None)
    env.update(HOME=str(home),USERPROFILE=str(home),XDG_CONFIG_HOME=str(home/'config'),XDG_DATA_HOME=str(home/'data'),XDG_CACHE_HOME=str(home/'cache'),APPDATA=str(home/'appdata'),LOCALAPPDATA=str(home/'local'),PATH=str(tools) if os.name!='nt' else str(Path(git).parent))
    subprocess_args=[git,'-c','core.hooksPath=','init','--quiet']
    import subprocess
    subprocess.run(subprocess_args,cwd=project,check=True,capture_output=True)
    data=root/'data';dbpath=data/'workbench.sqlite3'
    serial=0;session=None
    cases=[]
    def rows(table):
        with closing(sqlite3.connect(dbpath.as_uri()+'?mode=ro',uri=True)) as db:
            db.row_factory=sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM '+table+' ORDER BY rowid')]
    def task():return rows('tasks')[0]
    def draft(partner):return json.loads(task()['partner_drafts']).get(str(partner),{}).get('drafts',[])
    def forms():return [p['form'] for p in rows('partners')]
    def launch():
        nonlocal serial,session
        serial+=1
        session=Session([str(binary), '--continue', '--data-dir', str(data),'--images','off','--color','mono',*(['--ascii'] if size[0]==60 else [])], project,env,size,output/('terminal-'+str(serial)))
        wait_for(session,'Storage: saved',timeout=35)
    def homeward():
        for _ in range(8):
            footer=session.display().rstrip().splitlines()[-1].strip()
            if footer.startswith(('Type Request |','Enter Send |','Draft only |',)):return
            session.send(ESC)
        raise AssertionError(session.display())
    def wait_forms(expected):until(session,lambda:forms()==expected,'saved forms '+str(expected))
    def quit_app():
        session.command('quit');confirm_exit(session);assert session.wait_exit()==0;session.close()
    try:
        launch()
        session.command('goal');session.paste('Formation local acceptance');session.send('\r')
        until(session,lambda:rows('tasks') and task()['goal']=='Formation local acceptance','saved goal')
        homeward()
        for count in (2,3):
            session.command('party');session.command('add')
            until(session,lambda:len(rows('partners'))==count,'actual added partner')
            homeward()
        for ident,form,index in ((1,'vaporeon',1),(2,'jolteon',2),(3,'glaceon',7)):
            session.command('lead '+str(ident));until(session,lambda:task()['lead_partner']==ident,'lead saved')
            homeward();session.command(form);until(session,lambda:rows('partners')[ident-1]['form']==index,'form saved');homeward()
        session.command('lead 1');until(session,lambda:task()['lead_partner']==1,'lead one')
        homeward();session.paste(DRAFT)
        session.send(ESC+'[H'+RIGHT*5)
        session.send('\x13')
        until(session,lambda:any(d['field']=='Request' and d['text']==DRAFT for d in draft(1)),'draft stored')
        preserved=draft(1)
        # Save is a complete inspectable inert recipe; neutral Enter/paste cannot publish.
        session.command('formation-save arrangement.toml')
        hint='y create new file'
        wait_for(session,hint)
        session.capture('formation-save-review')
        session.send('\r');session.paste('y');assert not (project/'arrangement.toml').exists()
        session.send(ESC);homeward();assert not (project/'arrangement.toml').exists()
        session.command('formation-save arrangement.toml');wait_for(session,hint);session.send('y')
        until(session,lambda:(project/'arrangement.toml').exists(),'reviewed new formation file')
        original=(project/'arrangement.toml').read_bytes()
        assert b'schema_version = 2' in original and b'[[roles]]' in original
        cases.append('reviewed-save-and-neutral-cancel-no-file')
        homeward();session.command('formation-save arrangement.toml');wait_for(session,hint);session.send('y')
        until(session,lambda:'existing exports are never overwritten' in session.display() or 'Cannot create' in session.display(),'existing destination refusal')
        assert (project/'arrangement.toml').read_bytes()==original
        session.send(ESC);homeward()
        cases.append('existing-output-byte-identical')
        # A user may author new inert request proposals in a saved formation.
        original=original.replace(b'request = ""', b'request = "Review the local check"')
        (project/'arrangement.toml').write_bytes(original)
        for ident in (1,2,3):
            session.command('lead '+str(ident));until(session,lambda:task()['lead_partner']==ident,'lead changed');homeward()
            session.command('eevee');until(session,lambda:rows('partners')[ident-1]['form']==0,'neutral form saved');homeward()
        session.command('lead 1');until(session,lambda:task()['lead_partner']==1,'lead returned');homeward()
        session.command('formation arrangement.toml')
        map_hint='y apply'
        wait_for(session,map_hint);session.capture('formation-three-role-mapping')
        session.send('\r');session.paste('y');assert forms()==[0,0,0]
        session.send(ESC);homeward();assert draft(1)==preserved
        session.command('formation arrangement.toml');wait_for(session,map_hint)
        session.send('y');wait_forms([1,2,7]);homeward()
        assert draft(1)==preserved
        assert any(d['field']=='Request' and d['text']=='Review the local check' for d in draft(2))
        assert any(d['field']=='Request' and d['text']=='Review the local check' for d in draft(3))
        assert [p['auto_form'] for p in rows('partners')]==[0,0,0]
        assert not rows('active_work') and not rows('executions') and not rows('provider_journal')
        cases.append('three-partner-atomic-apply-preserved-Unicode-caret-no-effects')
        session.capture('formation-applied-direct-home')
        # A duplicate late mapping must roll back even earlier staged changes.
        session.command('formation arrangement.toml');wait_for(session,map_hint)
        session.send(DOWN+ESC+'[D')  # runner now duplicates scout (#1)
        before=rows('partners'),task()['partner_drafts']
        session.send('y')
        until(session,lambda:'different partner' in session.display(),'duplicate mapping refusal')
        assert (rows('partners'),task()['partner_drafts'])==before
        session.capture('duplicate-mapping-refused')
        session.send(ESC);homeward();cases.append('duplicate-mapping-rolls-back')
        # A saved definition can have more roles than the current party; missing remains visible.
        extra='\n[[roles]]\nrole = "missing"\nform = "Sylveon"\nrequest = "Prepare documentation"\n'
        (project/'four.toml').write_bytes(original+extra.encode())
        session.command('formation four.toml');wait_for(session,map_hint)
        wait_for(session,'empty slot');session.capture('missing-slot')
        session.send(ESC);homeward();cases.append('missing-partner-visible-without-creation')
        quit_app();launch()
        assert forms()==[1,2,7] and draft(1)==preserved
        session.command('formation arrangement.toml');wait_for(session,map_hint)
        session.capture('saved-definition-reopened-after-restart');session.send(ESC);homeward()
        assert not rows('active_work') and not rows('executions') and not rows('provider_journal')
        cases.append('restart-definition-and-owned-drafts-retained')
        # Actual owned local work must block formation apply, not merely a fake UI flag.
        session.command('lead 2');until(session,lambda:task()['lead_partner']==2,'busy runner lead');homeward()
        argv=['python3','-c','import time; print("FORMATION_BUSY", flush=True); time.sleep(30)']
        session.command('command-edit');session.paste(json.dumps(argv));session.send('\r')
        until(session,lambda:task()['command']==json.dumps(argv),'saved literal command')
        session.command('execute')
        until(session,lambda:any(title in session.display() for title in ('Review one-shot execution permission',)),'neutral command review')
        session.send(RIGHT+'\r')
        until(session,lambda:rows('active_work') and rows('executions')[-1]['reserved'],'actual busy owner')
        session.command('lead 1');until(session,lambda:task()['lead_partner']==1,'front a different idle owner');homeward()
        before=rows('partners'),task()['partner_drafts']
        session.command('formation arrangement.toml');wait_for(session,map_hint);session.send('y')
        until(session,lambda:any(word in session.display() for word in ('owned work before',)),'busy formation refusal')
        assert (rows('partners'),task()['partner_drafts'])==before
        session.capture('actual-busy-owner-refused-with-drafts-retained')
        session.send(ESC);session.command('lead 2');session.command('run');session.command('cancel')
        until(session,lambda:not rows('active_work') and not rows('executions')[-1]['reserved'],'owned cancellation settled')
        assert len(rows('executions'))==1 and not rows('provider_journal')
        homeward();cases.append('actual-busy-owner-refusal-and-explicit-cancellation')
        quit_app()
        result={'status':'passed','size':size,'cases':cases}
        (output/'results.json').write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n')
        return result
    finally:
        if session:
            if session.alive():session.capture('failure-last')
            session.close()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);
    args=parser.parse_args();binary=args.binary.resolve();output=args.output.resolve();output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix='ev-formations-') as temp:
        results=[run(binary,output/str(size[0]),Path(temp)/str(size[0]),size,) for size in ((120,36),(60,18))]
    report={'status':'passed','binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'journeys':results}
    (output/'results.json').write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
