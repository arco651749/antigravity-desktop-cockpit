"""
cockpit_cli.py (Root Forwarder / Backward Compatibility Proxy)

遵循 Antigravity Agent Skill 標準規範，主執行腳本已歸位至：
skills/desktop-cockpit/scripts/cockpit_cli.py

此檔案保留作為根目錄捷徑與向後相容轉發。
"""
import sys
import runpy
from pathlib import Path

target_script = Path(__file__).parent / "skills" / "desktop-cockpit" / "scripts" / "cockpit_cli.py"

if not target_script.exists():
    print(f"[ERROR] 找不到 Skill 腳本: {target_script}", file=sys.stderr)
    sys.exit(1)

runpy.run_path(str(target_script), run_name="__main__")
