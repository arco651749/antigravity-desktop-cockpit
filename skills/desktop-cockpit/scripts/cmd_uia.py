import asyncio
import json
import sys
from collections import Counter

from ws_client import (
    WS_URL,
    normalize_center_to_uv,
    parse_hwnd,
    send_cmd,
)

HANDLED_COMMANDS = {"uia", "act", "invoke", "fill", "set-value", "toggle", "expand", "collapse", "diff"}


def handle(args):
    if args.command == "uia":
        cmd = {"action": "get_uia_tree", "depth": args.depth}
        if args.hwnd:
            cmd["hwnd"] = parse_hwnd(args.hwnd)
        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_tree"))
        if not res or res.get("status") == "error":
            msg = res.get("message") if isinstance(res, dict) else "無法取得 UIA 樹"
            print(f"[ERROR] UIA 掃描失敗: {msg}")
            sys.exit(1)

        tree = res.get("data", {}) if isinstance(res, dict) else {}
        if args.json:
            print(json.dumps(res, ensure_ascii=False, indent=2))
            return

        root_rect = tree.get("bounding_rect", [0, 0, 0, 0])
        all_nodes, ref_nodes, matches = [], [], []

        def walk(node, depth=0):
            ref_id = node.get("ref_id")
            name = node.get("name", "").strip()
            ctype = node.get("control_type", "")
            rect = node.get("bounding_rect", [0, 0, 0, 0])
            center = node.get("center_point", [0, 0])
            cid = node.get("id", "")
            val = node.get("value")
            tog = node.get("toggle_state")
            exp = node.get("expand_state")
            sel = node.get("is_selected")
            focused = node.get("is_focused", False)
            enabled = node.get("is_enabled", True)
            pats = node.get("supported_patterns", [])

            has_size = rect and len(rect) == 4 and rect[2] > 0 and rect[3] > 0
            if name or has_size or ref_id:
                u, v = normalize_center_to_uv(center, root_rect) if center and len(center) == 2 else (None, None)
                state_parts = []
                if val is not None:
                    v_disp = val if len(str(val)) <= 24 else str(val)[:21] + "..."
                    state_parts.append(f'val="{v_disp}"')
                if tog is not None:
                    state_parts.append(f"toggle={tog}")
                if exp is not None:
                    state_parts.append(f"{exp}")
                if sel is True:
                    state_parts.append("Selected")
                if focused:
                    state_parts.append("FOCUSED")
                if not enabled:
                    state_parts.append("DISABLED")

                item = {
                    "ref": ref_id or "-",
                    "depth": depth,
                    "type": ctype,
                    "name": name,
                    "rect": rect,
                    "center": center,
                    "id": cid,
                    "u": u,
                    "v": v,
                    "state": ", ".join(state_parts) if state_parts else "-",
                    "patterns": pats,
                }
                all_nodes.append(item)
                if ref_id:
                    ref_nodes.append(item)
                if args.find:
                    kw = args.find.lower()
                    if (
                        kw in name.lower()
                        or kw in ctype.lower()
                        or kw in cid.lower()
                        or (ref_id and kw == ref_id.lower())
                    ):
                        matches.append(item)

            for child in node.get("children", []):
                walk(child, depth + 1)

        walk(tree)

        if args.find:
            print(f"[UIA-FIND] 關鍵字 '{args.find}' 搜尋結果 (共 {len(matches)} 筆匹配):")
            print("-" * 122)
            print(f"{'Ref':<6} | {'深度':<4} | {'類型':<12} | {'歸一化 (u, v)':<17} | {'狀態 / 值':<22} | {'名稱 / 標籤 [動作]'}")
            print("-" * 122)
            for item in matches:
                uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                name_display = item['name'][:42] if item['name'] else (f"[ID: {item['id']}]" if item['id'] else "-")
                pats_str = f" [{','.join(item['patterns'])}]" if item['patterns'] else ""
                print(f"{item['ref']:<6} | {item['depth']:<4} | {item['type']:<12} | {uv_str:<17} | {item['state'][:22]:<22} | {name_display}{pats_str}")
            print("-" * 122)
            if matches:
                m0 = matches[0]
                if m0["ref"] != "-":
                    print(f"💡 語意觸發提示: python.exe -X utf8 cockpit_cli.py act {m0['ref']}")
                elif m0["u"] is not None:
                    print(f"💡 座標點擊提示: python.exe -X utf8 cockpit_cli.py mouse click {m0['u']} {m0['v']}")
        elif args.summary:
            ctls = Counter(n["type"] for n in all_nodes)
            print(f"[UIA-SUMMARY] 節點總數: {len(all_nodes)} | 可互動 @Ref 總數: {len(ref_nodes)}")
            print("控制項類型分布:")
            for ctl, count in ctls.most_common(12):
                print(f"  - {ctl:<16}: {count}")
            named = [n for n in all_nodes if n["name"]]
            print(f"具備文字語意名稱節點數: {len(named)}")
        else:
            win_title = tree.get("name", "未知視窗")
            print(f"[UIA-SNAPSHOT] 視窗: \"{win_title}\" | 總節點: {len(all_nodes)} | 可互動 @Ref: {len(ref_nodes)}")
            if not ref_nodes:
                print("⚠️ 此視窗無可互動的 UIA @Ref 控制項（可能為 Blender/CAD/自繪 Canvas），請改用 `capture` + `mouse click <u, v>` 操控。")
                for item in all_nodes[:10]:
                    uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                    print(f"  - [{item['type']}] \"{item['name']}\" uv={uv_str}")
            else:
                print("-" * 122)
                print(f"{'Ref':<6} | {'類型':<12} | {'歸一化 (u, v)':<17} | {'狀態 / 當前值':<24} | {'名稱 / ID [支援 Pattern]'}")
                print("-" * 122)
                for item in ref_nodes:
                    uv_str = f"({item['u']}, {item['v']})" if item['u'] is not None else "-"
                    label = f"\"{item['name'][:40]}\"" if item['name'] else (f"[ID:{item['id'][:30]}]" if item['id'] else "(無標籤)")
                    pats_str = f" [{','.join(item['patterns'])}]" if item['patterns'] else ""
                    print(f"{item['ref']:<6} | {item['type']:<12} | {uv_str:<17} | {item['state'][:24]:<24} | {label}{pats_str}")
                print("-" * 122)
                print("💡 操作提示: `cockpit_cli.py act @1` (靜默觸發/自動降級點擊) | `cockpit_cli.py fill @N \"文字\"` (瞬間設值)")

    elif args.command in ("act", "invoke", "fill", "set-value", "toggle", "expand", "collapse"):
        hwnd = parse_hwnd(args.hwnd) if getattr(args, "hwnd", None) else None
        if args.command in ("act", "invoke"):
            subaction, val = "act", None
        elif args.command in ("fill", "set-value"):
            subaction, val = "set_value", args.text
        else:
            subaction, val = args.command, None

        cmd = {"action": "uia_action", "target": args.target, "subaction": subaction}
        if hwnd:
            cmd["hwnd"] = hwnd
        if val is not None:
            cmd["value"] = val

        res = asyncio.run(send_cmd(cmd, wait_for_event="uia_action_done"))
        if not res:
            print("[ERROR] UIA 動作未收到引擎回應")
            sys.exit(1)

        status = res.get("status")
        data = res.get("data", {}) if isinstance(res.get("data"), dict) else {}
        msg = res.get("message", "")

        if status == "ok":
            ver_str = f" | 狀態/驗證值: {data.get('verified_value')}" if data.get("verified_value") is not None else ""
            print(
                f"[UIA-{subaction.upper()}] ✅ 成功: {data.get('target')} -> "
                f"[{data.get('control_type')}] \"{data.get('resolved_name')}\" "
                f"(機制: {data.get('mechanism')}){ver_str}"
            )
        elif status == "fallback_required" and not getattr(args, "no_fallback", False):
            u_val, v_val = normalize_center_to_uv(data.get("center_point", [0, 0]))

            async def exec_fallback():
                import websockets

                async with websockets.connect(WS_URL) as ws:
                    if hwnd:
                        await ws.send(json.dumps({"action": "set_window", "hwnd": hwnd}))
                        await asyncio.sleep(0.08)

                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "move",
                        "button": "none", "u": u_val, "v": v_val, "delta": 0,
                    }))
                    await asyncio.sleep(0.12)
                    await ws.send(json.dumps({
                        "action": "mouse", "source": "agent", "mode": "direct", "subaction": "click",
                        "button": "left", "u": u_val, "v": v_val, "delta": 0,
                    }))
                    await asyncio.sleep(0.10)

                    if subaction == "set_value" and val is not None:
                        if not getattr(args, "append", False):
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                                "vk": ord("A"), "ctrl": True, "alt": False, "shift": False,
                            }))
                            await asyncio.sleep(0.04)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                                "vk": ord("A"), "ctrl": True, "alt": False, "shift": False,
                            }))
                            await asyncio.sleep(0.05)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "down",
                                "vk": 8, "ctrl": False, "alt": False, "shift": False,
                            }))
                            await asyncio.sleep(0.03)
                            await ws.send(json.dumps({
                                "action": "key", "source": "agent", "mode": "direct", "subaction": "up",
                                "vk": 8, "ctrl": False, "alt": False, "shift": False,
                            }))
                            await asyncio.sleep(0.05)

                        t_cmd = {"action": "type_text", "text": val}
                        if hwnd:
                            t_cmd["hwnd"] = hwnd
                        await ws.send(json.dumps(t_cmd))
                        await asyncio.sleep(0.15)
                        print(
                            f"[UIA-FILL] ⚡ {args.target} (\"{data.get('resolved_name')}\") 不支援直接 ValuePattern，"
                            f"已自動降級執行點擊聚焦 ({u_val}, {v_val}) + WM_CHAR 注入 ({len(val)} 字元)！"
                        )
                    else:
                        print(
                            f"[UIA-{subaction.upper()}] ⚡ {args.target} (\"{data.get('resolved_name')}\") 未提供靜默 Pattern，"
                            f"已自動降級執行虛擬游標點擊 ({u_val}, {v_val})！"
                        )

            asyncio.run(exec_fallback())
        else:
            print(f"[ERROR] UIA 動作失敗 ({args.target}): {msg}")
            sys.exit(1)

    elif args.command == "diff":
        if getattr(args, "hwnd", None):
            h_val = parse_hwnd(args.hwnd)
            asyncio.run(send_cmd({"action": "set_window", "hwnd": h_val}, wait_for_event="window_set"))
        res = asyncio.run(send_cmd({"action": "get_visual_diff"}, wait_for_event="visual_diff"))
        if getattr(args, "json", False):
            print(json.dumps(res, ensure_ascii=False, indent=2))
            return
        diff = res.get("data") if isinstance(res, dict) else None
        if not diff:
            print("[DIFF] 尚無足夠的歷史快照可供比對（請至少執行 2 次 uia 或 diff）。")
        else:
            print("[DIFF] Visual Memory 狀態與版面差異比對報告:")
            add_w = diff.get("added_windows", [])
            cls_w = diff.get("closed_windows", [])
            dlg_w = diff.get("new_dialogs", [])
            add_e = diff.get("added_elements", [])
            rem_e = diff.get("removed_elements", [])
            chg_e = diff.get("state_changed_elements", [])
            if dlg_w:
                print(f"  🚨 新彈出對話框 ({len(dlg_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in dlg_w))
            if add_w:
                print(f"  ➕ 新增視窗 ({len(add_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in add_w))
            if cls_w:
                print(f"  ➖ 關閉視窗 ({len(cls_w)}): " + ", ".join(f"\"{w['title']}\" (0x{w['hwnd']:X})" for w in cls_w))
            if chg_e:
                print(f"  🔄 狀態/數值變更控制項 ({len(chg_e)}):")
                for c in chg_e[:25]:
                    print(f"     * {c}")
            if add_e:
                print(f"  🆕 新增控制項 ({len(add_e)}):")
                for a in add_e[:25]:
                    print(f"     + {a}")
            if rem_e:
                print(f"  🗑️ 移除控制項 ({len(rem_e)}):")
                for r in rem_e[:15]:
                    print(f"     - {r}")
            if not any([dlg_w, add_w, cls_w, chg_e, add_e, rem_e]):
                print("  ✅ 畫面與控制項狀態無變化。")
