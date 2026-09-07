"""Build-time schema expectations from unchanged upstream models; no DB connection."""
import json
from pathlib import Path
from sqlalchemy.dialects.postgresql import dialect
import models

result = {}
for table in models.Base.metadata.sorted_tables:
    result[table.name] = {
        "columns": {column.name: {"nullable": column.nullable,
            "type": str(column.type.compile(dialect=dialect())).replace("FLOAT", "DOUBLE PRECISION")}
            for column in table.columns},
        "pk": [column.name for column in table.primary_key],
        "fk": [[fk.parent.name, fk.column.table.name, fk.column.name] for fk in table.foreign_keys],
    }
Path("/opt/lazytt/schema.json").write_text(json.dumps(result))
