"""Compare packaged upstream Python files against the fixed source checkout."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def main(image, source):
    expected = {}
    for path in (source / "mem0").rglob("*.py"):
        expected["mem0/" + str(path.relative_to(source / "mem0"))] = hashlib.sha256(path.read_bytes()).hexdigest()
    server = source / "server"
    files = list(server.glob("*.py"))
    for directory in ("routers", "alembic"):
        files.extend((server / directory).rglob("*.py"))
    for path in files:
        expected["server/" + str(path.relative_to(server))] = hashlib.sha256(path.read_bytes()).hexdigest()
    script = '''
import hashlib, importlib.util, json
from pathlib import Path
root = Path(importlib.util.find_spec("mem0").origin).parent
actual = {"mem0/" + str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
          for p in root.rglob("*.py")}
root = Path("/app")
files = list(root.glob("*.py"))
for directory in ("routers", "alembic"):
    files.extend((root / directory).rglob("*.py"))
actual.update({"server/" + str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
               for p in files})
print(json.dumps(actual))
'''
    result = subprocess.run(["docker", "run", "--rm", "--network", "none", "--entrypoint", "python", "-i", image],
                            input=script, text=True, capture_output=True, check=True)
    actual = json.loads(result.stdout)
    if actual != expected:
        raise RuntimeError("Packaged Mem0 source differs from the pinned upstream")
    print(f"MEM0_UPSTREAM_SOURCE_OK: {len(expected)} unchanged Python files")


if __name__ == "__main__":
    main(sys.argv[1], Path(sys.argv[2]))
