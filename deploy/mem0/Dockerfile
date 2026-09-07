FROM python:3.12 AS runtime
WORKDIR /app
# Package the fixed upstream source unchanged; only add the ES client and our
# external administration tools. Do not patch or remove upstream providers.
COPY _upstream/mem0/server/*.py _upstream/mem0/server/alembic.ini _upstream/mem0/server/requirements.txt ./
COPY _upstream/mem0/server/routers ./routers
COPY _upstream/mem0/server/alembic ./alembic
COPY _upstream/mem0/pyproject.toml _upstream/mem0/README.md _upstream/mem0/LICENSE ./packages/
COPY _upstream/mem0/mem0 ./packages/mem0
RUN pip install --no-cache-dir ./packages -r requirements.txt elasticsearch==8.19.3
COPY deploy/mem0/*.py deploy/mem0/alembic.ini deploy/ecs/database.py deploy/ecs/configuration.py /opt/lazytt/
COPY deploy/mem0/migration /opt/lazytt/migration
COPY deploy/ecs/database/comments.json deploy/ecs/database/03_verify.sql /opt/lazytt/
RUN PYTHONPATH=/app python /opt/lazytt/schema_manifest.py
ENV PYTHONUNBUFFERED=1 PYTHONPATH=/opt/lazytt:/app MEM0_TELEMETRY=false
LABEL io.lazytt.mem0.revision="b357a5a1b03c299ec8229c268e63cfac0f7c6566"
CMD ["uvicorn", "gateway:app", "--host", "0.0.0.0", "--port", "8000"]
