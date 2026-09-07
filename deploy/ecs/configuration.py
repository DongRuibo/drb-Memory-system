"""Explicit, credential-silent Mem0 configuration and readback."""
import os
import sys
import json

import configure


def desired():
    return {
        "vector_store": configure.vector_store_config(int(configure.required("EMBEDDING_DIMENSIONS"))),
        "embedder": {"provider": "openai", "config": {
            "api_key": configure.required("EMBEDDING_API_KEY"),
            "model": configure.required("EMBEDDING_MODEL"),
            "embedding_dims": int(configure.required("EMBEDDING_DIMENSIONS")),
            "openai_base_url": configure.provider_base_url("EMBEDDING_BASE_URL")}},
        "llm": {"provider": "openai", "config": {
            "api_key": configure.required("LLM_API_KEY"),
            "model": configure.required("LLM_MODEL"),
            "openai_base_url": configure.provider_base_url("LLM_BASE_URL")}},
    }


def compare(expected, current, secrets=False):
    for section, wanted in expected.items():
        actual = current.get(section, {})
        if actual.get("provider") != wanted["provider"]:
            raise RuntimeError("Provider configuration mismatch")
        for field, value in wanted["config"].items():
            if not secrets and field in {"api_key", "password"}:
                continue  # Upstream GET /configure returns [redacted].
            if actual.get("config", {}).get(field) != value:
                raise RuntimeError("Configuration readback mismatch")


def main(action):
    expected = desired()
    url = configure.required("MEM0_BASE_URL").rstrip("/") + "/configure"
    key = configure.required("MEM0_API_KEY")
    current = configure.request_json(url, "GET", key)
    if action == "configure":
        # Existing Elasticsearch data needs a separate index/model migration plan.
        if current.get("vector_store", {}).get("provider") == "elasticsearch":
            for section, fields in {
                "vector_store": ("collection_name", "embedding_model_dims", "host", "port"),
                "embedder": ("model", "embedding_dims", "openai_base_url"),
            }.items():
                for field in fields:
                    if current.get(section, {}).get("config", {}).get(field) != expected[section]["config"][field]:
                        raise RuntimeError("Existing index/model change requires a reviewed data migration")
        configure.request_json(url, "POST", key, expected)
        current = configure.request_json(url, "GET", key)
    compare(expected, current)
    # Read through the runtime identity; never print the stored settings payload.
    from db import SessionLocal
    from models import Settings
    with SessionLocal() as session:
        row = session.get(Settings, "config_overrides")
        if row is None:
            raise RuntimeError("Configuration was not persisted")
        compare(expected, json.loads(row.value), secrets=True)
    print("MEM0_CONFIGURATION_OK")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2 or sys.argv[1] not in {"configure", "check"}:
            raise RuntimeError("Expected configure or check")
        main(sys.argv[1])
    except Exception:
        # Do not echo upstream response bodies, URLs or credentials.
        print("MEM0_CONFIGURATION_FAILED: inspect configuration privately", file=sys.stderr)
        sys.exit(1)
