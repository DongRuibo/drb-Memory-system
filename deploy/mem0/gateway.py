"""LazyTT's internal HTTP entry for the unchanged Mem0 SDK (ES only)."""
from contextlib import asynccontextmanager
from copy import deepcopy
import logging
import os
import secrets
import threading
from typing import Any

from fastapi import Depends, FastAPI, Header, HTTPException
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from mem0 import Memory
from mem0.configs.base import MemoryConfig
from pydantic import BaseModel, ConfigDict, Field

import database
import storage

LOCK = threading.RLock()
DB = storage.engine()
CURRENT = {}
MEMORY = None
KEY = os.environ.get("ADMIN_API_KEY", "")
HISTORY = os.environ.get("HISTORY_DB_PATH", "/app/history/history.db")


def validate_config(config):
    if set(config) != {"vector_store", "embedder", "llm"}:
        raise ValueError("Expected vector_store, embedder and llm")
    if config["vector_store"].get("provider") != "elasticsearch":
        raise ValueError("Only Elasticsearch is supported")
    if any(config[name].get("provider") != "openai" for name in ("embedder", "llm")):
        raise ValueError("Expected OpenAI-compatible model providers")
    MemoryConfig(**config)
    return deepcopy(config)


def compatible(config):
    if not CURRENT:
        return
    for section, fields in {"vector_store": ("collection_name", "embedding_model_dims", "host", "port"),
                            "embedder": ("model", "embedding_dims", "openai_base_url")}.items():
        for field in fields:
            if CURRENT[section]["config"].get(field) != config[section]["config"].get(field):
                raise HTTPException(409, "Index/model change requires a reviewed data migration")


def new_memory(config):
    return Memory.from_config({**config, "history_db_path": HISTORY})


@asynccontextmanager
async def lifespan(app):
    global CURRENT, MEMORY
    try:
        if len(KEY.encode()) < 32:
            raise ValueError("Invalid API key")
        # No schema writes at startup. Empty settings permit only administrative setup.
        with DB.connect() as connection:
            database.identity(connection)
            database.verify(connection)
        CURRENT = storage.load_config(DB)
        if CURRENT:
            CURRENT = validate_config(CURRENT)
    except Exception:
        raise RuntimeError("Mem0 startup failed; verify schema/configuration/dependencies privately") from None
    if CURRENT:
        try:
            MEMORY = new_memory(CURRENT)
        except Exception:
            # Keep authenticated setup reachable for a reviewed credential update.
            logging.getLogger("lazytt.mem0").error("Stored ES configuration is not ready; administrative setup remains available")
    yield
    DB.dispose()


app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)


def authenticated(x_api_key: str | None = Header(default=None)):
    if not x_api_key or not secrets.compare_digest(x_api_key, KEY):
        raise HTTPException(401, "Invalid API key")


@app.exception_handler(RequestValidationError)
async def invalid_request(request, error):
    return JSONResponse(status_code=422, content={"code": "INVALID_REQUEST"})


@app.middleware("http")
async def bounded_failure(request, call_next):
    try:
        return await call_next(request)
    except Exception as error:
        logging.getLogger("lazytt.mem0").error("Mem0 request failed: %s", type(error).__name__)
        return JSONResponse(status_code=503, content={"code": "MEM0_UNAVAILABLE"})


def ready_memory():
    if MEMORY is None:
        raise HTTPException(503, "Mem0 configuration is required")
    return MEMORY


@app.get("/auth/setup-status")
def setup_status():
    if MEMORY is not None:
        readiness()
    return {"configured": MEMORY is not None}


@app.get("/health/ready")
def readiness():
    with LOCK:
        if not ready_memory().vector_store.client.ping():
            raise HTTPException(503, "Elasticsearch is unavailable")
        with DB.connect() as connection:
            connection.exec_driver_sql("SELECT 1")
    return {"status": "ready", "vector_store": "elasticsearch"}


def redact(value):
    if isinstance(value, dict):
        return {key: "[redacted]" if key in {"api_key", "password"} else redact(item)
                for key, item in value.items()}
    return value


@app.get("/configure", dependencies=[Depends(authenticated)])
def get_config():
    with LOCK:
        return redact(CURRENT)


@app.post("/configure", dependencies=[Depends(authenticated)])
def set_config(config: dict[str, Any]):
    global CURRENT, MEMORY
    with LOCK:
        try:
            config = validate_config(config)
        except (ValueError, TypeError, KeyError):
            raise HTTPException(422, "Invalid Elasticsearch/model configuration") from None
        compatible(config)
        candidate = new_memory(config)
        storage.save_config(DB, config)  # Activate only after a successful commit.
        CURRENT, MEMORY = config, candidate
    return {"message": "Configuration set successfully"}


class Message(BaseModel):
    role: str = Field(min_length=1, max_length=32)
    content: str = Field(min_length=1, max_length=100000)


class AddRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    messages: list[Message] = Field(min_length=1, max_length=100)
    user_id: str = Field(min_length=1, max_length=512)
    metadata: dict[str, Any] = Field(default_factory=dict)
    infer: bool = True


class SearchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    query: str = Field(min_length=1, max_length=100000)
    filters: dict[str, Any]
    top_k: int = Field(default=20, ge=1, le=100)


@app.post("/memories", dependencies=[Depends(authenticated)])
def add_memory(body: AddRequest):
    with LOCK:
        return ready_memory().add([message.model_dump() for message in body.messages],
            user_id=body.user_id, metadata=body.metadata, infer=body.infer)


@app.post("/search", dependencies=[Depends(authenticated)])
def search(body: SearchRequest):
    if not isinstance(body.filters.get("user_id"), str) or not body.filters["user_id"]:
        raise HTTPException(422, "user_id filter is required")
    with LOCK:
        return ready_memory().search(body.query, filters=body.filters, top_k=body.top_k)


@app.get("/memories/{memory_id}", dependencies=[Depends(authenticated)])
def get_memory(memory_id: str):
    with LOCK:
        result = ready_memory().get(memory_id)
        if result is None:
            raise HTTPException(404, "Memory not found")
        return result


@app.delete("/memories/{memory_id}", dependencies=[Depends(authenticated)])
def delete_memory(memory_id: str):
    with LOCK:
        if ready_memory().get(memory_id) is None:
            raise HTTPException(404, "Memory not found")
        ready_memory().delete(memory_id)
    return {"message": "Memory deleted successfully"}
