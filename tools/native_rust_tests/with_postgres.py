#!/usr/bin/env python3
"""Start a private PostgreSQL18 fixture over TCP, run tests, always stop/reap it."""
import argparse,json,os,pathlib,shutil,socket,subprocess,sys,time

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--frontend',type=pathlib.Path)
    p.add_argument('--target-dir',type=pathlib.Path)
    args=p.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    pg_bin=pathlib.Path(os.environ['PG_BIN']); pg_share=os.environ['PG_SHARE']
    data=args.output/'database'; logs=args.output/'postgres.log'; ledger=[]
    cmd=[str(pg_bin/'initdb'),'-D',str(data),'-L',pg_share,'--no-locale','--encoding=UTF8','--auth-local=trust','--auth-host=trust','--username=fluxr_native_test']
    with (args.output/'initdb.log').open('wb') as log:
        result=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,timeout=60)
    ledger.append({'command':cmd,'exit':result.returncode})
    if result.returncode: raise RuntimeError('initdb failed')
    with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    cmd=[str(pg_bin/'postgres'),'-D',str(data),'-h','127.0.0.1','-p',str(port),'-c','unix_socket_directories=','-c','fsync=on','-c','synchronous_commit=on','-c','max_connections=20']
    record={'schema':'fluxr-native-live-postgres-lifecycle-v1','port':port,'initdb':ledger,'postgres_command':cmd,'passed':False}
    with logs.open('wb') as log:
        server=subprocess.Popen(cmd,stdout=log,stderr=subprocess.STDOUT)
        try:
            deadline=time.monotonic()+15
            while True:
                if server.poll() is not None: raise RuntimeError('Postgres exited during startup')
                try:
                    with socket.create_connection(('127.0.0.1',port),timeout=.1): break
                except OSError:
                    if time.monotonic()>deadline: raise TimeoutError('Postgres startup deadline')
                    time.sleep(.05)
            env=dict(os.environ,FLUXR_TEST_PG_HOST='127.0.0.1',FLUXR_TEST_PG_PORT=str(port),FLUXR_TEST_PG_USER='fluxr_native_test',FLUXR_TEST_PG_DATABASE='postgres')
            test_cmd=[sys.executable,'-B',str(pathlib.Path(__file__).with_name('run_native_tests.py')),'--output',str(args.output/'tests'),'--postgres']
            if args.frontend: test_cmd+=['--frontend',str(args.frontend)]
            if args.target_dir: test_cmd+=['--target-dir',str(args.target_dir)]
            result=subprocess.run(test_cmd,env=env,timeout=900)
            record.update({'test_command':test_cmd,'test_exit':result.returncode,'passed':result.returncode==0})
        finally:
            stop=[str(pg_bin/'pg_ctl'),'-D',str(data),'-m','immediate','-w','stop']
            with (args.output/'stop.log').open('wb') as stop_log:
                stopped=subprocess.run(stop,stdout=stop_log,stderr=subprocess.STDOUT,timeout=30)
            record['stop_exit']=stopped.returncode
            try: server.wait(timeout=10)
            except subprocess.TimeoutExpired: server.kill(); server.wait(); record['forced_kill']=True
            record['server_exit']=server.returncode
            (args.output/'lifecycle.json').write_text(json.dumps(record,indent=2)+'\n')
            if server.poll() is not None: shutil.rmtree(data)
    print(json.dumps({'passed':record['passed'],'output':str(args.output)}))
    raise SystemExit(0 if record['passed'] else 1)
if __name__=='__main__':main()
