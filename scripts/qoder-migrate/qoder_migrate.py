#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Qoder 跨机迁移助手 - Python 版
=================================

从本机已登录的 Qoder IDE 中提取 access_token / refresh_token,
导出为 cockpit-tools 兼容 JSON;反向可把 JSON 注入到 Qoder 的
state.vscdb,使 Qoder 启动后即认为已登录。

加密层(完全对照 cockpit-tools 源码 vscode_inject.rs:153-263):
    Local State 里的 os_crypt.encrypted_key (base64, 前缀 "DPAPI")
    -> CryptUnprotectData -> 32 字节 AES-256 key
    -> AES-256-GCM (v10 前缀 + 12 字节 nonce + ciphertext + 16 字节 tag)

依赖:
    pip install cryptography
    Python 3.10+ (Win11 自带 / Microsoft Store 版均可)
    Windows 10/11 (DPAPI 是 Windows-only)

对位证据链:
    - state.vscdb  -> src-tauri/src/modules/qoder_instance.rs:80-82
    - Local State  -> src-tauri/src/modules/vscode_paths.rs:91-93
    - secret key   -> src-tauri/src/modules/qoder_account.rs:13-15
    - DPAPI + GCM  -> src-tauri/src/modules/vscode_inject.rs:153-263
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import sqlite3
import sys
import tempfile
from dataclasses import dataclass, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

try:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM  # type: ignore
    _HAVE_CRYPTOGRAPHY = True
except ImportError:
    _HAVE_CRYPTOGRAPHY = False


# ─────────────────────────── 常量(严格对照源码) ───────────────────────────
SECRET_USER_INFO = "secret://aicoding.auth.userInfo"
SECRET_USER_PLAN = "secret://aicoding.auth.userPlan"
SECRET_CREDIT    = "secret://aicoding.auth.creditUsage"
V10_PREFIX       = b"v10"          # vscode_inject.rs:51
CONFIG_FILENAME  = "qoder_migrate.config.json"


# ─────────────────────────── 配置(数据类 + JSON 双向) ───────────────────────────
@dataclass
class QoderConfig:
    qoder_user_data_dir: str = ""      # Qoder 用户数据根目录
    state_db_relpath: str = "User/globalStorage/state.vscdb"
    local_state_filename: str = "Local State"
    output_json: str = ""              # extract 模式产物
    input_json: str = ""               # inject 模式输入
    verbose: bool = True
    secret_user_info_key: str = SECRET_USER_INFO
    secret_user_plan_key: str = SECRET_USER_PLAN
    secret_credit_key:    str = SECRET_CREDIT

    def resolve(self) -> "QoderConfig":
        out = QoderConfig(**asdict(self))
        if not out.qoder_user_data_dir:
            out.qoder_user_data_dir = _default_qoder_user_data_dir()
        if not out.output_json:
            out.output_json = str(Path.cwd() / "qoder_credentials.json")
        if not out.input_json:
            out.input_json = str(Path.cwd() / "qoder_credentials.json")
        return out

    @property
    def state_db_path(self) -> Path:
        return Path(self.qoder_user_data_dir) / self.state_db_relpath

    @property
    def local_state_path(self) -> Path:
        return Path(self.qoder_user_data_dir) / self.local_state_filename


def _default_qoder_user_data_dir() -> str:
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA")
        if not appdata:
            raise RuntimeError("环境变量 APPDATA 未设置")
        return str(Path(appdata) / "Qoder")
    if sys.platform == "darwin":
        return str(Path.home() / "Library/Application Support/Qoder")
    return str(Path.home() / ".config/Qoder")


def load_config(path: Path) -> QoderConfig:
    if not path.exists():
        return QoderConfig()
    raw = json.loads(path.read_text(encoding="utf-8"))
    valid_keys = set(QoderConfig.__dataclass_fields__.keys())
    clean = {k: v for k, v in raw.items() if k in valid_keys}
    return QoderConfig(**clean)


