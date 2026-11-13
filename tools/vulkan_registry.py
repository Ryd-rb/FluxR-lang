#!/usr/bin/env python3
"""Pinned Vulkan registry inventory and generated raw binding coverage.

Records every registry call/type/enum/flag/handle/alias/platform/pNext relation.
Generation, loading, device enablement and execution are separate dimensions.
The output is evidence, not a safe FFI proof or a claim every call was executed.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import urllib.request
import xml.etree.ElementTree as ET

PIN = {
    "tag": "v1.3.281",
    "url": "https://raw.githubusercontent.com/KhronosGroup/Vulkan-Headers/v1.3.281/registry/vk.xml",
    "sha256": "96e57e5d5713be1384eb6aa1fe41e1539512217507af182367810c8988cc719a",
    "ash_version": "0.38.0+1.3.281",
    "license": "Apache-2.0 OR MIT (registry); MIT OR Apache-2.0 (ash)",
}
MAX_REGISTRY_BYTES = 8 * 1024 * 1024

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()

def signature(element):
    return "".join(element.itertext()).strip()

def api_has(value, api="vulkan"):
    return value is None or api in value.split(",")

def inventory(root):
    owners = {}
    for feature in root.findall("feature"):
        if api_has(feature.get("api")):
            for require in feature.findall("require"):
                if api_has(require.get("api")):
                    for command in require.findall("command"):
                        owners.setdefault(command.get("name"), []).append({"kind":"core", "name":feature.get("name"), "depends":require.get("depends")})
    for extension in root.findall("extensions/extension"):
        if api_has(extension.get("supported")):
            for require in extension.findall("require"):
                if api_has(require.get("api")):
                    for command in require.findall("command"):
                        owners.setdefault(command.get("name"), []).append({"kind":"extension", "name":extension.get("name"), "platform":extension.get("platform"), "depends":require.get("depends") or extension.get("depends")})
    types=[]
    handles={}
    for item in root.findall("types/type"):
        name=item.get("name") or item.findtext("name")
        members=[{"name":m.findtext("name"),"type":m.findtext("type"),"declaration":signature(m),"attributes":dict(m.attrib)} for m in item.findall("member")]
        record={"name":name,"attributes":dict(item.attrib),"declaration":signature(item),"members":members}
        types.append(record)
        if item.get("category")=="handle": handles[name]=item.get("parent")
    commands=[]
    by_name={x.get("name") or x.findtext("proto/name"):x for x in root.findall("commands/command")}
    def resolve(name):
        seen=set()
        while by_name[name].get("alias"):
            if name in seen: raise ValueError("cyclic command alias")
            seen.add(name); name=by_name[name].get("alias")
            if name not in by_name:raise ValueError("unresolved command alias")
        return name
    def dispatch(item):
        first=item.findtext("param/type")
        if not first or first not in handles:return "global"
        seen=set()
        while first:
            if first in ("VkDevice","VkQueue","VkCommandBuffer"):return "device"
            if first in ("VkInstance","VkPhysicalDevice"):return "instance"
            if first in seen:raise ValueError("cyclic handle parent")
            seen.add(first);first=handles.get(first)
        return "global"
    for name,item in sorted(by_name.items()):
        target=resolve(name);resolved=by_name[target]
        commands.append({"name":name,"alias_of":item.get("alias"),"canonical_name":target,"return_type":resolved.findtext("proto/type"),"attributes":dict(item.attrib),"parameters":[{"name":p.findtext("name"),"type":p.findtext("type"),"declaration":signature(p),"attributes":dict(p.attrib)} for p in resolved.findall("param")],"dispatch":dispatch(resolved),"vulkan_required_by":owners.get(name,[]),"generated":None,"loader_support":"not_queried","device_capability":"not_queried","executed":False})
    enums=[{"group":group.get("name"),"group_attributes":dict(group.attrib),"values":[dict(v.attrib) for v in group.findall("enum")]} for group in root.findall("enums")]
    # Extension/core additions carry values, aliases and offset/bitpos formulas.
    additions=[{"owner":owner.get("name"),"requirements":[{"attributes":dict(req.attrib),"enums":[dict(x.attrib) for x in req.findall("enum")],"types":[dict(x.attrib) for x in req.findall("type")]} for req in owner.findall("require")]} for owner in list(root.findall("feature"))+list(root.findall("extensions/extension"))]
    return {"schema":"fluxr-vulkan-registry-v1","pin":PIN,"commands":commands,"types":types,"enums":enums,"requirements":additions,"platforms":[dict(p.attrib) for p in root.findall("platforms/platform")],"extensions":[dict(p.attrib) for p in root.findall("extensions/extension")],"pnext":[{"name":t["name"],"extends":t["attributes"]["structextends"].split(","),"allowduplicate":t["attributes"].get("allowduplicate","false")} for t in types if "structextends" in t["attributes"]]}

def verify_ash(catalog, ash):
    manifest=(ash/"Cargo.toml").read_text()
    if f'version = "{PIN["ash_version"]}"' not in manifest:raise ValueError("wrong ash version")
    files=sorted((ash/"src").rglob("*.rs"))
    sources="\n".join(p.read_text() for p in files)
    pfns=set(re.findall(r"\bPFN_(vk\w+)\b",sources))
    symbols=set(re.findall(r'"(vk[A-Z]\w*)\\0"',sources))
    missing=[]
    for item in catalog["commands"]:
        generated=item["canonical_name"] in pfns and item["name"] in symbols
        item["generated"]={"canonical_pfn":item["canonical_name"] in pfns,"loader_symbol":item["name"] in symbols,"covered":generated}
        if item["vulkan_required_by"] and not generated:missing.append(item["name"])
    return {"ash_version":PIN["ash_version"],"ash_source_sha256":hashlib.sha256(canonical([{ "path":str(p.relative_to(ash)),"sha256":hashlib.sha256(p.read_bytes()).hexdigest()} for p in files])).hexdigest(),"required_vulkan_commands":sum(bool(c["vulkan_required_by"]) for c in catalog["commands"]),"missing_required_commands":missing,"method":"canonical PFN identifier plus exact null-terminated loader symbol; alias and requirement closure from pinned XML","scope":"command presence and complete registry metadata; not ABI equivalence proof, runtime loading, enabled features, platform execution, or pNext validity proof"}

def load_registry(path):
    with path.open("rb") as stream:data=stream.read(MAX_REGISTRY_BYTES+1)
    if len(data)>MAX_REGISTRY_BYTES:raise ValueError("registry byte budget exceeded")
    if hashlib.sha256(data).hexdigest()!=PIN["sha256"]:raise ValueError("pinned registry checksum mismatch")
    if b"<!DOCTYPE" in data or b"<!ENTITY" in data:raise ValueError("entity declarations refused")
    return ET.fromstring(data)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry",required=True,type=Path)
    parser.add_argument("--fetch",action="store_true")
    parser.add_argument("--ash-source",required=True,type=Path)
    parser.add_argument("--output",required=True,type=Path)
    parser.add_argument("--catalog-output",type=Path)
    args=parser.parse_args()
    if args.fetch:
        if args.registry.exists():raise ValueError("acquisition destination exists")
        with urllib.request.urlopen(PIN["url"],timeout=30) as response:data=response.read(MAX_REGISTRY_BYTES+1)
        if len(data)>MAX_REGISTRY_BYTES or hashlib.sha256(data).hexdigest()!=PIN["sha256"]:raise ValueError("acquisition checksum or size mismatch")
        args.registry.write_bytes(data)
    catalog=inventory(load_registry(args.registry));report=verify_ash(catalog,args.ash_source)
    report.update(schema="fluxr-vulkan-coverage-v1",pin=PIN,all_registry_commands=len(catalog["commands"]),retained_non_vulkan_commands=sum(not c["vulkan_required_by"] for c in catalog["commands"]),type_records=len(catalog["types"]),enum_groups=len(catalog["enums"]),platforms=len(catalog["platforms"]),pnext_structures=len(catalog["pnext"]),catalog_sha256=hashlib.sha256(canonical(catalog)).hexdigest(),executed_call_coverage="separate execution probe; no all-call execution claim")
    if args.catalog_output:args.catalog_output.write_bytes(canonical(catalog)+b"\n")
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps(report,indent=2))
    return bool(report["missing_required_commands"])
if __name__=="__main__":raise SystemExit(main())
