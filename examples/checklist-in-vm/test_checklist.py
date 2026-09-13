import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


class ChecklistTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.f = str(Path(self.tmp.name) / "cl.json")

    def cli(self, *args):
        return subprocess.run(
            [sys.executable, "checklist.py", "--file", self.f, *args],
            capture_output=True,
            text=True,
        )

    def test_add_and_list(self):
        r = self.cli("add", "Buy", "milk")
        self.assertEqual(r.returncode, 0)
        self.assertIn("1: Buy milk", r.stdout)

        r = self.cli("list")
        self.assertEqual(r.returncode, 0)
        self.assertIn("[ ] 1: Buy milk", r.stdout)
        self.assertIn("0/1 done", r.stdout)

    def test_done_persists_to_file(self):
        self.cli("add", "Walk", "dog")
        r = self.cli("done", "1")
        self.assertEqual(r.returncode, 0)

        r = self.cli("list")
        self.assertIn("[x] 1: Walk dog", r.stdout)
        self.assertIn("1/1 done", r.stdout)

        # the state is really on disk, not just in memory
        with open(self.f, encoding="utf-8") as fh:
            data = json.load(fh)
        self.assertIs(data["items"][0]["done"], True)

    def test_undone(self):
        self.cli("add", "x")
        self.cli("done", "1")
        r = self.cli("undone", "1")
        self.assertEqual(r.returncode, 0)
        r = self.cli("list")
        self.assertIn("[ ] 1: x", r.stdout)

    def test_remove_and_clear(self):
        self.cli("add", "a")
        self.cli("add", "b")
        self.cli("done", "1")

        r = self.cli("clear")  # removes done items only
        self.assertEqual(r.returncode, 0)
        r = self.cli("list")
        self.assertNotIn("1: a", r.stdout)
        self.assertIn("2: b", r.stdout)

        r = self.cli("remove", "2")
        self.assertEqual(r.returncode, 0)
        r = self.cli("list")
        self.assertIn("empty", r.stdout)

    def test_clear_all(self):
        self.cli("add", "a")
        r = self.cli("clear", "--all")
        self.assertEqual(r.returncode, 0)
        r = self.cli("list")
        self.assertIn("empty", r.stdout)

    def test_bad_id_fails(self):
        r = self.cli("done", "99")
        self.assertEqual(r.returncode, 1)
        self.assertIn("no item", r.stderr)

    def test_ids_keep_counting_after_remove(self):
        self.cli("add", "a")
        self.cli("add", "b")
        self.cli("remove", "1")
        r = self.cli("add", "c")
        self.assertIn("3: c", r.stdout)


if __name__ == "__main__":
    unittest.main()
