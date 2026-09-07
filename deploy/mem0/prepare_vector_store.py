"""Remove the unused upstream backend before installing or copying runtime files."""
from pathlib import Path
import sys


def prepare(root):
    def replace(relative, before, after, count=1):
        path = root / relative
        source = path.read_text()
        if source.count(before) != count:
            raise RuntimeError(f"Pinned upstream changed: {relative}")
        path.write_text(source.replace(before, after))

    replace("main.py", '''POSTGRES_HOST = os.environ.get("POSTGRES_HOST", "postgres")
POSTGRES_PORT = os.environ.get("POSTGRES_PORT", "5432")
POSTGRES_DB = os.environ.get("POSTGRES_DB", "postgres")
POSTGRES_USER = os.environ.get("POSTGRES_USER", "postgres")
POSTGRES_PASSWORD = os.environ.get("POSTGRES_PASSWORD", "postgres")
POSTGRES_COLLECTION_NAME = os.environ.get("POSTGRES_COLLECTION_NAME", "memories")

''', "")
    replace("main.py", '''        "provider": "pgvector",
        "config": {
            "host": POSTGRES_HOST,
            "port": int(POSTGRES_PORT),
            "dbname": POSTGRES_DB,
            "user": POSTGRES_USER,
            "password": POSTGRES_PASSWORD,
            "collection_name": POSTGRES_COLLECTION_NAME,
        },''', '''        "provider": "elasticsearch",
        "config": {},''')
    replace("server_state.py", "        _memory_instance = Memory.from_config(_current_config)",
            '''        if _current_config.get("vector_store", {}).get("provider") != "elasticsearch":
            raise RuntimeError("Only Elasticsearch is supported")
        _memory_instance = Memory.from_config(_current_config) if overrides else None''')
    replace("server_state.py", "        next_config = _merge_config(_current_config, updates)",
            '''        next_config = _merge_config(_current_config, updates)
        if next_config.get("vector_store", {}).get("provider") != "elasticsearch":
            raise ValueError("Only Elasticsearch is supported")''')
    replace("packages/mem0/utils/factory.py",
            '        "pgvector": "mem0.vector_stores.pgvector.PGVector",\n', "")
    replace("packages/mem0/vector_stores/configs.py",
            '        "pgvector": "PGVectorConfig",\n', "")
    replace("packages/mem0/memory/main.py", "qdrant, elasticsearch, pgvector",
            "qdrant, elasticsearch", count=2)
    for relative in ("vector_stores/pgvector.py", "configs/vector_stores/pgvector.py"):
        (root / "packages/mem0" / relative).unlink()
    # psycopg remains in the server requirements for authentication/settings SQL.
    replace("requirements.txt", "psycopg-pool>=3.2.6,<4.0.0\n", "")
    replace("requirements.txt", "mem0ai>=0.1.48\n", "")
    replace("packages/pyproject.toml", '    "psycopg-pool>=3.2.6,<4.0.0",\n', "")
    replace("packages/pyproject.toml", '    "psycopg>=3.2.8",\n', "")
    # Fail the build if future upstream packaging would reintroduce the backend.
    for path in root.rglob("*"):
        if path.is_file() and ("pgvector" in path.name.lower()
                              or b"pgvector" in path.read_bytes().lower()):
            raise RuntimeError(f"Removed backend remains in prepared source: {path.relative_to(root)}")


if __name__ == "__main__":
    prepare(Path(sys.argv[1]))
