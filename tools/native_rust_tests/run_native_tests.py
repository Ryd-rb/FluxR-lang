#!/usr/bin/env python3
"""Run independent native test oracles; no silent skips and no mock PG results."""
import argparse, hashlib, importlib.util, json, os, pathlib, re, shutil, subprocess, sys, time

ROOT=pathlib.Path(__file__).resolve().parent

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=pathlib.Path,required=True)
    parser.add_argument('--postgres',action='store_true')
    parser.add_argument('--frontend',type=pathlib.Path)
    parser.add_argument('--target-dir',type=pathlib.Path)
    args=parser.parse_args()
    if args.frontend:
        if args.target_dir is None:
            parser.error('--target-dir is required for unified Runtime qualification')
        from run_unified_native_tests import main as unified_main
        raise SystemExit(unified_main(args))
    args.output.mkdir(parents=True,exist_ok=False)
    rustc=shutil.which('rustc')
    if not rustc: raise SystemExit('rustc unavailable; no qualification was executed')
    rows=[]
    source_paths=[ROOT/name for name in ['capabilities.rs','capabilities.fxn','postgres_wire.rs','postgres_wire.fxn','structured_semantics.rs','structured_semantics.fxn','structured_forms.py','make_fluxr_fixture.py','run_native_tests.py','with_postgres.py']]
    source_paths+=sorted((ROOT/'compile_fail').glob('*'))
    source_paths=[p for p in source_paths if p.is_file()]
    sources_before={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source_paths}
    inputs={name: ROOT/name for name in ['capabilities.rs','postgres_wire.rs','structured_semantics.rs']}
    cases=json.loads((ROOT/'compile_fail/manifest.json').read_text())['cases']
    negative_root=ROOT/'compile_fail'
    def run(name,cmd,expect=0,diagnostic=None,fatal=True):
        start=time.monotonic()
        result=subprocess.run(cmd,capture_output=True,timeout=180)
        (args.output/f'{name}.stdout.log').write_bytes(result.stdout)
        (args.output/f'{name}.stderr.log').write_bytes(result.stderr)
        passed=result.returncode==0 if expect==0 else result.returncode!=0
        if diagnostic is not None:
            messages=[]
            for line in result.stderr.splitlines():
                try: messages.append(json.loads(line))
                except ValueError: pass
            codes=[m.get('code',{}).get('code') for m in messages if isinstance(m.get('code'),dict)]
            passed=passed and diagnostic in codes
        row={'name':name,'command':cmd,'exit':result.returncode,'wall_seconds':time.monotonic()-start,'passed':passed}
        summary=re.search(rb'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;',result.stdout)
        if summary:
            row['rust_test_counts']=dict(zip(['passed','failed','ignored','measured','filtered_out'],map(int,summary.groups())))
        if b"unix_filesystem_listener" in result.stdout and b"PermissionDenied" in result.stdout and b"Operation not permitted" in result.stdout:
            row['environment_observation']='filesystem UnixListener::bind denied with EPERM; required full gate remains failed'
        rows.append(row)
        (args.output/'commands.json').write_text(json.dumps(rows,indent=2)+'\n')
        if not passed and fatal: raise RuntimeError(f'{name} failed, inspect retained logs')
    report={'schema':'fluxr-native-capabilities-oracle-v1','passed':False,'commands':rows,'postgres_requested':args.postgres, 'fluxr_emitted':False,
            'limitations':['Direct Rust reference tests, not FluxR-emitted qualification.','Not conformance-suite or challenge-scale qualification.']}
    try:
        run('rustc-version',[rustc,'-vV'])
        for mode,flags in [('debug',[]),('release',['-C','opt-level=3','-C','codegen-units=1'])]:
            binary=str(args.output/f'capabilities-{mode}')
            run(f'compile-capabilities-{mode}',[rustc,'--edition=2024','--test',str(inputs['capabilities.rs']),'-o',binary,*flags])
            run(f'capabilities-{mode}',[binary,'--test-threads=4'],fatal=False)
            binary=str(args.output/f'structured-semantics-{mode}')
            run(f'compile-semantics-{mode}',[rustc,'--edition=2024','--test',str(inputs['structured_semantics.rs']),'-o',binary,*flags])
            run(f'semantics-{mode}',[binary,'--test-threads=2'],fatal=False)
        for case in cases:
            run('reject-'+pathlib.Path(case['file']).stem,[rustc,'--edition=2024','--error-format=json',str(negative_root/case['file']),'-o',str(args.output/'must-not-exist')],expect=1,diagnostic=case['diagnostic'],fatal=False)
        if args.postgres:
            binary=str(args.output/'postgres-wire')
            run('compile-postgres',[rustc,'--edition=2024','--test',str(inputs['postgres_wire.rs']),'-o',binary,'-C','opt-level=3'])
            run('postgres-live',[binary,'--test-threads=2'],fatal=False)
        report['passed']=all(row['passed'] for row in rows)
        report['groups']={
          'structured_semantics':all(row['passed'] for row in rows if row['name'].startswith('semantics-')),
          'compile_rejections':all(row['passed'] for row in rows if row['name'].startswith('reject-')),
          'postgres_live':next((row['passed'] for row in rows if row['name']=='postgres-live'),None),
          'capabilities_full':all(row['passed'] for row in rows if row['name'].startswith('capabilities-')),
        }
    finally:
        report['sources_before']=sources_before
        report['sources_after']={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source_paths}
        report['sources_unchanged']=report['sources_before']==report['sources_after']
        report['frontend_sha256_at_load']=None
        report['frontend_unchanged']=True
        report['compiled_sources']={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [*inputs.values(),*(negative_root/case['file'] for case in cases)]}
        report['passed']=report['passed'] and report['sources_unchanged'] and report['frontend_unchanged']
        report['postgres_status']='executed-see-command-results' if args.postgres else 'not-executed'
        (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'commands':len(rows),'output':str(args.output)}))
    raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
