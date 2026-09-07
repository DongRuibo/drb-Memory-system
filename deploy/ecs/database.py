"""Administrator-only native migration; runtime checks never change schema."""
import json
import os
from pathlib import Path
import subprocess
import sys

from sqlalchemy import inspect, text
import storage

DATABASE = "lazytt_memory_mem0"
OWNER = "lazytt_memory_mem0_owner"
RUNTIME = "lazytt_memory_mem0_runtime"
HEAD = "006"
COMMENTS = json.loads(Path(__file__).with_name("comments.json").read_text())


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def identity(conn, migration=False):
    role = OWNER if migration else RUNTIME
    require(conn.execute(text("SELECT current_database(), current_user")).one() == (DATABASE, role),
            "Wrong canonical database or role")
    require(conn.scalar(text("SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()")) == OWNER,
            "Wrong database owner")
    for name in (OWNER, RUNTIME):
        flags = conn.execute(text("SELECT rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls FROM pg_roles WHERE rolname=:role"), {"role": name}).one()
        require(not any(flags), "Elevated role flags")
        require(not conn.scalar(text("SELECT count(*) FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.member WHERE r.rolname=:role"), {"role": name}),
                "Unexpected role membership")
    require(not conn.scalar(text("SELECT has_schema_privilege(:role, 'public', 'CREATE')"), {"role": RUNTIME}),
            "Runtime may create schema objects")
    require(not conn.scalar(text("SELECT has_database_privilege(:role, current_database(), 'TEMP')"), {"role": RUNTIME}),
            "Runtime may create temporary tables")
    other = conn.execute(text("SELECT datname FROM pg_database WHERE NOT datistemplate AND datname != current_database() AND has_database_privilege(:role, oid, 'CONNECT')"), {"role": RUNTIME}).all()
    require(not other, "Cross-database CONNECT remains; coordinate shared ACL with Store administrator")


def verify(conn):
    require(conn.execute(text("SELECT version_num FROM public.alembic_version")).scalars().all() == [HEAD],
            "Unexpected Alembic head")
    inspector = inspect(conn)
    require(set(inspector.get_table_names(schema="public")) == set(COMMENTS), "Unexpected table set")
    # Native model columns, nullability and PK/FK definitions are checked separately from comments.
    expected_schema = json.loads(Path(__file__).with_name("schema.json").read_text())
    for table, expected in expected_schema.items():
        actual = {c["name"]: c for c in inspector.get_columns(table, schema="public")}
        require(set(actual) == set(expected["columns"]), "Column set mismatch")
        for name, column in expected["columns"].items():
            require(actual[name]["nullable"] == column["nullable"], "Column nullability mismatch")
            require(str(actual[name]["type"].compile(dialect=conn.dialect)) == column["type"],
                    "Column type mismatch: " + table + "." + name)
        require(set(inspector.get_pk_constraint(table)["constrained_columns"]) == set(expected["pk"]), "Primary key mismatch")
        found = {(fk["constrained_columns"][0], fk["referred_table"], fk["referred_columns"][0]) for fk in inspector.get_foreign_keys(table)}
        require({tuple(fk) for fk in expected["fk"]} == found, "Foreign key mismatch")
        require(all(fk.get("options", {}).get("ondelete") == "CASCADE" for fk in inspector.get_foreign_keys(table)),
                "Foreign key delete rule mismatch")
    require(any(item["column_names"] == ["email"] for item in inspector.get_unique_constraints("users")),
            "Missing email uniqueness constraint")
    indexes = {row[0]: row[1] for row in conn.execute(text("SELECT indexname, indexdef FROM pg_indexes WHERE schemaname='public'"))}
    require("USING brin (created_at)" in indexes.get("ix_request_logs_created_at", ""), "Missing BRIN index")
    require("UNIQUE INDEX" in indexes.get("ix_users_only_one_admin", "") and "WHERE" in indexes["ix_users_only_one_admin"], "Missing admin uniqueness constraint")
    require(not conn.scalar(text("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='i' AND obj_description(c.oid, 'pg_class') IS NULL")),
            "Missing index comments")
    require(not conn.scalar(text("SELECT count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public' AND obj_description(c.oid, 'pg_constraint') IS NULL")),
            "Missing constraint comments")
    for table, description in COMMENTS.items():
        require(conn.scalar(text("SELECT pg_get_userbyid(relowner) FROM pg_class WHERE oid=to_regclass(:name)"), {"name": "public." + table}) == OWNER, "Wrong table owner")
        require(inspector.get_table_comment(table)["text"] == description["description"], "Missing table comment")
        for column in inspector.get_columns(table):
            require(column["comment"] == description["columns"].get(column["name"]), "Missing column comment")
        for privilege in ("SELECT", "INSERT", "UPDATE", "DELETE"):
            # Version metadata is read-only for the application.
            expected = table != "alembic_version" or privilege == "SELECT"
            require(conn.scalar(text("SELECT has_table_privilege(:role, :table, :privilege)"),
                                {"role": RUNTIME, "table": "public." + table, "privilege": privilege}) == expected, "Runtime table privilege mismatch")


