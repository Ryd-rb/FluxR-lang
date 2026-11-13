#!/usr/bin/env python3
"""Replace oracle functions with canonical FluxR native forms; retain test code."""
from pathlib import Path
import re
from structured_forms import FORMS as STRUCTURED_FORMS
ROOT=Path(__file__).resolve().parent
FORMS={
 'pg_roundtrip':'''native-fn native_pg_roundtrip 2 pg "&mut Pg" sql "&str" "io::Result<Reply>" 1
tail method path pg query 1 path sql
''',
 'atomic_increment':'''native-fn atomic_increment 2 counter "&AtomicUsize" iterations usize unit 1
for unused bin range int 0usize path iterations 1
do method path counter fetch_add 2 int 1usize path Ordering::Relaxed
''',
 'thread_value':'''native-fn native_thread_value 0 usize 1
tail method method call std::thread::spawn 1 move 0 int 42usize join 0 unwrap 0
''',
 'tcp_write':'''native-fn native_tcp_write 2 stream "&mut TcpStream" bytes "&[u8]" "io::Result<()>" 1
tail method path stream write_all 1 path bytes
'''}
FORMS.update(STRUCTURED_FORMS)
def raw(data):
    return b'rust '+str(len(data)).encode()+b'\n'+data+b'\n'
def make(stem):
    source=(ROOT/(stem+'.rs')).read_bytes()
    output=bytearray(b'fluxr 1\n')
    cursor=0
    for match in re.finditer(rb'// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN (\w+)\n.*?// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END \1\n',source,re.S):
        output.extend(raw(source[cursor:match.start()]))
        output.extend(FORMS[match.group(1).decode()].encode())
        cursor=match.end()
    output.extend(raw(source[cursor:]))
    (ROOT/(stem+'.fxn')).write_bytes(output)
if __name__=='__main__':
    make('capabilities')
    make('postgres_wire')
    make('structured_semantics')
