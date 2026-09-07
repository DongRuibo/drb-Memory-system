"""Small, asserted patches for the pinned upstream server only."""
from pathlib import Path


def replace(path, before, after):
    file = Path(path)
    source = file.read_text()
    if source.count(before) != 1:
        raise RuntimeError(f"Pinned upstream changed: {path}")
    file.write_text(source.replace(before, after))


replace("/app/db.py", 'return f"postgresql+psycopg://{user}:{password}@{host}:{port}/{db}"',
        'return __import__("sqlalchemy").URL.create("postgresql+psycopg", user, password, host, int(port), db).render_as_string(hide_password=False)')
replace("/app/db.py", "create_engine(_build_database_url(), pool_pre_ping=True)",
        "create_engine(_build_database_url(), pool_pre_ping=True, pool_size=3, max_overflow=2)")
replace("/app/alembic/env.py", 'config.set_main_option("sqlalchemy.url", _build_database_url())',
        'config.set_main_option("sqlalchemy.url", _build_database_url().replace("%", "%%"))')
# With an empty settings table expose authenticated configuration first. Do not
# construct the upstream pgvector default with a DML-only PostgreSQL role.
replace("/app/server_state.py", "_memory_instance = Memory.from_config(_current_config)",
        "_memory_instance = Memory.from_config(_current_config) if overrides else None")
replace("/app/server_state.py", 'logging.warning("Failed to persist config overrides to database", exc_info=True)',
        'raise RuntimeError("Failed to persist config overrides") from None')
