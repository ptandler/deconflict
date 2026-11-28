"""deconflict — interactive resolver for 'conflicted copy'-style sync conflict files."""

from importlib.metadata import PackageNotFoundError, version

try:
    __version__ = version("deconflict")
except PackageNotFoundError:  # pragma: no cover - running from source without install
    __version__ = "0.0.0"
