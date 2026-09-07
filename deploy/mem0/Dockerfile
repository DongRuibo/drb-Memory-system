FROM python:3.12 AS source
WORKDIR /app
# The build script and CI provide the same fixed upstream revision.
COPY _upstream/mem0/server/*.py _upstream/mem0/server/alembic.ini _upstream/mem0/server/requirements.txt ./
COPY _upstream/mem0/server/routers ./routers
COPY _upstream/mem0/server/alembic ./alembic
COPY _upstream/mem0/pyproject.toml _upstream/mem0/README.md _upstream/mem0/LICENSE ./packages/
COPY _upstream/mem0/mem0 ./packages/mem0
COPY deploy/mem0/prepare_vector_store.py deploy/mem0/patch_server_state.py deploy/ecs/image_patch.py /tmp/
RUN python /tmp/prepare_vector_store.py /app \
    && python /tmp/patch_server_state.py && python /tmp/image_patch.py

FROM python:3.12 AS runtime
WORKDIR /app
# Only the prepared source enters runtime layers; upstream deployment examples
# and build-time removal scripts are not included in the application image.
COPY --from=source /app/requirements.txt ./requirements.txt
RUN pip install --no-cache-dir -r requirements.txt
COPY --from=source /app/packages ./packages
RUN pip install --no-cache-dir ./packages elasticsearch==8.19.3
COPY --from=source /app/*.py /app/alembic.ini ./
COPY --from=source /app/routers ./routers
COPY --from=source /app/alembic ./alembic
COPY deploy/mem0/configure.py deploy/ecs/database.py deploy/ecs/configuration.py /opt/lazytt/
COPY deploy/ecs/database/comments.json /opt/lazytt/comments.json
ENV PYTHONUNBUFFERED=1 PYTHONPATH=/app
LABEL io.lazytt.mem0.revision="b357a5a1b03c299ec8229c268e63cfac0f7c6566"
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000"]