def save_config(path: Path, cfg: QoderConfig) -> None:
    path.write_text(
        json.dumps(asdict(cfg), ensure_ascii=False, indent=2),
        encoding="utf-8",
    )


# ─────────────────────────── DPAPI(crypt32) ───────────────────────────
class _DataBlob(ctypes.Structure):
    _fields_ = [("cbData", wt.DWORD), ("pbData", ctypes.POINTER(ctypes.c_byte))]


def dpapi_unprotect(cipher: bytes) -> bytes:
    if sys.platform != "win32":
        raise RuntimeError("DPAPI 仅在 Windows 可用;非 Windows 请走 cockpit-tools UI 导入。")
    CryptUnprotectData = ctypes.windll.crypt32.CryptUnprotectData
    CryptUnprotectData.argtypes = [
        ctypes.POINTER(_DataBlob), ctypes.c_wchar_p, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, wt.DWORD, ctypes.POINTER(_DataBlob),
    ]
    CryptUnprotectData.restype = wt.BOOL
    LocalFree = ctypes.windll.kernel32.LocalFree
    LocalFree.argtypes = [ctypes.c_void_p]
    LocalFree.restype = ctypes.c_void_p

    in_blob = _DataBlob()
    in_blob.cbData = len(cipher)
    in_blob.pbData = ctypes.cast(
        ctypes.c_char_p(cipher), ctypes.POINTER(ctypes.c_byte)
    )
    out_blob = _DataBlob()
    ok = CryptUnprotectData(
        ctypes.byref(in_blob), None, None, None, None, 0, ctypes.byref(out_blob)
    )
    if not ok:
        err = ctypes.GetLastError()
        raise OSError(f"CryptUnprotectData 失败,Win32 错误码: {err}")
    try:
        return bytes(ctypes.string_at(out_blob.pbData, out_blob.cbData))
    finally:
        LocalFree(out_blob.pbData)


# ─────────────────────────── AES-256-GCM ───────────────────────────
def _require_crypto():
    if not _HAVE_CRYPTOGRAPHY:
        raise RuntimeError("缺少 cryptography 库。请运行: pip install cryptography")


def aesgcm_decrypt(key: bytes, nonce: bytes, ciphertext_with_tag: bytes) -> bytes:
    _require_crypto()
    if len(nonce) != 12:
        raise ValueError(f"nonce 必须是 12 字节,实际 {len(nonce)}")
    if len(ciphertext_with_tag) < 16:
        raise ValueError("密文太短,不可能含 16 字节 GCM tag")
    ct  = ciphertext_with_tag[:-16]
    tag = ciphertext_with_tag[-16:]
    return AESGCM(key).decrypt(nonce, ct + tag, None)


def aesgcm_encrypt(key: bytes, nonce: bytes, plaintext: bytes) -> bytes:
    _require_crypto()
    return AESGCM(key).encrypt(nonce, plaintext, None)


# ─────────────────────────── 取 master key ───────────────────────────
def get_qoder_encryption_key(local_state_path: Path, verbose: bool = True) -> bytes:
    if not local_state_path.exists():
        raise FileNotFoundError(f"未找到 Local State: {local_state_path}")
    raw = local_state_path.read_text(encoding="utf-8")
    j = json.loads(raw)
    b64 = j.get("os_crypt", {}).get("encrypted_key")
    if not b64:
        raise RuntimeError("Local State 缺少 os_crypt.encrypted_key")
    blob = base64.b64decode(b64)
    if len(blob) < 6 or blob[:5] != b"DPAPI":
        raise RuntimeError(f"encrypted_key 头 5 字节不是 DPAPI,实际: {blob[:5]!r}")
    key = dpapi_unprotect(blob[5:])
    if len(key) != 32:
        raise RuntimeError(f"DPAPI 解出的 AES key 长度异常: {len(key)} (期望 32)")
    if verbose:
        print("       OK,32 字节 AES-256 master key 已就绪", flush=True)
    return key


