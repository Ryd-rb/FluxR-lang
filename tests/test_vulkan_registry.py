import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
spec=importlib.util.spec_from_file_location("registry",Path(__file__).resolve().parents[1]/"tools/vulkan_registry.py")
registry=importlib.util.module_from_spec(spec);spec.loader.exec_module(registry)

XML='''<registry><platforms><platform name="wayland" protect="VK_USE_PLATFORM_WAYLAND_KHR"/></platforms><types><type category="handle"><type>VK_DEFINE_HANDLE</type>(<name>VkInstance</name>)</type><type category="handle" parent="VkInstance"><type>VK_DEFINE_HANDLE</type>(<name>VkDevice</name>)</type><type category="struct" name="VkExtra" structextends="VkBase"><member optional="true"><type>void</type>* <name>pNext</name></member></type></types><commands><command><proto><type>void</type> <name>vkA</name></proto><param><type>VkDevice</type> <name>d</name></param></command><command name="vkAlias" alias="vkA"/><command><proto><type>void</type> <name>vkSC</name></proto></command></commands><feature api="vulkan" name="VK_VERSION_1_0"><require><command name="vkA"/></require></feature><extensions><extension name="VK_EXT_a" supported="vulkan" platform="wayland"><require><command name="vkAlias"/><enum name="VK_SOMETHING_EXT" offset="1"/></require></extension><extension name="VK_SC_a" supported="vulkansc"><require><command name="vkSC"/></require></extension></extensions></registry>'''
class RegistryTests(unittest.TestCase):
    def test_alias_dispatch_platform_and_pnext_preserved(self):
        result=registry.inventory(ET.fromstring(XML));commands={c['name']:c for c in result['commands']}
        self.assertEqual(commands['vkAlias']['canonical_name'],'vkA')
        self.assertEqual(commands['vkAlias']['dispatch'],'device')
        self.assertEqual(commands['vkAlias']['vulkan_required_by'][0]['platform'],'wayland')
        self.assertEqual(commands['vkSC']['vulkan_required_by'],[])
        self.assertEqual(result['pnext'][0]['extends'],['VkBase'])
        self.assertEqual(result['requirements'][1]['requirements'][0]['enums'][0]['offset'],'1')
    def test_alias_cycle_refused(self):
        root=ET.fromstring(XML);root.find('commands/command').set('alias','vkAlias')
        with self.assertRaises(ValueError):registry.inventory(root)
    def test_changed_registry_refused_before_parsing(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'vk.xml';path.write_text(XML)
            with self.assertRaisesRegex(ValueError,'checksum'):registry.load_registry(path)
    def test_canonical_pfn_alone_does_not_claim_alias_loading(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp);(path/'src').mkdir();(path/'Cargo.toml').write_text('version = "0.38.0+1.3.281"')
            (path/'src/lib.rs').write_text('type PFN_vkA = (); let name = b"vkA\\0";')
            report=registry.verify_ash(registry.inventory(ET.fromstring(XML)),path)
            self.assertEqual(report['missing_required_commands'],['vkAlias'])
if __name__=='__main__':unittest.main()
