"""Run unchanged upstream revisions with the project's safe connection settings."""
from alembic import context
import storage

with storage.engine().connect() as connection:
    context.configure(connection=connection)
    with context.begin_transaction():
        context.run_migrations()