# ─────────────────────────── SQLite ───────────────────────────
def read_secret_row(db_path: Path, key: str) -> Optional[str]:
    if not db_path.exists():
        return None
    tmp_dir = Path(tempfile.mkdtemp(prefix="qoder-mig-"))
    try:
        tmp_db = tmp_dir / "state.vscdb"
        shutil.copy2(db_path, tmp_db)
        try:
            with sqlite3.connect(tmp_db) as conn:
                conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
                cur = conn.execute("SELECT value FROM ItemTable WHERE key = ?", (key,))
                row = cur.fetchone()
                return row[0] if row else None
        except sqlite3.DatabaseError:
            return None
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def write_secret_row(db_path: Path, key: str, value: str) -> None:
    if not db_path.parent.exists():
        db_path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(db_path) as conn:
        conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        conn.execute(
            "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
            (key, value),
        )
        conn.commit()


# ─────────────────────────── 解密 / 加密 secret ───────────────────────────
def decode_qoder_secret(master_key: bytes, sqlite_value: str) -> str:
    envelope = json.loads(sqlite_value)
    data = bytes(envelope["data"])
    if not data.startswith(V10_PREFIX):
        raise RuntimeError(f"密文前缀不是 v10,实际: {data[:3]!r}")
    if len(data) < 3 + 12 + 16:
        raise RuntimeError("密文长度不足")
    nonce = data[3:15]
    cipher = data[15:]
    plain = aesgcm_decrypt(master_key, nonce, cipher)
    return plain.decode("utf-8", errors="replace")


def encode_qoder_secret(master_key: bytes, plaintext: str) -> str:
    import secrets
    nonce = secrets.token_bytes(12)
    pt_bytes = plaintext.encode("utf-8")
    ct_with_tag = aesgcm_encrypt(master_key, nonce, pt_bytes)
    envelope = V10_PREFIX + nonce + ct_with_tag
    return json.dumps({"type": "Buffer", "data": list(envelope)}, ensure_ascii=False)


