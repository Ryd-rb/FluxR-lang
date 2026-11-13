#!/usr/bin/env python3
"""Validate the current requirements graph using only the Python standard library."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def validate(document):
    def require(condition, message):
        if not condition:
            raise ValueError(message)

    require(document.get('format') == 'fluxr-implementation-requirements-v1', 'unsupported requirements format')
    tasks = document['tasks']
    profiles = document['release_profiles']
    require(isinstance(tasks, list) and isinstance(profiles, list), 'tasks and profiles must be arrays')
    ids = [task['id'] for task in tasks]
    require(len(ids) == len(set(ids)), 'duplicate task IDs')
    require(all(re.fullmatch(r'[A-Z][0-9]{2}', item) for item in ids), 'invalid task ID')
    by_id = {task['id']: task for task in tasks}
    known = set(ids)
    criterion_ids = set()
    evidence = document['evidence_profiles']
    references = document['reference_sources']
    require(all(isinstance(rows, list) and rows and all(isinstance(row, str) and row for row in rows) for rows in evidence.values()), 'empty or malformed evidence profile')
    require(all(isinstance(url, str) and url.startswith(('https://', 'http://')) for url in references.values()), 'invalid reference URL')

    def check_refs(refs, label):
        require(isinstance(refs, list) and all(isinstance(item, str) for item in refs), label + ': expected string array')
        require(len(refs) == len(set(refs)), label + ': duplicate reference')
        require(set(refs) <= known, label + ': unknown task reference')

    for task in tasks:
        tid = task['id']
        for key in ('depends_on', 'start_after', 'optional_dependencies'):
            check_refs(task.get(key, []), f'{tid}.{key}')
            require(tid not in task.get(key, []), f'{tid}: self-dependency')
        for ref in task.get('reference_sources', []):
            require(ref in references, f'{tid}: unknown source reference {ref}')
        require(task['acceptance_checks'], f'{tid}: no acceptance checks')
        for criterion in task['acceptance_checks']:
            cid = criterion['id']
            require(re.fullmatch(re.escape(tid) + r'-AC[0-9]{2}', cid), f'{tid}: malformed criterion ID')
            require(cid not in criterion_ids, f'duplicate criterion {cid}')
            criterion_ids.add(cid)
            require(isinstance(criterion['criterion'], str) and criterion['criterion'].strip(), f'{cid}: empty criterion')
            require(criterion['evidence'] in evidence, f'{cid}: unknown evidence profile')

    def acyclic(graph, label):
        done = set()
        active = set()

        def visit(node):
            require(node not in active, f'{label}: dependency cycle at {node}')
            if node in done:
                return
            active.add(node)
            for dependency in graph[node]:
                visit(dependency)
            active.remove(node)
            done.add(node)

        for node in graph:
            visit(node)

    graph = {tid: set(task['depends_on']) for tid, task in by_id.items()}
    acyclic(graph, 'completion graph')
    start_graph = {tid: graph[tid] | set(task.get('start_after', task['depends_on'])) for tid, task in by_id.items()}
    acyclic(start_graph, 'combined start/completion graph')
    profile_ids = [profile['id'] for profile in profiles]
    require(len(profile_ids) == len(set(profile_ids)), 'duplicate profile IDs')
    for profile in profiles:
        pid = profile['id']
        check_refs(profile['required_tasks'], f'{pid}.required_tasks')
        check_refs(profile['required_closure'], f'{pid}.required_closure')
        scoped = {key: set(values) for key, values in graph.items()}
        for tid, refs in profile['scope_dependencies'].items():
            require(tid in known, f'{pid}: unknown scoped task')
            check_refs(refs, f'{pid}.{tid}')
            scoped[tid].update(refs)
        acyclic(scoped, pid)
        closure = set()
        pending = list(profile['required_tasks'])
        while pending:
            tid = pending.pop()
            if tid not in closure:
                closure.add(tid)
                pending.extend(scoped[tid])
        require(closure == set(profile['required_closure']), f'{pid}: inconsistent release closure')
    for owner in document['shared_contract_owners'].values():
        require(owner in known, f'unknown contract owner: {owner}')
    for gate in document['decision_gates']:
        require(gate['profile_id'] is None or gate['profile_id'] in profile_ids, f"{gate['id']}: unknown profile")
        check_refs(gate['required_tasks'], gate['id'])
    actual = {'tasks': len(tasks), 'acceptance_criteria': len(criterion_ids), 'release_profiles': len(profiles)}
    require(document['counts'] == actual, 'declared counts differ from actual contents')
    require(document['source_language']['header'] == 'fluxr 1', 'unexpected current language')
    require(document['source_language']['qualification_scopes_are_languages'] is False, 'release scopes cannot select separate languages')
    return actual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('path', nargs='?', type=Path, default=Path(__file__).resolve().parents[1] / 'docs' / 'requirements.json')
    args = parser.parse_args()
    try:
        data = json.loads(args.path.read_text(encoding='utf-8'), object_pairs_hook=unique_object)
        counts = validate(data)
    except (OSError, ValueError, KeyError, TypeError, RecursionError) as error:
        parser.exit(1, f'requirements invalid: {error}\n')
    print(json.dumps({'status': 'passed', **counts}, sort_keys=True))


if __name__ == '__main__':
    main()
