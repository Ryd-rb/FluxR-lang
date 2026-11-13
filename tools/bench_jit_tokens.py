#!/usr/bin/env python3
"""Pinned full-string JIT authoring pairs; no live-model or competence claim."""
import hashlib
from importlib.metadata import version
import json
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from eval.isolation import (UntrustedExecutionDisabled, admit_frozen_fixtures,
                            require_untrusted_sandbox)


def source(name):
    p = (ROOT / name).resolve(strict=True)
    if not p.is_relative_to(ROOT) or p.stat().st_size > 2 * 1024 * 1024:
        raise ValueError('static source outside admitted bound')
    return p.read_text(encoding='utf-8')


def identity(text):
    return {'bytes':len(text.encode()), 'sha256':hashlib.sha256(text.encode()).hexdigest()}


def main():
    import tiktoken
    if version('tiktoken') != '0.14.0':
        raise ValueError('reproduce with pinned tiktoken==0.14.0')
    catalog = tomllib.loads(source('benchmarks/jit-v12.toml'))['case']
    report = {'schema':'fluxr-jit-static-token-panel-v12', 'tiktoken_version':version('tiktoken'),
        'equivalent_rust_max_ratio':0.1, 'profiles':[],
        'scope':'Actual full-string encodings including whitespace, punctuation, adjacency and separately JSON quoting; static authoring costs only.',
        'code_episode':{'status':'not_executed','model':None,'trained_custom_tokenizer':False,
            'attempts':None,'cached_input_tokens':None,'uncached_input_tokens':None,'output_tokens':None,
            'tool_tokens':None,'repairs':None,'review_tokens':None,'handoffs':None,'latency_seconds':None,
            'accepted_task_success':None,'reason':'No qualified arbitrary-candidate compilation/execution adapter or pinned live-model episode telemetry; development agents are not a frozen held-out trial.'}}
    for name in ('o200k_base','cl100k_base'):
        enc = tiktoken.get_encoding(name)
        h = hashlib.sha256()
        for token,rank in sorted(enc._mergeable_ranks.items(), key=lambda x:x[1]):
            h.update(rank.to_bytes(4,'little'));h.update(len(token).to_bytes(4,'little'));h.update(token)
        rows = []
        for key,c in catalog.items():
            a,b = source(c['fluxr']),source(c['rust'])
            ac,bc = len(enc.encode_ordinary(a)),len(enc.encode_ordinary(b))
            rows.append({'case':key,'fluxr':{'path':c['fluxr'],**identity(a),'tokens':ac,
                'json_wire_tokens':len(enc.encode_ordinary(json.dumps(a,ensure_ascii=False)))},
                'rust':{'path':c['rust'],**identity(b),'tokens':bc,
                'json_wire_tokens':len(enc.encode_ordinary(json.dumps(b,ensure_ascii=False)))},
                'source_ratio':ac/bc,'passes_0_1_target':ac <= .1*bc})
        fa1='fa1 v2\npub calc 2 x int y int int ret rem add mul x y 17 997'
        rust=source('examples/fixtures/jit_scalar.rs')
        rows.append({'case':'jit_scalar_fa1','fluxr':{'literal':fa1,**identity(fa1),'tokens':len(enc.encode_ordinary(fa1)),'json_wire_tokens':len(enc.encode_ordinary(json.dumps(fa1)))},
            'rust':{'path':'examples/fixtures/jit_scalar.rs',**identity(rust),'tokens':len(enc.encode_ordinary(rust)),'json_wire_tokens':len(enc.encode_ordinary(json.dumps(rust)))},
            'source_ratio':len(enc.encode_ordinary(fa1))/len(enc.encode_ordinary(rust)),
            'passes_0_1_target':len(enc.encode_ordinary(fa1)) <= .1*len(enc.encode_ordinary(rust))})
        common = []
        for path,kind in [('examples/bench_jit.rs','shared Rust/FluxR measurement host and binding source'),('docs/prepared-execution.md','complete API/semantics/measurement retrieval document')]:
            text=source(path);common.append({'path':path,'kind':kind,**identity(text),'tokens':len(enc.encode_ordinary(text)),'allocation':'reported separately; not silently charged only to the Rust algorithm baseline or omitted from a live episode'})
        report['profiles'].append({'tokenizer':name,'vocabulary_rank_bytes_sha256':h.hexdigest(),
            'regex_sha256':hashlib.sha256(enc._pat_str.encode()).hexdigest(),'method':'encode_ordinary once per full UTF-8 string; no component-token summation',
            'compatible_trained_model_or_embedding_evaluation':'not performed; existing encoding is a measurement profile, not a model competency result',
            'rows':rows,'common_host_and_retrieval_costs':common,'source_target_passes':sum(r['passes_0_1_target'] for r in rows)})
    try:
        require_untrusted_sandbox()
    except UntrustedExecutionDisabled as e:
        report['arbitrary_candidate_admission']={'status':'blocked_as_required','message':str(e),'compiler_and_execution_qualification':False}
    else:
        raise AssertionError('unexpected untrusted execution admission')
    frozen=tomllib.loads(source('benchmarks/cases.toml'))['heldout']
    admitted=admit_frozen_fixtures(ROOT,frozen)
    report['frozen_legacy_admission']={k:v for k,v in admitted.items() if k not in ('replay_bytes','cases')}
    report['frozen_legacy_admission']['execution']='not run by this static counter; unchanged authority bytes verified only'
    print(json.dumps(report,indent=2))


if __name__ == '__main__':
    main()
