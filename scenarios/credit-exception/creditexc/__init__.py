"""Regulated credit-policy exception workflow (ECorp scenario 3).

Standard-library only. See ``README.md`` for start/stop commands.
"""

from .service import ExceptionService
from .store import Store

__all__ = ["ExceptionService", "Store", "build_server", "__version__"]
__version__ = "1.0.0"


def build_server(host: str = "127.0.0.1", port: int = 9313, *, verbose: bool = False):
    """Create a threaded HTTP server bound to a fresh service instance."""
    from http.server import ThreadingHTTPServer

    from .api import ApiHandler

    service = ExceptionService()

    class BoundHandler(ApiHandler):
        pass

    BoundHandler.service = service

    httpd = ThreadingHTTPServer((host, port), BoundHandler)
    httpd.daemon_threads = True
    httpd.verbose = verbose
    httpd.service = service
    return httpd
