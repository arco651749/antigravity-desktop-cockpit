import sys
from pathlib import Path

_SCRIPTS_DIR = Path(__file__).resolve().parent / "skills" / "desktop-cockpit" / "scripts"
if str(_SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS_DIR))

from cockpit_cli import main

if __name__ == "__main__":
    main()