# ─────────────────────────── 提取 ───────────────────────────
def cmd_extract(cfg: QoderConfig) -> int:
    cfg = cfg.resolve()
    print("[1/4] 读取 Local State 并 DPAPI 解密 AES master key ...", flush=True)
    print(f"       {cfg.local_state_path}", flush=True)
    master_key = get_qoder_encryption_key(cfg.local_state_path, cfg.verbose)

    print("[2/4] 从 state.vscdb 读 userInfo 密文 ...", flush=True)
    print(f"       {cfg.state_db_path}", flush=True)
    raw_user = read_secret_row(cfg.state_db_path, cfg.secret_user_info_key)
    if not raw_user:
        raise RuntimeError(
            f"未在 state.vscdb 找到 {cfg.secret_user_info_key} —— "
            f"Qoder 未登录,或路径不对。"
        )

    print("[3/4] AES-256-GCM 解密 userInfo ...", flush=True)
    user_info = json.loads(decode_qoder_secret(master_key, raw_user))

    print("[4/4] 抓 userPlan / creditUsage(可空)...", flush=True)
    raw_plan  = read_secret_row(cfg.state_db_path, cfg.secret_user_plan_key)
    raw_usage = read_secret_row(cfg.state_db_path, cfg.secret_credit_key)
    plan_json  = json.loads(decode_qoder_secret(master_key, raw_plan))  if raw_plan  else None
    usage_json = json.loads(decode_qoder_secret(master_key, raw_usage)) if raw_usage else None

    email         = user_info.get("email")
    user_id       = user_info.get("id")
    access_token  = user_info.get("token")
    refresh_token = user_info.get("refreshToken")
    if not email:
        raise RuntimeError("userInfo 缺少 email")
    if not refresh_token:
        raise RuntimeError("userInfo 缺少 refreshToken —— Qoder 登录可能没完成")

    output = {
        "schema": "cockpit-tools/qoder-migrate/v1",
        "extractedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "email": email,
        "userId": user_id,
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "expireTime": user_info.get("expireTime"),
        "refreshTokenExpireTime": user_info.get("refreshTokenExpireTime"),
        "authUserInfoRaw":    user_info,
        "authUserPlanRaw":    plan_json,
        "authCreditUsageRaw": usage_json,
    }
    out_path = Path(cfg.output_json)
    out_path.write_text(
        json.dumps(output, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )

    print("", flush=True)
    print("提取完成", flush=True)
    print(f"  email        : {email}", flush=True)
    print(f"  userId       : {user_id}", flush=True)
    print(f"  access_token : {_mask(access_token)}(len={len(access_token or '')})", flush=True)
    print(f"  refresh_token: {_mask(refresh_token)}(len={len(refresh_token or '')})", flush=True)
    print("", flush=True)
    print(f"  -> 已写入: {out_path}", flush=True)
    print("  -> 下一步: 传到 B 机后,在 B 机执行 inject。", flush=True)
    return 0


# ─────────────────────────── 注入 ───────────────────────────
def cmd_inject(cfg: QoderConfig) -> int:
    cfg = cfg.resolve()
    in_path = Path(cfg.input_json)
    if not in_path.exists():
        raise FileNotFoundError(f"找不到输入文件: {in_path}")
    creds = json.loads(in_path.read_text(encoding="utf-8"))

    print(f"[1/5] 校验导入 JSON: {in_path}", flush=True)
    if not creds.get("refreshToken"):
        raise RuntimeError("JSON 缺少 refreshToken")
    print(f"       email = {creds.get('email')}", flush=True)

    print("[2/5] 检查 Qoder 进程 ...", flush=True)
    _ensure_qoder_closed()

    print("[3/5] 用 B 机 DPAPI 重新生成 master key 缓存 ...", flush=True)
    print(f"       {cfg.local_state_path}", flush=True)
    master_key = get_qoder_encryption_key(cfg.local_state_path, cfg.verbose)

    print("[4/5] 用 B 机 master key 重新加密 userInfo / userPlan / creditUsage ...", flush=True)
    re_user = encode_qoder_secret(
        master_key,
        json.dumps(creds["authUserInfoRaw"], ensure_ascii=False, separators=(",", ":")),
    )
    re_plan = encode_qoder_secret(
        master_key,
        json.dumps(creds["authUserPlanRaw"], ensure_ascii=False, separators=(",", ":")),
    ) if creds.get("authUserPlanRaw") else None
    re_use = encode_qoder_secret(
        master_key,
        json.dumps(creds["authCreditUsageRaw"], ensure_ascii=False, separators=(",", ":")),
    ) if creds.get("authCreditUsageRaw") else None

    print("[5/5] 写回 state.vscdb ...", flush=True)
    print(f"       {cfg.state_db_path}", flush=True)
    write_secret_row(cfg.state_db_path, cfg.secret_user_info_key, re_user)
    print("       UPDATE userInfo OK", flush=True)
    if re_plan:
        write_secret_row(cfg.state_db_path, cfg.secret_user_plan_key, re_plan)
        print("       UPDATE userPlan OK", flush=True)
    if re_use:
        write_secret_row(cfg.state_db_path, cfg.secret_credit_key, re_use)
        print("       UPDATE creditUsage OK", flush=True)

    print("", flush=True)
    print("注入完成", flush=True)
    print(f"  email       : {creds.get('email')}", flush=True)
    print(f"  state.vscdb : {cfg.state_db_path}", flush=True)
    print("", flush=True)
    print(f"  -> 现在启动 Qoder IDE,它会认为 {creds.get('email')} 已登录。", flush=True)
    print("  -> 德姨提示:不要同时启动 A 机和 B 机的 cockpit-tools 刷新配额。", flush=True)
    return 0


# ─────────────────────────── 辅助 ───────────────────────────
def _mask(s, n: int = 24) -> str:
    if not s:
        return "<empty>"
    return f"{s[:n]}..." if len(s) > n else s


def _ensure_qoder_closed() -> None:
    if sys.platform != "win32":
        return
    import subprocess
    out = subprocess.run(
        ["tasklist", "/FI", "IMAGENAME eq Qoder.exe", "/FO", "CSV", "/NH"],
        capture_output=True, text=True
    )
    if "Qoder.exe" in (out.stdout or ""):
        raise RuntimeError(
            "检测到运行中的 Qoder.exe。\n"
            "请先手动关闭 Qoder IDE 后重跑 inject 模式。\n"
            "(脚本不强制 kill,杀进程会触发 Qoder 自身的加密重写,可能覆盖刚写入的数据)"
        )
    print("       没有运行中的 Qoder 进程", flush=True)


# ─────────────────────────── init / show ───────────────────────────
def cmd_init(config_path: Path) -> int:
    cfg = QoderConfig().resolve()
    save_config(config_path, cfg)
    print(f"已生成默认配置: {config_path}", flush=True)
    print("", flush=True)
    print("主人请按需修改里面的字段,主要可调:", flush=True)
    print(f"  qoder_user_data_dir  = {cfg.qoder_user_data_dir!r}", flush=True)
    print(f"  state_db_relpath     = {cfg.state_db_relpath!r}", flush=True)
    print(f"  local_state_filename = {cfg.local_state_filename!r}", flush=True)
    print(f"  output_json          = {cfg.output_json!r}", flush=True)
    print(f"  input_json           = {cfg.input_json!r}", flush=True)
    return 0


def cmd_show(cfg: QoderConfig) -> int:
    cfg = cfg.resolve()
    print("Qoder 跨机迁移助手 - Python 版", flush=True)
    print("", flush=True)
    print("解析后的路径(从配置文件 + 系统默认值合并):", flush=True)
    items = [
        ("qoder_user_data_dir", cfg.qoder_user_data_dir),
        ("state.vscdb",          str(cfg.state_db_path)),
        ("Local State",          str(cfg.local_state_path)),
        ("output_json",          cfg.output_json),
        ("input_json",           cfg.input_json),
    ]
    for name, p in items:
        path = Path(p) if p else None
        ok = bool(path and path.exists())
        marker = "[OK]" if ok else "[ - ]"
        color = "OK" if ok else " - "
        print(f"  {marker} {name:24s} = {p}  ({color})", flush=True)
    return 0


# ─────────────────────────── 入口 ───────────────────────────
def main() -> int:
    parser = argparse.ArgumentParser(
        prog="qoder_migrate",
        description="Qoder 跨机迁移助手 - 提取 / 注入 refresh token",
    )
    parser.add_argument(
        "mode", nargs="?", default="show",
        choices=["extract", "inject", "init", "show"],
        help="extract=A机导出; inject=B机导入; init=生成默认配置; show=显示当前解析后的路径",
    )
    parser.add_argument(
        "-c", "--config", default=CONFIG_FILENAME,
        help=f"配置文件路径(默认 {CONFIG_FILENAME})",
    )
    args = parser.parse_args()

    config_path = Path(args.config).resolve()

    if args.mode == "init":
        return cmd_init(config_path)

    cfg = load_config(config_path)
    cfg = cfg.resolve()

    if args.mode == "show":
        return cmd_show(cfg)
    if args.mode == "extract":
        return cmd_extract(cfg)
    if args.mode == "inject":
        return cmd_inject(cfg)
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n[中断] 主人按了 Ctrl+C,德姨先告退。", flush=True)
        sys.exit(130)
    except Exception as e:
        print(f"\n[错误] {type(e).__name__}: {e}", flush=True)
        print(f"        文件 {__file__}", flush=True)
        sys.exit(1)
