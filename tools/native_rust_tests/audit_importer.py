#!/usr/bin/env python3
"""Independent pinned-source mutation probes for the multicore importer."""
import argparse,hashlib,importlib.util,json,pathlib,shutil,sys,tempfile

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--reference',type=pathlib.Path,required=True)
    parser.add_argument('--output',type=pathlib.Path,required=True)
    args=parser.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    toolroot=pathlib.Path(__file__).resolve().parents[1]
    sys.path.insert(0,str(toolroot))
    import fluxr as native
    import prepare_multicore as importer
    rows=[]
    source=args.reference/importer.SUBMISSION
    with tempfile.TemporaryDirectory(prefix='import-audit-',dir=args.output) as directory:
        stage=pathlib.Path(directory)
        fixture=stage/'reference'; (fixture/importer.SUBMISSION).parent.mkdir(parents=True)
        shutil.copytree(source,fixture/importer.SUBMISSION)
        shutil.copyfile(args.reference/'LICENSE',fixture/'LICENSE')
        initial={p.relative_to(fixture).as_posix():p.read_bytes() for p in fixture.rglob('*') if p.is_file()}
        for mode in ['lossless','structured']:
            output=stage/('valid-'+mode)
            provenance=importer.prepare(fixture,output,mode)
            manifest=output/'fluxr.json'
            project=native.load_project(manifest)
            assert project.receipt['runtime_required'] is True
            assert '.fluxr-runtime/src/runtime.rs' in project.files
            if mode=='lossless':
                # Emission adds the required runtime. The lossless promise is
                # about imported native payloads, not VM-free emitted files.
                config=json.loads(manifest.read_text())
                for entry in config['files']:
                    data=(output/entry['source']).read_bytes()
                    if entry['mode']=='source':
                        data=native.parse_source(data).native
                    assert data==(source/entry['output']).read_bytes()
            rows.append({'case':'valid-'+mode,'passed':True,'structured_functions':sum(x.get('structured_functions',0) for x in provenance['files'])})
        def rejected(label,mutator):
            mutator()
            output=stage/label
            try: importer.prepare(fixture,output,'structured')
            except (native.NativeError,OSError) as error:
                assert not output.exists(), 'rejected importer published output'
                rows.append({'case':label,'passed':True,'error':str(error)})
            else: raise AssertionError(label+' was not rejected')
            for p in list(fixture.rglob('*')):
                if p.is_file() and p.relative_to(fixture).as_posix() not in initial: p.unlink()
            for name,data in initial.items(): (fixture/name).write_bytes(data)
        rejected('tampered-auth',lambda:(fixture/importer.SUBMISSION/'src/auth.rs').write_bytes(initial[importer.SUBMISSION+'/src/auth.rs']+b'\n// changed\n'))
        rejected('tampered-cargo',lambda:(fixture/importer.SUBMISSION/'Cargo.toml').write_bytes(initial[importer.SUBMISSION+'/Cargo.toml']+b'\n# changed\n'))
        rejected('tampered-license',lambda:(fixture/'LICENSE').write_bytes(b'different license'))
        rejected('missing-module',lambda:(fixture/importer.SUBMISSION/'src/wire.rs').unlink())
        rejected('extra-module',lambda:(fixture/importer.SUBMISSION/'src/unpinned.rs').write_text('pub fn unexpected(){}\n'))
    result={'schema':'fluxr-native-importer-independent-audit-v1','passed':all(r['passed'] for r in rows),'cases':rows,'frontend_sha256':hashlib.sha256((toolroot/'fluxr.py').read_bytes()).hexdigest(),'importer_sha256':hashlib.sha256((toolroot/'prepare_multicore.py').read_bytes()).hexdigest()}
    (args.output/'report.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'passed':result['passed'],'cases':len(rows)}))
if __name__=='__main__':main()
