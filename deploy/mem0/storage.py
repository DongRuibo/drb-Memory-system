"""Project connection/configuration storage; never imports the upstream REST server."""
import json
import os

from sqlalchemy import URL, create_engine, text


def engine():
    url = URL.create("postgresql+psycopg", username=os.environ["POSTGRES_USER"],
                     password=os.environ["POSTGRES_PASSWORD"], host=os.environ["POSTGRES_HOST"],
                     port=int(os.environ["POSTGRES_PORT"]), database=os.environ["APP_DB_NAME"])
    return create_engine(url, pool_pre_ping=True, pool_size=3, max_overflow=2,
                         hide_parameters=True, connect_args={"connect_timeout": 5})


def load_config(db):
    with db.connect() as connection:
        value = connection.scalar(text("SELECT value FROM public.settings WHERE key='config_overrides'"))
    return json.loads(value) if value is not None else {}


def save_config(db, config):
    with db.begin() as connection:
        connection.execute(text("""INSERT INTO public.settings (key, value, updated_at)
            VALUES ('config_overrides', :value, now())
            ON CONFLICT (key) DO UPDATE SET value=EXCLUDED.value, updated_at=now()"""),
            {"value": json.dumps(config)})
