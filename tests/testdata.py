"""Centralized test data expectations for sample-files fixtures."""

from pathlib import Path

SAMPLE_DIR = Path(__file__).resolve().parent.parent / "sample-files"


EXPECTED_GROUPS = 3

EXPECTED_NEXTCLOUD_GROUPS = 2

EXPECTED_PACMAN_GROUPS = 1

EXPECTED_SYNCTHING_GROUPS = 0

NEXTCLOUD_BASE_NAMES = [
    "Readme.md",
    "IMG-picsum.photos-30-200x300.jpg",
]

PACMAN_BASE_NAMES = [
    "pacdiff/some.txt",
]

ALL_BASE_NAMES = NEXTCLOUD_BASE_NAMES + PACMAN_BASE_NAMES

IMAGE_BASE_PREFIX = "IMG-"

PACMAN_BASE_PREFIX = "pacdiff/some.txt"
