#!/usr/bin/env python3
"""Memory dev deployment. One Compose, immutable candidates, no automatic migration."""
import argparse
import copy
import fcntl
import hashlib
import json
import ipaddress
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile

PROJECT = "lazytt-memory-ecs-dev"
IMAGE = re.compile(r"ghcr\.io/[a-z0-9_.\-/]+@sha256:[0-9a-f]{64}\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
REQUIRED = (
    "MEM0_POSTGRES_HOST MEM0_POSTGRES_PORT MEM0_POSTGRES_RUNTIME_PASSWORD MEM0_API_KEY "
    "LAZYTT_MEMORY_ELASTICSEARCH_ENDPOINT ELASTICSEARCH_PORT ELASTICSEARCH_USERNAME STORE_ELASTICSEARCH_PASSWORD "
    "ELASTICSEARCH_INDEX_PREFIX MEMORY_EMBEDDING_BASE_URL MEMORY_EMBEDDING_API_KEY MEMORY_EMBEDDING_MODEL "
    "MEMORY_EMBEDDING_DIMENSIONS MEMORY_EXTRACT_BASE_URL MEMORY_EXTRACT_API_KEY MEMORY_EXTRACT_MODEL "
    "LAZYTT_MEMORY_HOST_BIND LAZYTT_MEMORY_HOST_PORT LAZYTT_MEMORY_HEALTH_PORT MEMORY_JWT_HS256_SECRET MEMORY_JWT_ISSUER MEMORY_JWT_AUDIENCE MEMORY_TENANT_IDS"
).split()


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def run(command, env=None, input=None):
    result = subprocess.run(command, env=env, input=input, capture_output=True, text=True,
                            stdin=subprocess.DEVNULL if input is None else None)
    # Never echo child output: Compose/driver/provider diagnostics can contain secrets.
    require(result.returncode == 0, "Command failed at " + Path(command[0]).name + "; inspect privately")
    return result.stdout


def read_config(path, migration=False, database_only=False, owner=(0, 0)):
    file = Path(path)
    mode = file.lstat()
    require(stat.S_ISREG(mode.st_mode) and (mode.st_uid, mode.st_gid) == owner
            and stat.S_IMODE(mode.st_mode) == 0o600, "Config must be a regular 0600 file owned by uid:gid " + ":".join(map(str, owner)))
    # Config is trusted administrator-owned Bash, explicitly sourced in a fresh shell.
    output = run(["/bin/bash", "-c", 'set -eu; set +x; source "$1"; env -0', "memory-config", str(file)],
                 env={"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"})
    values = dict(item.split("=", 1) for item in output.split("\0") if "=" in item)
    names = ["MEM0_MIGRATION_PASSWORD"] if migration else REQUIRED
    if database_only:
        names = ["MEM0_POSTGRES_HOST", "MEM0_POSTGRES_PORT", "MEM0_POSTGRES_RUNTIME_PASSWORD"]
    for key in names:
        value = values.get(key, "")
        require(value and not any(c in value for c in "\r\n") and "<" not in value,
                "Missing/placeholder config: " + key)
    forbidden = {"MEM0_MIGRATION_PASSWORD", "ECS_SSH_PRIVATE_KEY", "GITHUB_TOKEN", "DATABASE_URL", "LAZYTT_DATABASE_ADMIN_PASSWORD"}
    require(migration or not forbidden.intersection(values), "Administrator/publishing credentials in application config")
    if not migration and not database_only:
        for key in ("MEM0_POSTGRES_PORT", "ELASTICSEARCH_PORT", "LAZYTT_MEMORY_HOST_PORT", "LAZYTT_MEMORY_HEALTH_PORT"):
            require(values[key].isdigit() and 1 <= int(values[key]) <= 65535, "Invalid port: " + key)
        require(ipaddress.ip_address(values["LAZYTT_MEMORY_HOST_BIND"]).version == 4, "Expected IPv4 business bind")
        require(values["LAZYTT_MEMORY_HOST_PORT"] != values["LAZYTT_MEMORY_HEALTH_PORT"], "Business and health ports must differ")
        require(values["MEMORY_EMBEDDING_DIMENSIONS"].isdigit() and 1 <= int(values["MEMORY_EMBEDDING_DIMENSIONS"]) <= 4096,
                "Invalid embedding dimensions")
        for key in ("MEM0_API_KEY", "MEMORY_JWT_HS256_SECRET"):
            require(len(values[key].encode()) >= 32, "Secret must be at least 32 bytes: " + key)
        require(len({values[key] for key in ("MEM0_API_KEY", "MEMORY_JWT_HS256_SECRET")}) == 2,
                "Independent auth secrets must not be reused")
    return {key: values[key] for key in names}


def fingerprint(values):
    return hashlib.sha256(json.dumps(values, sort_keys=True).encode()).hexdigest()


def candidate_identity(revision, memory_image, mem0_image):
    require(SHA.fullmatch(revision or "") and all(IMAGE.fullmatch(image or "") for image in (memory_image, mem0_image)),
            "Explicit full candidate revision and both immutable GHCR digests required")
    return {"revision": revision, "memory_image": memory_image, "mem0_image": mem0_image}


def structure(document):
    result = copy.deepcopy(document)
    result.pop("x-lazytt", None)
    for service in result["services"].values():
        service["image"] = "<image>"
    return result


class Deployment:
    def __init__(self, directory, values=None):
        self.directory = Path(directory)
        self.path = self.directory / "compose.yml"
        self.values = values or {}
        self.env = {key: os.environ[key] for key in ("PATH", "HOME", "DOCKER_CONFIG") if key in os.environ}
        self.env.update(self.values)
        self.env["COMPOSE_DISABLE_ENV_FILE"] = "true"

    def read(self):
        return json.loads(self.path.read_text())

    def write(self, document):
        with tempfile.NamedTemporaryFile(mode="w", dir=self.directory, delete=False) as file:
            json.dump(document, file, indent=2)
            temporary = file.name
        os.replace(temporary, self.path)

    def compose(self, *args, extra=None):
        return run(["docker", "compose", "--env-file", "/dev/null", "--project-directory", str(self.directory),
                    "-p", PROJECT, "-f", str(self.path), *args], env={**self.env, **(extra or {})})

    def task(self, program, action, service="admin", extra=None):
        if program == "configuration":
            mapping = {
                "MEM0_API_KEY": "MEM0_API_KEY", "MEM0_ELASTICSEARCH_HOST": "LAZYTT_MEMORY_ELASTICSEARCH_ENDPOINT",
                "MEM0_ELASTICSEARCH_PORT": "ELASTICSEARCH_PORT", "MEM0_ELASTICSEARCH_USER": "ELASTICSEARCH_USERNAME",
                "MEM0_ELASTICSEARCH_PASSWORD": "STORE_ELASTICSEARCH_PASSWORD", "MEM0_COLLECTION_NAME": "ELASTICSEARCH_INDEX_PREFIX",
                "EMBEDDING_BASE_URL": "MEMORY_EMBEDDING_BASE_URL", "EMBEDDING_API_KEY": "MEMORY_EMBEDDING_API_KEY",
                "EMBEDDING_MODEL": "MEMORY_EMBEDDING_MODEL", "EMBEDDING_DIMENSIONS": "MEMORY_EMBEDDING_DIMENSIONS",
                "LLM_API_KEY": "MEMORY_EXTRACT_API_KEY", "LLM_BASE_URL": "MEMORY_EXTRACT_BASE_URL", "LLM_MODEL": "MEMORY_EXTRACT_MODEL",
            }
            extra = {key: self.values[value] for key, value in mapping.items()}
            extra.update(MEM0_BASE_URL="http://mem0:8000", MEM0_VECTOR_STORE_PROVIDER="elasticsearch", MEM0_ELASTICSEARCH_VERIFY_CERTS="true")
        args = ["run", "--rm", "--no-deps", "-T", "--entrypoint", "python"]
        for key in (extra or {}):
            args.extend(["-e", key])  # Values are never command-line arguments.
        return self.compose(*args, service, "/opt/lazytt/" + program + ".py", action, extra=extra)

    def ownership(self):
        containers = run(["docker", "ps", "-aq", "--filter", "label=com.docker.compose.project=" + PROJECT]).split()
        require(not containers or self.path.exists(), "Existing project requires explicit adoption review")
        for kind, name in (("network", PROJECT + "_default"), ("volume", PROJECT + "_mem0-history")):
            found = run(["docker", kind, "ls", "--format", "{{.Name}}"])
            if name in found.splitlines():
                resource = json.loads(run(["docker", kind, "inspect", name]))[0]
                require(resource.get("Labels", {}).get("com.docker.compose.project") == PROJECT,
                        "Resource belongs to another owner: " + name)

    def check_image(self, image, revision):
        require(IMAGE.fullmatch(image) and SHA.fullmatch(revision), "Expected immutable GHCR digest and full Git SHA")
        run(["docker", "pull", image])
        found = json.loads(run(["docker", "image", "inspect", image]))[0]
        require(found["Architecture"] == "amd64" and found["Os"] == "linux", "Expected linux/amd64 image")
        require(found["Config"].get("Labels", {}).get("org.opencontainers.image.revision") == revision,
                "Image OCI revision differs from candidate")

    def prepare(self, source, revision, memory_image, mem0_image):
        candidate = candidate_identity(revision, memory_image, mem0_image)
        old = self.read() if self.path.exists() else None
        recorded = self.directory / "admin" / revision / "candidate.json"
        for prior in ([old["x-lazytt"]["candidate"]] if old else []) + ([json.loads(recorded.read_text())] if recorded.exists() else []):
            require(prior["revision"] != revision or prior == candidate,
                    "Cannot replace image digests for the same revision; reuse the prepared candidate")
        for image in (memory_image, mem0_image):
            self.check_image(image, revision)
        incoming = json.loads(Path(source).read_text())
        require(incoming["name"] == PROJECT, "Wrong Compose project")
        require(set(incoming["services"]) == {"memory-service", "mem0", "admin"}, "Unexpected service set")
        for service in incoming["services"].values():
            require("build" not in service, "ECS must never build images")
        self.ownership()
        if old:
            require(structure(old) == structure(incoming), "Live Compose topology changed; requires a reviewed resource upgrade")
            incoming = old
        else:
            incoming["services"]["memory-service"]["image"] = memory_image
            incoming["services"]["mem0"]["image"] = mem0_image
        incoming["services"]["admin"]["image"] = mem0_image
        state = incoming.setdefault("x-lazytt", {})
        state["candidate"] = candidate
        self.write(incoming)
        print("PREPARED " + revision + ": images only; no migration/start/configuration")

    def assert_candidate(self, revision, memory_image, mem0_image):
        expected = candidate_identity(revision, memory_image, mem0_image)
        document = self.read()
        candidate = document["x-lazytt"]["candidate"]
        require(candidate == expected, "Prepared candidate changed; do not migrate or release stale images")
        require(document["services"]["admin"]["image"] == candidate["mem0_image"], "Candidate administration image mismatch")

    def install_admin_bundle(self, revision, memory_image, mem0_image, config_file):
        require(re.fullmatch(r"/[A-Za-z0-9_./-]+", config_file or "") and config_file != "/", "Explicit absolute ECS config path required")
        self.assert_candidate(revision, memory_image, mem0_image)
        source = Path(__file__).parent
        files = [Path("release.py"), Path("README.md"), Path("环境变量.md"), Path(".env.dev.example")]
        files += [p.relative_to(source) for p in sorted((source / "database").iterdir()) if p.suffix in {".sql", ".json"}]
        destination = self.directory / "admin" / revision
        destination.mkdir(parents=True, exist_ok=True)
        candidate_file = destination / "candidate.json"
        candidate = candidate_identity(revision, memory_image, mem0_image)
        if candidate_file.exists():
            require(not candidate_file.is_symlink() and json.loads(candidate_file.read_text()) == candidate,
                    "Administration candidate changed for the same revision")
        else:
            candidate_file.write_text(json.dumps(candidate, indent=2) + "\n")
        for relative in files:
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                require(not target.is_symlink() and target.read_bytes() == (source / relative).read_bytes(),
                        "Administration bundle changed for the same revision")
            else:
                shutil.copyfile(source / relative, target)
        wrapper = self.directory / "database.sh"
        wrapper.write_text('#!/usr/bin/env bash\nset -euo pipefail\n'
            'directory="$(cd -- "$(dirname -- "$0")" && pwd)"\n'
            'action="${1:-migrate}"; if (( $# )); then shift; fi\n'
            'case "$action" in migrate|database-check) ;; *) exit 2 ;; esac\n'
            'exec python3 "$directory/admin/' + revision + '/release.py" "$action" '
            '--directory "$directory" "$@" --config ' + config_file + ' --revision ' + revision +
            ' --memory-image ' + memory_image + ' --mem0-image ' + mem0_image + '\n')
        wrapper.chmod(0o700)
        print("ADMIN_BUNDLE_READY: " + str(wrapper) + " migrate --migration-config /etc/lazytt/memory-migrate.env")

    def capacity(self):
        # Applications budget 2.5 GiB; an administrator task needs another 0.5 GiB.
        available = re.search(r"^MemAvailable:\s+(\d+) kB", Path("/proc/meminfo").read_text(), re.M)
        require(available and int(available[1]) >= 4 * 1024 * 1024, "Need 4 GiB MemAvailable including deployment headroom")

    def preflight(self):
        self.ownership()
        self.compose("config", "--quiet")
        self.capacity()

    def configure(self):
        self.preflight()
        self.task("database", "check")
        self.task("configuration", "preflight")
        document = self.read()
        require(document["services"]["mem0"]["image"] == document["x-lazytt"]["candidate"]["mem0_image"],
                "Configure requires Mem0 running the prepared candidate; upgrade/release first")
        self.compose("up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180", "mem0")
        self.task("configuration", "configure")
        print("CONFIGURED: authenticated write and readback; Memory Service not started by this action")

    def verify_running(self, document):
        for service in ("mem0", "memory-service"):
            ids = self.compose("ps", "-q", service).split()
            require(len(ids) == 1, "Missing or duplicate application container")
            container = json.loads(run(["docker", "inspect", ids[0]]))[0]
            require(container["Config"]["Image"] == document["services"][service]["image"], "Running image differs from Compose")
            require(container["State"].get("Health", {}).get("Status") == "healthy" and not container["State"].get("OOMKilled"),
                    "Application unhealthy/OOM")

    def release(self):
        self.preflight()
        previous = self.read()
        candidate = previous["x-lazytt"]["candidate"]
        self.task("database", "check")  # Candidate image against the existing schema.
        self.task("configuration", "preflight")  # Read-only; reject incompatible settings before touching services.
        compatible = False
        if previous["x-lazytt"].get("released") and previous["x-lazytt"].get("config_hash") == fingerprint(self.values):
            try:
                self.task("database", "check", service="mem0")
                self.verify_running(previous)
                compatible = True
            except RuntimeError:
                pass
        updated = copy.deepcopy(previous)
        updated["services"]["mem0"]["image"] = candidate["mem0_image"]
        updated["services"]["memory-service"]["image"] = candidate["memory_image"]
        # Preflight failures above do not touch existing services.
        self.write(updated)
        try:
            self.compose("up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180", "mem0")
            self.task("configuration", "ensure")  # Configure only when explicitly changed or not yet stored.
            self.compose("up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180", "memory-service")
            self.verify_running(updated)
        except RuntimeError:
            updated["x-lazytt"]["released"] = False
            self.write(updated)
            self.compose("stop", "memory-service", "mem0")
            if compatible:
                previous["x-lazytt"]["released"] = False
                self.write(previous)
                self.compose("up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180", "mem0")
                self.compose("up", "-d", "--no-build", "--no-deps", "--wait", "--wait-timeout", "180", "memory-service")
                self.verify_running(previous)
                previous["x-lazytt"]["released"] = True
                self.write(previous)
                print("RESTORED compatible previous images; database was not rolled back")
            else:
                updated["x-lazytt"]["released"] = False
                self.write(updated)
                print("STOPPED applications; preserved database/volume; forward repair required")
            raise
        updated["x-lazytt"].update(released=True, revision=candidate["revision"], config_hash=fingerprint(self.values))
        self.write(updated)
        print("RUNTIME_READY " + candidate["revision"] + "; business/rejection/recovery acceptance is separate")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "database-check", "check", "migrate", "configure", "release"))
    parser.add_argument("--directory", required=True)
    parser.add_argument("--config", required=True, help="ECS_ENV_FILE: absolute path to the administrator-owned application config")
    parser.add_argument("--migration-config")
    parser.add_argument("--source")
    parser.add_argument("--revision")
    parser.add_argument("--memory-image")
    parser.add_argument("--mem0-image")
    args = parser.parse_args()
    directory = Path(args.directory)
    require(directory.is_absolute() and str(directory) not in {"/", "/etc", "/root"}, "Expected dedicated absolute deployment directory")
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / ".deploy.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.action == "prepare":
            require(all((args.source, args.revision, args.memory_image, args.mem0_image)), "Prepare requires source and exact image/revision arguments")
            deployment = Deployment(directory)
            deployment.prepare(args.source, args.revision, args.memory_image, args.mem0_image)
            deployment.install_admin_bundle(args.revision, args.memory_image, args.mem0_image, args.config)
            return
        values = read_config(args.config, database_only=args.action in {"migrate", "database-check"})
        deployment = Deployment(directory, values)
        deployment.assert_candidate(args.revision, args.memory_image, args.mem0_image)
        if args.action == "migrate":
            require(args.migration_config, "Explicit migration credential file required")
            secret = read_config(args.migration_config, migration=True)["MEM0_MIGRATION_PASSWORD"]
            deployment.preflight()
            deployment.task("database", "migrate", extra={"POSTGRES_USER": "lazytt_memory_mem0_owner", "POSTGRES_PASSWORD": secret})
            deployment.task("database", "check")
            print("MIGRATED prepared candidate; runtime permissions and structure verified")
        elif args.action == "database-check":
            deployment.task("database", "check")
            print("CANDIDATE_DATABASE_READY " + args.revision)
        elif args.action == "configure":
            deployment.configure()
        elif args.action == "release":
            deployment.release()
        else:
            deployment.preflight()
            deployment.task("database", "check")
            deployment.task("configuration", "check")
            deployment.verify_running(deployment.read())
            print("RUNTIME_CHECK_OK; no real model extraction/recall performed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(str(error) if type(error) is RuntimeError else "DEPLOY_FAILED: inspect current phase privately", file=sys.stderr)
        sys.exit(1)
