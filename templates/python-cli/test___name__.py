import subprocess, sys

def test_runs():
    r = subprocess.run([sys.executable, "{{name}}.py", "a", "b", "--json"], capture_output=True, text=True)
    assert r.returncode == 0 and '"count": 2' in r.stdout
