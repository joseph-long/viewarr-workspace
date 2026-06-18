c = get_config()

from jupyterlab import galata

galata.configure_jupyter_server(c)

# CI runs these tests inside the official Playwright container, which runs as
# root; Jupyter refuses to start as root without this.
c.ServerApp.allow_root = True
