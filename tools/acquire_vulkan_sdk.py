#!/usr/bin/env python3
"""Acquire a pinned optional Linux x86_64 software Vulkan/validator SDK locally.

No system install, root access, authentication changes, or shipped binary copies.
Native dependencies are recorded, not promised portable to every Linux distro.
"""
import argparse, hashlib, json, platform, subprocess, urllib.request
from pathlib import Path

PACKAGES = [
    {"name":"mesa-vulkan.deb","url":"https://deb.debian.org/debian/pool/main/m/mesa/mesa-vulkan-drivers_25.0.7-2+deb13u1_amd64.deb","sha256":"caf2f7d296b7522efe74e40a74bce762a3ce84c33b53b5e54e1248ac2e0a13a5","bytes":14185752,"license":"MIT and other per-file licenses; usr/share/doc/mesa-vulkan-drivers/copyright in package"},
    {"name":"spirv-tools.deb","url":"https://deb.debian.org/debian/pool/main/s/spirv-tools/spirv-tools_2025.1~rc1-1_amd64.deb","sha256":"2cc272ef1340cf0b47de6c5fcccaf91574d7a8f4061abb718d6c639463e85aab","bytes":4260164,"license":"Apache-2.0; usr/share/doc/spirv-tools/copyright in package"},
]
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('destination',type=Path);a=p.parse_args()
    if platform.system()!='Linux' or platform.machine() not in ('x86_64','amd64'):raise SystemExit('this SDK route is Linux x86_64 only')
    root=a.destination.resolve();root.mkdir(parents=True,exist_ok=False)
    records=[]
    for package in PACKAGES:
        path=root/package['name'];digest=hashlib.sha256();size=0
        with urllib.request.urlopen(package['url'],timeout=60) as src,path.open('xb') as dst:
            while block:=src.read(128*1024):
                size+=len(block)
                if size>package['bytes']:raise ValueError('download exceeds pinned size')
                digest.update(block);dst.write(block)
        if size!=package['bytes'] or digest.hexdigest()!=package['sha256']:raise ValueError('download identity mismatch')
        run=subprocess.run(['dpkg-deb','-x',str(path),str(root)],capture_output=True,text=True)
        records.append({**package,'exit_status':run.returncode,'stdout':run.stdout,'stderr':run.stderr})
        if run.returncode:raise RuntimeError(run.stderr)
        path.unlink() # Reacquirable package copy not needed after extraction.
    icd=root/'usr/share/vulkan/icd.d/lvp_icd.json';data=json.loads(icd.read_text());data['ICD']['library_path']=str(root/'usr/lib/x86_64-linux-gnu/libvulkan_lvp.so')
    local=root/'lvp-local.json';local.write_text(json.dumps(data,indent=2)+'\n')
    report={'packages':records,'VK_DRIVER_FILES':str(local),'spirv_val':str(root/'usr/bin/spirv-val'),'spirv_dis':str(root/'usr/bin/spirv-dis'),'scope':'software Vulkan; physical GPU not supplied; LLVM/system library dependencies must resolve'}
    (root/'acquisition.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()
