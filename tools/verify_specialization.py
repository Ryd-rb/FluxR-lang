#!/usr/bin/env python3
"""Qualify fixed VKS1 template via independent SPIRV-Tools and real Vulkan.

Reviewed fixtures only. Compiler and executor are not a hostile-candidate sandbox.
The v1 compute gate remains a separate required compatibility qualification.
"""
import argparse
import json
import os
from pathlib import Path
import re
import struct
from verify_compute import CommandRecorder, digest, validate_timeout


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--target-dir', type=Path, required=True)
    p.add_argument('--spirv-tools', type=Path, required=True)
    p.add_argument('--icd', type=Path, required=True)
    p.add_argument('--timeout-seconds', type=float, default=300)
    a = p.parse_args()
    validate_timeout(a.timeout_seconds)
    root = Path(__file__).resolve().parents[1]
    out = a.output.resolve()
    if out.is_relative_to(root):
        raise ValueError('evidence must be outside verified checkout')
    out.mkdir(parents=True, exist_ok=False)
    target = a.target_dir.resolve()
    sdk = a.spirv_tools.resolve(strict=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), VK_DRIVER_FILES=str(a.icd.resolve(strict=True)))
    run = CommandRecorder(root, out, env, a.timeout_seconds).run
    head = run('source-head', ['git', 'rev-parse', 'HEAD', 'HEAD^{tree}']).splitlines()
    dirty = run('source-status', ['git', 'status', '--porcelain=v1', '--untracked-files=all'])
    if dirty:
        raise RuntimeError('commit reviewed source before qualification; dirty source has no exact Git tree identity')
    run('validator-version', [sdk/'spirv-val', '--version'])
    run('build', ['cargo', 'build', '--locked', '--release', '--features', 'vulkan', '--example', 'specialization_probe'])
    binary = target/'release/examples/specialization_probe'
    before_binary = digest(binary)
    spv = out/'template.spv'
    reflection = json.loads(run('emit', [binary, 'emit', spv]))
    run('validate-template', [sdk/'spirv-val', '--target-env', 'vulkan1.0', spv])
    dis = run('disassemble', [sdk/'spirv-dis', spv])
    checks = {
        'specialization_ids': sorted(map(int, re.findall(r'OpDecorate .* SpecId (\d+)', dis))) == [1, 2],
        'two_f32_spec_constants': len(re.findall(r'= OpSpecConstant %float 0', dis)) == 2,
        'two_bindings': sorted(map(int, re.findall(r'OpDecorate .* Binding (\d+)', dis))) == [0, 1],
        'two_arithmetic_no_contraction': len(re.findall(r'OpDecorate .* NoContraction', dis)) == 2,
        'single_dynamic_read': len(re.findall(r'OpDecorate .* NonWritable', dis)) == 1,
        'write_only_output': len(re.findall(r'OpDecorate .* NonReadable', dis)) == 1,
        'guard_before_storage': dis.index('OpULessThan') < dis.index('OpBranchConditional') < dis.index('OpFAdd') < dis.index('OpStore'),
        'variant_pins_differ': reflection['a']['variant_sha256'] != reflection['b']['variant_sha256'],
        'same_template': reflection['a']['spirv_sha256'] == reflection['b']['spirv_sha256'] == digest(spv),
    }
    if not all(checks.values()):
        raise RuntimeError(f'independent reflection checks failed: {checks}')
    # Independently replace specialization IDs using the standard optimizer,
    # freeze those constants, and validate the resulting fixed module as well.
    frozen = out/'frozen.spv'
    run('specialize-independent', [sdk/'spirv-opt', '--set-spec-const-default-value', '1:2 2:1', '--freeze-spec-const', spv, '-o', frozen])
    run('validate-frozen', [sdk/'spirv-val', '--target-env', 'vulkan1.0', frozen])
    frozen_dis = run('disassemble-frozen', [sdk/'spirv-dis', frozen])
    if 'OpSpecConstant' in frozen_dis or ' SpecId ' in frozen_dis:
        raise RuntimeError('independent full specialization did not freeze constants')
    words = list(struct.unpack('<'+'I'*(spv.stat().st_size//4), spv.read_bytes()))
    at = 5
    while at < len(words):
        if words[at] & 65535 == 71 and words[at+2] == 1:
            break
        at += words[at] >> 16
    # A SpecId decoration on an ordinary constant is invalid. A signal or an
    # arbitrary nonzero validator exit is not accepted as this normal refusal.
    target_id = words[at+1]
    changed = words.copy()
    at = 5
    while at < len(changed):
        if changed[at] & 65535 == 50 and changed[at+2] == target_id:
            changed[at] = (changed[at] & 0xffff0000) | 43
            break
        at += changed[at] >> 16
    malformed = out/'invalid-spec-decoration.spv'
    malformed.write_bytes(struct.pack('<'+'I'*len(changed), *changed))
    run('invalid-spec-decoration', [sdk/'spirv-val', '--target-env', 'vulkan1.0', malformed], expected=1)
    repeat = out/'template-repeat.spv'
    repeated_reflection = json.loads(run('emit-repeat', [binary, 'emit', repeat]))
    if repeat.read_bytes() != spv.read_bytes() or repeated_reflection != reflection:
        raise RuntimeError('shader or reflection changed across identical runs')
    run('real-vulkan', [binary, 'gpu', out/'execution.json'])
    execution = json.loads((out/'execution.json').read_text())
    if before_binary != digest(binary):
        raise RuntimeError('runtime binary drift')
    final_head = run('source-head-after', ['git', 'rev-parse', 'HEAD', 'HEAD^{tree}']).splitlines()
    final_dirty = run('source-status-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all'])
    if final_head != head or final_dirty != dirty:
        raise RuntimeError('source drift')
    report = {'schema':'fluxr-vks1-verification-v1', 'status':'passed', 'source_head':head[0], 'source_tree':head[1],
              'source_dirty':bool(dirty), 'binary_sha256':before_binary, 'reflection':reflection,
              'checks':checks, 'device':execution['device'], 'dispatches':181,
              'refusals':execution['refusals'], 'commands':'commands.json',
              'scope':'reviewed fixed fixture; real device identified above; no physical GPU inference, full Vulkan coverage, or general strict numeric equivalence'}
    (out/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
