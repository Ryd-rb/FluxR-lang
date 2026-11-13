"""Execution-bound regressions for the trusted native fixture runner."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT=Path(__file__).resolve().parents[1]/"tools/qualify_native_domains.py"

class QualificationBounds(unittest.TestCase):
    def test_nonfinite_and_nonpositive_deadlines_refuse_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for index,value in enumerate(["nan","inf","-inf","0","-1"]):
                with self.subTest(deadline=value):
                    output=root/f"run{index}"
                    result=subprocess.run([sys.executable,str(SCRIPT),"--binary",str(root/"never-run"),"--output",str(output),"--timeout-seconds="+value],capture_output=True,text=True,timeout=5)
                    self.assertEqual(result.returncode,2)
                    self.assertIn("finite and positive",result.stderr)
                    self.assertFalse(output.exists())

if __name__=="__main__":unittest.main()
