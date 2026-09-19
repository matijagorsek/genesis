"""The template's own test: it checks the program still starts and answers --help, nothing about what it
does. The job asked for decides the real assertions, so add them here as you implement it — a test that
encodes the template's placeholder behaviour fails the moment the program becomes something real."""
import subprocess, sys


def test_it_starts():
    r = subprocess.run([sys.executable, "{{name}}.py", "--help"], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip(), "--help should say something"
