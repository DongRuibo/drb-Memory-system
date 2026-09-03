from pathlib import Path


path = Path("/app/server_state.py")
source = path.read_text(encoding="utf-8")
old = """    for key, value in updates.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key] = _merge_config(merged[key], value)
        else:
            merged[key] = value
"""
new = """    for key, value in updates.items():
        current = merged.get(key)
        provider_changed = (
            key in {\"vector_store\", \"llm\", \"embedder\"}
            and isinstance(value, dict)
            and isinstance(current, dict)
            and value.get(\"provider\") != current.get(\"provider\")
        )
        if provider_changed:
            merged[key] = deepcopy(value)
        elif isinstance(value, dict) and isinstance(current, dict):
            merged[key] = _merge_config(current, value)
        else:
            merged[key] = value
"""
if source.count(old) != 1:
    raise RuntimeError("pinned Mem0 server_state.py did not match the expected source")
path.write_text(source.replace(old, new), encoding="utf-8")
