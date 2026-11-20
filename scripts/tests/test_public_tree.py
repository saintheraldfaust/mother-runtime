import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GUARD = Path(__file__).resolve().parents[1] / 'check_public_tree.py'


class PublicationGuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.git('init', '-b', 'main')
        (self.root / '.gitignore').write_text('MOTHER.md\n.mother-private/\n')
        (self.root / 'MOTHER.md').write_text('private fixture directive; never published\n')

    def tearDown(self):
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.name=Fixture', '-c',
                               'user.email=fixture@example.invalid', *args], cwd=self.root,
                              check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def guard(self):
        return subprocess.run([sys.executable, str(GUARD)], cwd=self.root,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def test_public_source_passes_with_private_local_files_present(self):
        (self.root / 'README.md').write_text('Public project documentation\n')
        (self.root / '.mother-private').mkdir()
        (self.root / '.mother-private' / 'notes.md').write_text('private fixture notes')
        self.git('add', '.gitignore', 'README.md')
        self.git('commit', '-m', 'docs: public fixture')
        self.assertEqual(self.guard().returncode, 0)

    def test_force_staged_private_document_is_rejected(self):
        self.git('add', '-f', 'MOTHER.md')
        self.assertNotEqual(self.guard().returncode, 0)

    def test_renamed_private_content_is_rejected(self):
        (self.root / 'innocent.md').write_bytes((self.root / 'MOTHER.md').read_bytes())
        self.git('add', 'innocent.md')
        self.assertNotEqual(self.guard().returncode, 0)

    def test_deletion_commit_does_not_hide_private_history(self):
        self.git('add', '-f', 'MOTHER.md')
        self.git('commit', '-m', 'test: private fixture in isolated history')
        self.git('rm', '--cached', 'MOTHER.md')
        self.git('commit', '-m', 'test: exclude local fixture')
        self.assertTrue((self.root / 'MOTHER.md').exists())
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'history', result.stderr)


if __name__ == '__main__':
    unittest.main()
