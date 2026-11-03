"""Builds the .dmg from tt-pane.app with dmgbuild (https://dmgbuild.readthedocs.io):

    python3 dmg.py <tt-pane.app> <out.dmg>

The window Finder opens on the image shows dmg-background.png (and its
@2x) with tt-pane and a link to Applications on it, their icons at 128
points. dmgbuild writes Finder's .DS_Store itself, so this needs no Finder
scripting and runs on a CI runner. package.sh installs the pinned dmgbuild
from dmg-requirements.txt into a venv and runs this with it. Adapted from
tt-git's crates/tt-git-gui/packaging/macos/dmg.py.
"""

import os
import sys

import dmgbuild
import dmgbuild.core


class _NoBookmark:
    """dmgbuild stores the background as an alias and as a bookmark (pBBk).
    macOS 26's Finder follows the bookmark, which names the volume as it was
    mounted while building, finds nothing and shows no background; with the
    alias alone it shows it on macOS 26 and on earlier releases."""

    @staticmethod
    def for_file(path):
        return None


dmgbuild.core.Bookmark = _NoBookmark


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: dmg.py <tt-pane.app> <out.dmg>")
    app, out = sys.argv[1:]
    here = os.path.dirname(os.path.abspath(__file__))
    name = os.path.basename(app)
    settings = {
        "format": "UDZO",
        "filesystem": "HFS+",
        "files": [app],
        "symlinks": {"Applications": "/Applications"},
        # dmg-background@2x.png sits beside it; dmgbuild joins the two into
        # one HiDPI TIFF.
        "background": os.path.join(here, "dmg-background.png"),
        # The mounted volume shows the app's own icon.
        "icon": os.path.join(app, "Contents", "Resources", "tt-pane.icns"),
        # The background's size in points (660x420) plus Finder's 28-point
        # title bar. Finder places the window from the bottom left of the
        # screen.
        "window_rect": ((200, 360), (660, 448)),
        "icon_size": 128,
        "text_size": 13,
        # Icon centres, matching the tags and arrow drawn in
        # dmg-background.svg.
        "icon_locations": {name: (190, 200), "Applications": (470, 200)},
        "default_view": "icon-view",
        "show_status_bar": False,
        "show_tab_view": False,
        "show_toolbar": False,
        "show_pathbar": False,
        "show_sidebar": False,
        "show_icon_preview": False,
    }
    dmgbuild.build_dmg(out, "tt-pane", settings=settings)


if __name__ == "__main__":
    main()
