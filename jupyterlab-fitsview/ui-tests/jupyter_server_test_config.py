"""Server configuration for integration tests.

!! Never use this configuration in production because it
opens the server to the world and provide access to JupyterLab
JavaScript objects through the global window variable.
"""
from jupyterlab.galata import configure_jupyter_server

configure_jupyter_server(c)

# CI runs these tests inside the official Playwright container, which runs as
# root; Jupyter refuses to start as root without this.
c.ServerApp.allow_root = True

# Uncomment to set server log level to debug level
# c.ServerApp.log_level = "DEBUG"