def annotate(conn):
    for table, description in COMMENTS.items():
        # All identifiers and comments come from the versioned, non-secret dictionary.
        quote = lambda value: "'" + value.replace("'", "''") + "'"
        conn.exec_driver_sql(f'COMMENT ON TABLE public."{table}" IS {quote(description["description"])}')
        for column, comment in description["columns"].items():
            conn.exec_driver_sql(f'COMMENT ON COLUMN public."{table}"."{column}" IS {quote(comment)}')
        conn.exec_driver_sql(f'REVOKE ALL ON public."{table}" FROM PUBLIC, {RUNTIME}')
        privileges = "SELECT" if table == "alembic_version" else "SELECT, INSERT, UPDATE, DELETE"
        conn.exec_driver_sql(f'GRANT {privileges} ON public."{table}" TO {RUNTIME}')
    for name, definition in conn.execute(text("SELECT indexname, indexdef FROM pg_indexes WHERE schemaname='public'")):
        comment = "原生迁移索引；用于唯一性约束或查询加速。定义：" + definition
        conn.exec_driver_sql(f'COMMENT ON INDEX public."{name}" IS {quote(comment)}')
    for table, name, definition in conn.execute(text("SELECT c.relname, con.conname, pg_get_constraintdef(con.oid) FROM pg_constraint con JOIN pg_class c ON c.oid=con.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'")):
        conn.exec_driver_sql(f'COMMENT ON CONSTRAINT "{name}" ON public."{table}" IS {quote("原生迁移完整性约束：" + definition)}')


def main(action):
    engine = storage.engine()
    with engine.connect() as conn:
        identity(conn, migration=action == "migrate")
        if action != "migrate":
            verify(conn)
    if action == "migrate":
        # Runs upstream revisions, never stamps a hand-built schema or runs DOWN.
        result = subprocess.run(["alembic", "-c", "/opt/lazytt/alembic.ini", "upgrade", HEAD], cwd="/app", capture_output=True)
        require(result.returncode == 0, "Native Alembic failed; preserve database and inspect history privately")
        with engine.begin() as conn:
            annotate(conn)
            verify(conn)
    with engine.connect() as conn:
        conn.exec_driver_sql(Path(__file__).with_name("03_verify.sql").read_text())
    print("MEMORY_DATABASE_OK head=" + HEAD)


if __name__ == "__main__":
    try:
        require(len(sys.argv) == 2 and sys.argv[1] in {"check", "migrate"}, "Expected check or migrate")
        main(sys.argv[1])
    except Exception as error:
        # Only our bounded errors are printable; driver errors can contain DSNs/SQL values.
        print(str(error) if type(error) is RuntimeError else "MEMORY_DATABASE_FAILED: inspect database privately", file=sys.stderr)
        sys.exit(1)
