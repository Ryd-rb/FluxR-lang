#!/usr/bin/env python3
"""Reproduce trusted VKF1 fixture validation and actual backend execution.

No arbitrary generated candidate admission. The independent validator/disassembler
is a separately acquired SPIRV-Tools binary. Every command and failure is retained.
"""
import argparse, hashlib, json, math, os, re, struct, subprocess, time
from pathlib import Path

def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def validate_timeout(timeout_seconds):
    if (type(timeout_seconds) not in (int, float)
            or not math.isfinite(timeout_seconds) or timeout_seconds <= 0):
        raise ValueError("timeout must be finite and positive")


class CommandRecorder:
    """Record real child outcomes, accepting only the exact normal exit code.

    Trusted fixture supervision only: this is not a sandbox or a promise of
    hostile descendant containment. A signal death, launch failure or timeout
    never establishes a validator refusal. Every started attempt has its log and
    ledger entry written before an unexpected outcome is raised to the caller.
    """
    def __init__(self, root, output, env=None, timeout_seconds=300):
        validate_timeout(timeout_seconds)
        self.root = Path(root).resolve(strict=True)
        self.output = Path(output).resolve(strict=True)
        self.env = dict(os.environ if env is None else env)
        self.timeout_seconds = timeout_seconds
        self.commands = []
        self.names = set()
        if (self.output / "commands.json").exists():
            raise FileExistsError("command evidence already exists; choose a new output directory")

    def run(self, name, cmd, expected=0):
        if type(expected) is not int or not 0 <= expected <= 255:
            raise ValueError("expected exit must be a normal integer exit code from 0 to 255")
        if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name)
                or name in self.names):
            raise ValueError("command names must be unique safe log names")
        log = self.output / (name + ".log")
        argv = [str(argument) for argument in cmd]
        # Reserve the log name before starting any child, so a retry cannot
        # overwrite a prior receipt or execute without its intended log slot.
        with log.open("xb") as stream:
            self.names.add(name)
            start = time.monotonic()
            launch_error = None
            timed_out = False
            try:
                result = subprocess.run(argv, cwd=self.root, env=self.env,
                                        capture_output=True, timeout=self.timeout_seconds)
                status = result.returncode
                stdout = result.stdout
                data = result.stdout + result.stderr
            except (OSError, subprocess.TimeoutExpired) as error:
                launch_error = f"{argv[0]!r}: {error!r}"
                timed_out = isinstance(error, subprocess.TimeoutExpired)
                status = None
                stdout = getattr(error, "stdout", b"") or b""
                data = (stdout + (getattr(error, "stderr", b"") or b"")
                        + f"{launch_error}: {error}".encode())
            stream.write(data)
        matched = launch_error is None and status == expected
        terminal = ("timeout" if timed_out else "launch_error" if launch_error is not None
                    else "matched_expected_exit" if matched else "unexpected_exit")
        record = {
            "name": name, "argv": argv, "cwd": str(self.root),
            "exit_status": status, "expected_exit": expected,
            "matched_expected_exit": matched, "terminal": terminal,
            "launch_error": launch_error, "timed_out": timed_out,
            "terminated_by_signal": os.name == "posix" and status is not None and status < 0,
            "timeout_seconds": self.timeout_seconds,
            "elapsed_seconds": time.monotonic() - start,
            "log": log.name, "sha256": digest(log),
        }
        self.commands.append(record)
        (self.output / "commands.json").write_text(json.dumps(self.commands, indent=2) + "\n")
        if not matched:
            raise RuntimeError(f"{name}: expected exit {expected}, observed {status} ({terminal}); see {log}")
        return stdout.decode(errors="replace")


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',required=True,type=Path);p.add_argument('--target-dir',required=True,type=Path);p.add_argument('--spirv-tools',required=True,type=Path);p.add_argument('--icd',type=Path);p.add_argument('--timeout-seconds',type=float,default=300);a=p.parse_args()
    try:
        validate_timeout(a.timeout_seconds)
    except ValueError as error:
        p.error(str(error))
    root=Path(__file__).resolve().parents[1];out=a.output.resolve();a.spirv_tools=a.spirv_tools.resolve(strict=True)
    if out.is_relative_to(root):raise ValueError('evidence must remain outside source checkout')
    out.mkdir(parents=True,exist_ok=False);env=dict(os.environ,CARGO_TARGET_DIR=str(a.target_dir.resolve()))
    if a.icd:env['VK_DRIVER_FILES']=str(a.icd.resolve())
    recorder = CommandRecorder(root, out, env, timeout_seconds=a.timeout_seconds)
    run = recorder.run
    head=run('source-head',['git','rev-parse','HEAD','HEAD^{tree}']).splitlines()
    run('tools-version',[a.spirv_tools/'spirv-val','--version'])
    run('build',['cargo','build','--locked','--release','--features','vulkan','--examples'])
    target=a.target_dir.resolve()/'release/examples'
    hello=run('hello',[target/'hello_agent'])
    if hello!='Hello, FluxR!\n':raise RuntimeError('FluxR hello world mismatch')
    spv=out/'affine.spv';reflection=json.loads(run('emit',[target/'compute_probe','emit','examples/vkf1-affine.fluxr',spv]))
    run('validate',[a.spirv_tools/'spirv-val','--target-env','vulkan1.0',spv])
    dis=run('disassemble',[a.spirv_tools/'spirv-dis',spv])
    checks={
        'compute_entry':bool(re.search(r'OpEntryPoint GLCompute .* "main"',dis)),
        'local_size_64':bool(re.search(r'OpExecutionMode .* LocalSize 64 1 1',dis)),
        'array_stride_4':bool(re.search(r'OpDecorate .* ArrayStride 4',dis)),
        'bindings_0_1_2':sorted(map(int,re.findall(r'OpDecorate .* Binding (\d+)',dis)))==[0,1,2],
        'descriptor_set_zero':re.findall(r'OpDecorate .* DescriptorSet (\d+)',dis)==['0','0','0'],
        'push_constant':bool(re.search(r'OpVariable .* PushConstant',dis)),
        'no_contraction_all_two_ops':len(re.findall(r'OpDecorate .* NoContraction',dis))==2,
        'guarded_storage_access':dis.index('OpULessThan')<dis.index('OpBranchConditional')<dis.index('OpFAdd')<dis.index('OpStore'),
        'read_only_inputs':len(re.findall(r'OpDecorate .* NonWritable',dis))==2,
        'write_only_output':len(re.findall(r'OpDecorate .* NonReadable',dis))==1,
    }
    if not all(checks.values()):raise RuntimeError(f'independent reflection failure: {checks}')
    words=list(struct.unpack('<'+'I'*(spv.stat().st_size//4),spv.read_bytes()))
    mutations=[]
    for name,index,value in [('bad_magic',0,0),('zero_bound',3,0),('zero_word_count',5,17)]:
        changed=words.copy();changed[index]=value;path=out/(name+'.spv');path.write_bytes(struct.pack('<'+'I'*len(changed),*changed));run(name,[a.spirv_tools/'spirv-val','--target-env','vulkan1.0',path],expected=1);mutations.append(name)
    again=out/'affine-again.spv';run('emit-repeat',[target/'compute_probe','emit','examples/vkf1-affine.fluxr',again])
    if spv.read_bytes()!=again.read_bytes():raise RuntimeError('SPIR-V reproducibility failure')
    run('vulkan-execution',[target/'compute_probe','gpu','examples/vkf1-affine.fluxr',out/'execution.json'])
    execution=json.loads((out/'execution.json').read_text())
    final={'schema':'fluxr-compute-verification-v1','head':head[0],'tree':head[1],'status':'passed','reflection':reflection,'independent_disassembly_checks':checks,'malformed_spirv_refused':mutations,'repeated_shader_bytes_equal':True,'spirv_sha256':digest(spv),'device':execution['device'],'dispatches':sum(len(c['samples']) for c in execution['cases']),'dispatches_scope':'original affine comparison matrix only; tensor bridge separately submits three operations (small success, nonfinite-output refusal after execution, and model affine)','tensor_bridge_submissions':3,'tensor_bridge':execution['tensor_bridge'],'scope':'reviewed fixture execution; no arbitrary candidate admission, general numerical equivalence, physical GPU, or all-command execution claim','commands':'commands.json'}
    (out/'report.json').write_text(json.dumps(final,indent=2)+'\n');print(json.dumps(final,indent=2))
if __name__=='__main__':main()
