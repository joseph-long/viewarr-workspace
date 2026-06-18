"""Pytest configuration for fitsview server extension tests."""

import pytest

pytest_plugins = ("pytest_jupyter.jupyter_server",)


@pytest.fixture
def jp_server_config(jp_server_config):
    """Configure the server to load the fitsview extension."""
    return {"ServerApp": {"jpserver_extensions": {"fitsview": True}}}
