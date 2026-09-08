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


def compatible(expected, current):
    if not current:
        return  # First configuration; no stored index/model to migrate.
    for section in expected:
        if current.get(section, {}).get("provider") != expected[section]["provider"]:
            raise RuntimeError("Existing provider change requires a reviewed data migration")
    for section, fields in {
        "vector_store": ("collection_name", "embedding_model_dims", "host", "port"),
        "embedder": ("model", "embedding_dims", "openai_base_url"),
    }.items():
        for field in fields:
            if current.get(section, {}).get("config", {}).get(field) != expected[section]["config"][field]:
                raise RuntimeError("Existing index/model change requires a reviewed data migration")


def main(action):
    print("MEM0_CONFIGURATION_STAGE: validate", flush=True)
    expected = desired()
    import storage
    db = storage.engine()
    if action in {"preflight", "ensure", "configure"}:
        print("MEM0_CONFIGURATION_STAGE: database_preflight", flush=True)
        compatible(expected, storage.load_config(db))
    if action == "preflight":
        print("MEM0_CONFIGURATION_PREFLIGHT_OK")
        return  # No HTTP/SDK initialization, settings write or ES index creation.
    url = configure.required("MEM0_BASE_URL").rstrip("/") + "/configure"
    key = configure.required("MEM0_API_KEY")
    print("MEM0_CONFIGURATION_STAGE: http_readback", flush=True)
    current = configure.request_json(url, "GET", key)
    if action == "ensure":
        try:
            compare(expected, current)
            compare(expected, storage.load_config(db), secrets=True)
            configure.request_json(configure.required("MEM0_BASE_URL").rstrip("/") + "/health/ready", "GET", key)
        except (RuntimeError, OSError):
            action = "configure"
    if action == "configure":
        compatible(expected, current)
        print("MEM0_CONFIGURATION_STAGE: http_configure", flush=True)
        configure.request_json(url, "POST", key, expected)
        current = configure.request_json(url, "GET", key)
    compare(expected, current)
    # Read through the runtime identity; never print the stored settings payload.
    print("MEM0_CONFIGURATION_STAGE: database_readback", flush=True)
    compare(expected, storage.load_config(db), secrets=True)
    print("MEM0_CONFIGURATION_STAGE: readiness", flush=True)
    configure.request_json(configure.required("MEM0_BASE_URL").rstrip("/") + "/health/ready", "GET", key)
    print("MEM0_CONFIGURATION_OK")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2 or sys.argv[1] not in {"configure", "check", "ensure", "preflight"}:
            raise RuntimeError("Expected configure, check, ensure or preflight")
        main(sys.argv[1])
    except Exception as error:
        # Do not echo upstream response bodies, URLs or credentials.
        status = error.code if isinstance(error, configure.urllib.error.HTTPError) else 0
        print(f"MEM0_CONFIGURATION_FAILED: {type(error).__name__} http_status={status}", file=sys.stderr)
        sys.exit(1)
