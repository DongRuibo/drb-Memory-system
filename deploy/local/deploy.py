#!/usr/bin/env python3
"""Explicit local operations; reuse the dev database and configuration tasks."""
import argparse
import fcntl
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/ecs"))
from release import Deployment, read_config, require, run

PROJECT = "lazytt-memory-local"


class LocalDeployment(Deployment):
    def __init__(self, values=None):
        super().__init__(ROOT, values)
        self.path = ROOT / "compose.local.yml"

    def compose(self, *args, extra=None):
        return run(["docker", "compose", "--env-file", "/dev/null", "--project-directory", str(ROOT),
                    "-p", PROJECT, "-f", str(self.path), *args], env={**self.env, **(extra or {})})

    def execute(self, action, migration_values=None):
        if action == "build":
            run(["bash", str(ROOT / "scripts/build-mem0-elasticsearch.sh")])
            self.compose("build", "memory-service")
        elif action == "validate":
            self.compose("config", "--quiet")
        elif action == "migrate":
            require(migration_values, "Explicit migration credential file required")
            self.task("database", "migrate", extra={"POSTGRES_USER": "lazytt_memory_mem0_owner",
                      "POSTGRES_PASSWORD": migration_values["MEM0_MIGRATION_PASSWORD"]})
            self.task("database", "check")
        elif action in {"configure", "up"}:
            self.task("database", "check")
            self.task("configuration", "preflight")
            self.compose("up", "-d", "--no-build", "--pull", "never", "--no-deps", "--wait",
                         "--wait-timeout", "180", "mem0")
            self.task("configuration", "configure" if action == "configure" else "check")
            if action == "up":
                self.compose("up", "-d", "--no-build", "--pull", "never", "--no-deps", "--wait",
                             "--wait-timeout", "180", "memory-service")
        elif action == "check":
            self.task("database", "check")
            self.task("configuration", "check")
            self.verify_running(self.read())
        elif action == "down":
            self.compose("down")  # Never remove persistent volumes or shared Store resources.
        elif action == "ps":
            containers = self.compose("ps", "--services", "--status", "running")
            print(containers, end="")
        print("LOCAL_" + action.upper() + "_OK")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "validate", "migrate", "configure", "up", "check", "ps", "down"))
    parser.add_argument("--config", type=Path, default=ROOT / ".env.local")
    parser.add_argument("--migration-config", type=Path)
    args = parser.parse_args()
    values = {}
    owner_values = None
    if args.action not in {"build", "validate", "down", "ps"}:
        owner = (os.getuid(), os.getgid())
        values = read_config(args.config.absolute(), database_only=args.action == "migrate", owner=owner)
        if args.action == "migrate":
            require(args.migration_config, "Explicit --migration-config required")
            owner_values = read_config(args.migration_config.absolute(), migration=True, owner=owner)
    deployment = LocalDeployment(values)
    if args.action in {"validate", "ps", "check"}:
        deployment.execute(args.action, owner_values)
    else:
        (ROOT / ".run").mkdir(exist_ok=True)
        with (ROOT / ".run/local-deploy.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            deployment.execute(args.action, owner_values)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(str(error) if type(error) is RuntimeError else "LOCAL_FAILED: inspect configuration or current phase privately", file=sys.stderr)
        sys.exit(1)
