#!/usr/bin/env python3
"""Exercise the installed CLI protocol through real bounded subprocesses.

Use an explicit compiled binary and fresh output directory. The caller owns
external resource coordination; every command and output is retained.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

SOURCE='fluxr 1\npub answer 1 x int int ret add x 1\n'

def qualify(binary, output):
    output.mkdir(parents=True,exist_ok=False)
    binary=binary.resolve()
    env={k:v for k,v in os.environ.items() if not k.startswith('FLUXR_CONTROL_')}
    checks=[]
    def command(name,args,input=None,expected=0):
        result=subprocess.run([str(binary),*args],input=input,stdout=subprocess.PIPE,
          stderr=subprocess.PIPE,env=env,timeout=30)
        (output/(name+'.stdout')).write_bytes(result.stdout)
        (output/(name+'.stderr')).write_bytes(result.stderr)
        checks.append({'name':name,'argv':[str(binary),*args],'exit_code':result.returncode,
          'expected_exit':expected,'stdout_sha256':hashlib.sha256(result.stdout).hexdigest(),
          'stderr_sha256':hashlib.sha256(result.stderr).hexdigest()})
        assert result.returncode==expected,(name,result.returncode,result.stderr.decode(errors='replace'))
        return result.stdout
    def request(op,**kw):
        return {'version':1,'token':'','command':{'op':op,**kw}}
    report={'schema':'fluxr-unified-cli-qualification-v1','status':'running',
      'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
      'checks':checks,'failures':[]}
    try:
        info=json.loads(command('version',['version']))
        assert info['language_version']==1 and info['runtime']=='embedded'
        source=output/'program.fluxr';source.write_bytes(SOURCE.encode())
        result=json.loads(command('run',['run',str(source),'answer','[{"i64":41}]']))
        assert result['ok'] and result['result']['value']=={'i64':42}
        updates=output/'updates.fluxr'
        updates.write_text('fluxr 1\npub calc 2 x int y int int do 0 3 bump x y scale x 3 ret x\n', encoding='utf-8', newline='\n')
        updated=json.loads(command('short-update-run',
            ['run',str(updates),'calc','[{"i64":2},{"i64":5}]']))
        assert updated['ok'] and updated['result']['value']=={'i64':21}
        update_aot=json.loads(command('short-update-aot',
            ['internal-emit-aot',str(updates),str(output/'updates.rs')]))
        assert update_aot['exports']==[{'name':'calc','status':'aot'}]
        invalid_update=output/'invalid-update.fluxr'
        invalid_update.write_text('fluxr 1\npub calc 1 x int int do 0 2 scale x true ret x\n', encoding='utf-8', newline='\n')
        rejected_update=json.loads(command('short-update-type-rejected',
            ['run',str(invalid_update),'calc','[{"i64":2}]'],expected=1))
        assert not rejected_update['ok']
        floats=output/'decimal-floats.fluxr'
        floats.write_text('fluxr 1\npub calc 1 x float float ret add mul x 0.5f 1.25f\n'
            'pub negative_zero 0 float ret -0f\n', encoding='utf-8', newline='\n')
        decimal_result=json.loads(command('decimal-float-run',
            ['run',str(floats),'calc',json.dumps([{'f32':0x40000000}])]))
        assert decimal_result['ok'] and decimal_result['result']['value']=={'f32':0x40100000}
        negative_zero=json.loads(command('decimal-negative-zero-run',
            ['run',str(floats),'negative_zero','[]']))
        assert negative_zero['ok'] and negative_zero['result']['value']=={'f32':0x80000000}
        overflow=output/'decimal-overflow.fluxr'
        overflow_source='fluxr 1\npub value 0 float ret 1e39f\n'
        overflow.write_bytes(overflow_source.encode())
        refused_overflow=json.loads(command('decimal-overflow-rejected',
            ['run',str(overflow),'value','[]'],expected=1))
        assert not refused_overflow['ok']
        predicate=output/'mixed-predicate.fluxr'
        predicate.write_text('fluxr 1\npub gate 2 enabled bool n int bool ret and enabled lt n 7\n', encoding='utf-8', newline='\n')
        for name,arguments,expected in [
            ('mixed-predicate-true',[{'bool':True},{'i64':5}],True),
            ('mixed-predicate-false',[{'bool':False},{'i64':5}],False),
        ]:
            result=json.loads(command(name,['run',str(predicate),'gate',json.dumps(arguments)]))
            assert result['ok'] and result['result']['value']=={'bool':expected}
        for name,arguments in [
            ('mixed-predicate-bool-tag-rejected',[{'i64':1},{'i64':5}]),
            ('mixed-predicate-int-tag-rejected',[{'bool':False},{'bool':True}]),
        ]:
            refused=json.loads(command(name,['run',str(predicate),'gate',json.dumps(arguments)],expected=1))
            assert not refused['ok']
        # A persistent stdio process is required: this catches a command handler
        # accidentally allocating a fresh runtime for each request.
        frames=[request('install',name='state',source=SOURCE,expected_revision=None),
          request('call',name='state',export='answer',arguments=[{'i64':41}]),
          request('install',name='state',source=SOURCE.replace('add x 1','add x 2'),expected_revision=1),
          request('call',name='state',export='answer',arguments=[{'i64':41}]),
          request('install',name='state',source=SOURCE,expected_revision=1),
          request('install',name='state',source=SOURCE.replace('int','uint'),expected_revision=2),
          request('call',name='state',export='answer',arguments=[{'i64':41}]),
          request('remove',name='state',expected_revision=2),
          request('install',name='state',source=SOURCE,expected_revision=None),
          request('inspect',name='state')]
        frames.insert(4,'malformed-json')
        wire=b''.join((x if isinstance(x,str) else json.dumps(x)).encode()+b'\n' for x in frames)
        responses=[json.loads(x) for x in command('persistent-stdio',['serve'],wire).splitlines()]
        assert len(responses)==len(frames)
        assert responses[0]['result']['revision']==1
        assert responses[1]['result']['value']=={'i64':42}
        assert responses[2]['result']['revision']==2
        assert responses[3]['result']['value']=={'i64':43}
        assert not responses[4]['ok'] and responses[4]['error']['code']=='invalid_request'
        assert not responses[5]['ok'] and responses[5]['error']['code']=='conflict'
        assert not responses[6]['ok']
        assert responses[7]['result']['value']=={'i64':43} and responses[7]['result']['revision']==2
        assert responses[8]['ok']
        assert responses[9]['result']['revision']==3 and responses[10]['result']['revision']==3
        # Equivalent authored float spellings publish a new revision while
        # preserving bits. A rejected overflowing literal cannot replace it.
        raw_float='fluxr 1\npub value 0 float ret 0x80000000f\n'
        decimal_float='fluxr 1\npub value 0 float ret -0f\n'
        float_frames=[request('install',name='float',source=raw_float,expected_revision=None),
          request('call',name='float',export='value',arguments=[]),
          request('install',name='float',source=decimal_float,expected_revision=1),
          request('call',name='float',export='value',arguments=[]),
          request('install',name='float',source=overflow_source,expected_revision=2),
          request('call',name='float',export='value',arguments=[]),
          request('inspect',name='float')]
        float_wire=b''.join(json.dumps(frame).encode()+b'\n' for frame in float_frames)
        float_responses=[json.loads(line) for line in
            command('decimal-persistent-stdio',['serve'],float_wire).splitlines()]
        assert len(float_responses)==len(float_frames)
        assert all(float_responses[index]['ok'] for index in (0,1,2,3,5,6))
        assert float_responses[0]['result']['revision']==1
        assert float_responses[2]['result']['revision']==2
        for index,revision in ((1,1),(3,2),(5,2)):
            assert float_responses[index]['result']['value']=={'f32':0x80000000}
            assert float_responses[index]['result']['revision']==revision
        assert not float_responses[4]['ok']
        assert float_responses[6]['result']['revision']==2
        # Explicit migration is separate from executing current source.
        legacy=output/'old.fa1';legacy.write_bytes(SOURCE.replace('fluxr 1','fa1 v2',1).encode())
        rejected=json.loads(command('legacy-rejected',['run',str(legacy),'answer'],expected=1))
        assert not rejected['ok']
        migrated=output/'migrated.fluxr'
        command('import',['import','fa1',str(legacy),str(migrated)])
        assert migrated.read_text().startswith('fluxr 1\n')
        result=json.loads(command('imported-run',['run',str(migrated),'answer','[{"i64":41}]']))
        assert result['result']['value']=={'i64':42}
        command('import-no-overwrite',['import','fa1',str(legacy),str(migrated)],expected=1)
        output_rust=output/'generated.rs'
        aot=json.loads(command('emit-aot',['internal-emit-aot',str(source),str(output_rust)]))
        assert aot['exports']==[{'name':'answer','status':'aot'}]
        assert 'pub fn bind' in output_rust.read_text()
        command('aot-no-overwrite',['internal-emit-aot',str(source),str(output_rust)],expected=1)
        host=output/'host.fluxr';host.write_text('fluxr 1\npub main 0 int ret host service int 0\n', encoding='utf-8', newline='\n')
        host_aot=json.loads(command('host-aot-fallback',['internal-emit-aot',str(host),str(output/'host.rs')]))
        assert host_aot['exports'][0]['status']=='vm' and host_aot['exports'][0]['reason']
        command('unterminated-wire',['serve'],b'{}',expected=1)
        command('oversized-wire',['serve'],b' '*(2*1024*1024)+b'\n',expected=1)
        report['status']='passed'
    except BaseException as error:
        report['status']='failed';report['failures'].append(type(error).__name__+': '+str(error))
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    return report

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    report=qualify(a.binary,a.output)
    print(json.dumps({'status':report['status'],'commands':len(report['checks']),'failures':report['failures']}))
    raise SystemExit(0 if report['status']=='passed' else 1)
