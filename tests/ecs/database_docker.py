#!/usr/bin/env python3
"""Real, isolated PostgreSQL test. No host ports, models, production data or credentials."""
import os
import re
from pathlib import Path
import secrets
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]


def run(command, env=None, data=None, success=True):
    result = subprocess.run(command, env=env, input=data, text=True, capture_output=True)
    if (result.returncode == 0) != success:
        if "/opt/lazytt/configuration.py" in command:
            for line in (result.stdout + result.stderr).splitlines():
                if re.fullmatch(r"MEM0_CONFIGURATION_STAGE: [a-z_]+|MEM0_CONFIGURATION_FAILED: [A-Za-z]+ http_status=[0-9]+", line):
                    print(line)
        # These tasks may contain disposable passwords in diagnostics; don't print raw output.
        print(result.stdout if command[-1] in {"check", "migrate"} else "", end="")
        if command[-1] in {"check", "migrate"}:
            print(result.stderr, end="")
        raise RuntimeError("Unexpected container test result: " + command[0] + f" (exit {result.returncode})")
    return result.stdout


def main(image, database_only=False):
    name = "memory-ecs-test-" + uuid.uuid4().hex[:12]
    pg = name + "-pg"
    admin = name + "-admin"
    mem0 = name + "-mem0"
    elasticsearch = name + "-es"
    model = name + "-model"
    # Unit-level failures are exercised in the same packaged Python environment.
    run(["docker", "run", "--rm", "--network", "none", "--entrypoint", "python", "-i",
         "-e", "POSTGRES_HOST=unused", "-e", "POSTGRES_PORT=5432", "-e", "POSTGRES_USER=fixture",
         "-e", "POSTGRES_PASSWORD=fixture", "-e", "APP_DB_NAME=lazytt_memory_mem0",
         "-e", "ADMIN_API_KEY=" + "f" * 40, image, "-"], data=(ROOT / "tests/ecs/gateway_unit.py").read_text())
    print("GATEWAY_FAILURE_TESTS_OK")
    owner_password = secrets.token_hex(24) + "@:%/"
    runtime_password = secrets.token_hex(24) + "@:%/"
    pg_env = {**os.environ, "POSTGRES_PASSWORD": secrets.token_hex(32)}
    run(["docker", "network", "create", "--internal", "--label", "io.lazytt.test=" + name, name])
    try:
        run(["docker", "run", "-d", "--name", pg, "--label", "io.lazytt.test=" + name,
             "--network", name, "--tmpfs", "/var/lib/postgresql", "-e", "POSTGRES_PASSWORD",
             "postgres:18.4-bookworm"], env=pg_env)
        for _ in range(30):
            result = subprocess.run(["docker", "exec", pg, "pg_isready", "-U", "postgres"], capture_output=True)
            if result.returncode == 0:
                break
            time.sleep(1)
        else:
            raise RuntimeError("Temporary PostgreSQL did not start")
        def sql(statement):
            return run(["docker", "exec", "-i", pg, "psql", "-U", "postgres", "-v", "ON_ERROR_STOP=1"], data=statement)
        sql((ROOT / "deploy/ecs/database/01_roles.sql").read_text())
        sql("CREATE DATABASE lazytt_memory_mem0 OWNER lazytt_memory_mem0_owner;\n"
            "REVOKE ALL ON DATABASE lazytt_memory_mem0 FROM PUBLIC;\n"
            "GRANT CONNECT ON DATABASE lazytt_memory_mem0 TO lazytt_memory_mem0_runtime;\n"
            "REVOKE CONNECT ON DATABASE postgres FROM PUBLIC;\n"
            "\\connect lazytt_memory_mem0\n"
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC;\n"
            "GRANT USAGE ON SCHEMA public TO lazytt_memory_mem0_runtime;\n"
            f"ALTER ROLE lazytt_memory_mem0_owner PASSWORD '{owner_password}';\n"
            f"ALTER ROLE lazytt_memory_mem0_runtime PASSWORD '{runtime_password}';\n")
        def task(action, owner=False, success=True):
            values = {**os.environ, "POSTGRES_HOST": pg, "POSTGRES_PORT": "5432",
                      "POSTGRES_DB": "lazytt_memory_mem0", "APP_DB_NAME": "lazytt_memory_mem0",
                      "POSTGRES_USER": "lazytt_memory_mem0_owner" if owner else "lazytt_memory_mem0_runtime",
                      "POSTGRES_PASSWORD": owner_password if owner else runtime_password}
            command = ["docker", "run", "--rm", "--name", admin, "--label", "io.lazytt.test=" + name,
                       "--network", name, "--entrypoint", "python"]
            for key in ("POSTGRES_HOST", "POSTGRES_PORT", "POSTGRES_DB", "APP_DB_NAME", "POSTGRES_USER", "POSTGRES_PASSWORD"):
                command.extend(["-e", key])
            return run(command + [image, "/opt/lazytt/database.py", action], env=values, success=success)
        task("check", success=False)
        task("migrate", owner=True)
        task("check")
        # Each drift is committed only inside this disposable database and then repaired.
        # Re-annotate restored objects; migrations remain the unchanged native upgrade path.
        for label, broken, repaired in (
            ("email index", "DROP INDEX ix_users_email", "CREATE INDEX ix_users_email ON users(email)"),
            ("expiry index", "DROP INDEX ix_refresh_token_jtis_expires_at", "CREATE INDEX ix_refresh_token_jtis_expires_at ON refresh_token_jtis(expires_at)"),
            ("server default", "ALTER TABLE users ALTER COLUMN role SET DEFAULT 'member'", "ALTER TABLE users ALTER COLUMN role SET DEFAULT 'admin'"),
            ("missing default", "ALTER TABLE settings ALTER COLUMN updated_at DROP DEFAULT", "ALTER TABLE settings ALTER COLUMN updated_at SET DEFAULT now()"),
            ("partial predicate", "DROP INDEX ix_users_only_one_admin; CREATE UNIQUE INDEX ix_users_only_one_admin ON users(role) WHERE role='member'",
             "DROP INDEX ix_users_only_one_admin; CREATE UNIQUE INDEX ix_users_only_one_admin ON users(role) WHERE role='admin'"),
            ("foreign key", "ALTER TABLE api_keys DROP CONSTRAINT api_keys_created_by_fkey; ALTER TABLE api_keys ADD CONSTRAINT api_keys_created_by_fkey FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE RESTRICT",
             "ALTER TABLE api_keys DROP CONSTRAINT api_keys_created_by_fkey; ALTER TABLE api_keys ADD CONSTRAINT api_keys_created_by_fkey FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE CASCADE"),
            ("unvalidated foreign key", "ALTER TABLE refresh_token_jtis DROP CONSTRAINT refresh_token_jtis_user_id_fkey; ALTER TABLE refresh_token_jtis ADD CONSTRAINT refresh_token_jtis_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE NOT VALID",
             "ALTER TABLE refresh_token_jtis VALIDATE CONSTRAINT refresh_token_jtis_user_id_fkey"),
            ("extra index", "CREATE INDEX unexpected_index ON settings(value); COMMENT ON INDEX unexpected_index IS 'fixture'", "DROP INDEX unexpected_index"),
            ("ledger type", "ALTER TABLE alembic_version ALTER COLUMN version_num TYPE varchar(64)", "ALTER TABLE alembic_version ALTER COLUMN version_num TYPE varchar(32)"),
            ("role limit", "ALTER ROLE lazytt_memory_mem0_runtime CONNECTION LIMIT -1", "ALTER ROLE lazytt_memory_mem0_runtime CONNECTION LIMIT 8"),
            ("owner cross-db", "GRANT CONNECT ON DATABASE postgres TO lazytt_memory_mem0_owner", "REVOKE CONNECT ON DATABASE postgres FROM lazytt_memory_mem0_owner"),
        ):
            sql("\\connect lazytt_memory_mem0\n" + broken + ";\n")
            task("check", success=False)
            sql("\\connect lazytt_memory_mem0\n" + repaired + ";\n")
            task("migrate", owner=True)
            task("check")
            print("SCHEMA_DRIFT_REJECTED: " + label)
        task("migrate", owner=True)  # Native Alembic no-op; no duplicate seeds.
        task("migrate", success=False)  # Runtime role cannot act as migration owner.
        sql("GRANT CONNECT ON DATABASE postgres TO lazytt_memory_mem0_runtime;\n")
        task("check", success=False)
        sql("REVOKE CONNECT ON DATABASE postgres FROM lazytt_memory_mem0_runtime;\n")
        sql("\\connect lazytt_memory_mem0\nCOMMENT ON COLUMN public.settings.value IS NULL;\n")
        task("check", success=False)
        task("migrate", owner=True)
        task("check")
        run(["docker", "exec", "-i", pg, "psql", "-U", "postgres", "-d", "lazytt_memory_mem0", "-v", "ON_ERROR_STOP=1"],
            data="SET ROLE lazytt_memory_mem0_runtime; CREATE TABLE forbidden(id int);", success=False)
        print("DATABASE_INTEGRATION_OK: native migrations, repeat, comments, low privilege and cross-db rejection")
        if database_only:
            return
        # This network has no external route. Client constructors/configuration must not
        # require a paid Provider call; empty settings expose configuration only.
        run(["docker", "run", "-d", "--name", elasticsearch, "--label", "io.lazytt.test=" + name,
             "--network", name, "--memory", "1g", "-e", "discovery.type=single-node",
             "-e", "xpack.security.enabled=false", "-e", "ES_JAVA_OPTS=-Xms512m -Xmx512m",
             "elasticsearch:8.11.3"])
        for _ in range(60):
            probe = subprocess.run(["docker", "exec", elasticsearch, "curl", "--fail", "--silent", "http://127.0.0.1:9200"], capture_output=True)
            if probe.returncode == 0:
                break
            time.sleep(1)
        else:
            raise RuntimeError("Temporary Elasticsearch did not start")
        values = {"POSTGRES_HOST": pg, "POSTGRES_PORT": "5432", "POSTGRES_DB": "lazytt_memory_mem0",
                  "APP_DB_NAME": "lazytt_memory_mem0", "POSTGRES_USER": "lazytt_memory_mem0_runtime",
                  "POSTGRES_PASSWORD": runtime_password, "MEM0_TELEMETRY": "false",
                  "ADMIN_API_KEY": secrets.token_hex(32),
                  "OPENAI_API_KEY": "fixture-not-a-real-key", "HISTORY_DB_PATH": "/tmp/history.db"}
        args = ["docker", "run", "-d", "--name", mem0, "--label", "io.lazytt.test=" + name, "--network", name]
        for key in values:
            args.extend(["-e", key])
        run(args + [image, "uvicorn", "gateway:app", "--host", "0.0.0.0", "--port", "8000"], env={**os.environ, **values})
        def ready():
            for _ in range(60):
                probe = subprocess.run(["docker", "exec", mem0, "python", "-c",
                    "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/auth/setup-status', timeout=2).read()"], capture_output=True)
                if probe.returncode == 0:
                    return
                time.sleep(1)
            raise RuntimeError("Temporary Mem0 did not become ready")
        ready()
        run(["docker", "exec", "-i", mem0, "python", "-"], data='''
import urllib.request, urllib.error, json
assert json.load(urllib.request.urlopen('http://127.0.0.1:8000/auth/setup-status'))['configured'] is False
try:
    urllib.request.urlopen('http://127.0.0.1:8000/health/ready')
except urllib.error.HTTPError as error:
    assert error.code == 503
else:
    raise AssertionError('Empty settings must not report business readiness')
''')
        run(["docker", "run", "-d", "--name", model, "--label", "io.lazytt.test=" + name,
             "--network", name, "--network-alias", "retrieval-models-cpu", "--entrypoint", "python", image,
             "-c", (ROOT / "tests/ecs/model_fixture.py").read_text()])
        config = {**values, "MEM0_BASE_URL": "http://" + mem0 + ":8000", "MEM0_API_KEY": values["ADMIN_API_KEY"],
                  "MEM0_VECTOR_STORE_PROVIDER": "elasticsearch", "MEM0_ELASTICSEARCH_HOST": "http://" + elasticsearch,
                  "MEM0_ELASTICSEARCH_PORT": "9200", "MEM0_ELASTICSEARCH_USER": "fixture",
                  "MEM0_ELASTICSEARCH_PASSWORD": "fixture", "MEM0_COLLECTION_NAME": "test-memories",
                  "EMBEDDING_DIMENSIONS": "16", "EMBEDDING_BASE_URL": "http://retrieval-models-cpu:8000/v1",
                  "EMBEDDING_API_KEY": "fixture", "EMBEDDING_MODEL": "fixture-embedding",
                  "LLM_BASE_URL": "http://retrieval-models-cpu:8000/v1", "LLM_API_KEY": "fixture", "LLM_MODEL": "fixture-chat"}
        args = ["docker", "run", "--rm", "--name", admin, "--label", "io.lazytt.test=" + name,
                "--network", name, "--entrypoint", "python"]
        for key in config:
            args.extend(["-e", key])
        for action in ("ensure", "check", "ensure"):
            try:
                run(args + [image, "/opt/lazytt/configuration.py", action], env={**os.environ, **config})
            except RuntimeError:
                logs = subprocess.run(["docker", "logs", mem0], capture_output=True, text=True)
                for line in (logs.stdout + logs.stderr).splitlines():
                    failure = re.search(r"Mem0 request failed: ([A-Za-z]+)$", line)
                    if failure:
                        print("MEM0_GATEWAY_FAILED: " + failure.group(1))
                raise
        # Exercise the actual Mem0 vector adapter without calling a model Provider.
        run(["docker", "exec", "-i", mem0, "python", "-"], data='''
import os, storage
from mem0 import Memory
store = Memory.from_config({**storage.load_config(storage.engine()), "history_db_path": "/tmp/adapter-test.db"}).vector_store
vector = [1.0] + [0.0] * 15
store.insert(vectors=[vector], ids=["isolated-vector-test"], payloads=[{"data": "test memory"}])
store.client.indices.refresh(index=store.collection_name)
assert store.get("isolated-vector-test").payload["data"] == "test memory"
assert store.search(query="", vectors=vector, top_k=1)[0].id == "isolated-vector-test"
''')
        print(run(["docker", "exec", "-i", mem0, "python", "-"], data=(ROOT / "tests/ecs/http_acceptance.py").read_text()), end="")
        run(["docker", "restart", mem0])
        ready()
        run(args + [image, "/opt/lazytt/configuration.py", "check"], env={**os.environ, **config})
        run(["docker", "exec", "-i", mem0, "python", "-"], data='''
import os, storage
from mem0 import Memory
store = Memory.from_config({**storage.load_config(storage.engine()), "history_db_path": "/tmp/adapter-test.db"}).vector_store
assert store.get("isolated-vector-test").payload["data"] == "test memory"
store.delete("isolated-vector-test")
''')
        print("MEM0_CONFIGURATION_INTEGRATION_OK: first start, authenticated configure, redacted GET, ES insert/search and persisted reload; no Provider calls")
    finally:
        subprocess.run(["docker", "rm", "-f", "-v", admin, mem0, model, elasticsearch, pg], capture_output=True)
        subprocess.run(["docker", "network", "rm", name], capture_output=True)
        remaining = run(["docker", "ps", "-aq", "--filter", "label=io.lazytt.test=" + name])
        if remaining.strip():
            raise RuntimeError("Temporary container cleanup incomplete")
        if name in run(["docker", "network", "ls", "--format", "{{.Name}}"]).splitlines():
            raise RuntimeError("Temporary network cleanup incomplete")


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image")
    parser.add_argument("--database-only", action="store_true")
    args = parser.parse_args()
    main(args.image, args.database_only)
