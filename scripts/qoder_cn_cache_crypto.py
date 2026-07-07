#!/usr/bin/env python3
"""
QoderCN SharedClientCache/cache/user & quota 加解密工具

加密方案（已逆向验证，往返字节级一致）:
  - 算法: AES-128-CBC + PKCS7 padding
  - 密钥 = IV = cache/id 文件内容的前 16 字节(UTF-8)
  - 外层: 标准 base64
  - 明文: UTF-8 JSON

用法:
  # 解密查看明文(默认从默认路径读取)
  python3 qoder_cn_cache_crypto.py decrypt quota
  python3 qoder_cn_cache_crypto.py decrypt user

  # 指定 cache 目录
  python3 qoder_cn_cache_crypto.py --cache-dir "/Users/xx/Library/Application Support/QoderCN/SharedClientCache/cache" decrypt user

  # 从 JSON 明文重新加密生成文件
  python3 qoder_cn_cache_crypto.py encrypt user --json /path/to/user.json --out /path/to/cache/user
  python3 qoder_cn_cache_crypto.py encrypt quota --json /path/to/quota.json --out /path/to/cache/quota

  # 编辑: 解密 → 用编辑器改 JSON → 重新加密写回
  python3 qoder_cn_cache_crypto.py edit user

依赖: openssl (系统自带), python3
"""
import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile

DEFAULT_CACHE_DIR = os.path.expanduser(
    "~/Library/Application Support/QoderCN/SharedClientCache/cache"
)


def read_key(cache_dir: str) -> bytes:
    """AES-128 密钥 = cache/id 前 16 字节"""
    id_path = os.path.join(cache_dir, "id")
    if not os.path.exists(id_path):
        die(f"找不到 cache/id: {id_path}")
    cid = open(id_path, "r").read().strip()
    key = cid.encode("utf-8")[:16]
    if len(key) < 16:
        die(f"cache/id 长度不足 16 字节: {cid!r}")
    return key


def aes_decrypt(ciphertext: bytes, key: bytes) -> bytes:
    """AES-128-CBC 解密, IV=key, 去 PKCS7 padding"""
    r = subprocess.run(
        ["openssl", "enc", "-d", "-aes-128-cbc",
         "-K", key.hex(), "-iv", key.hex(), "-nopad"],
        input=ciphertext, capture_output=True,
    )
    if r.returncode != 0:
        die(f"AES 解密失败: {r.stderr.decode()}")
    out = r.stdout
    if out and 1 <= out[-1] <= 16 and out[-out[-1]:] == bytes([out[-1]]) * out[-1]:
        out = out[:-out[-1]]
    return out


def aes_encrypt(plaintext: bytes, key: bytes) -> bytes:
    """AES-128-CBC 加密, IV=key, openssl 默认 PKCS7 padding"""
    r = subprocess.run(
        ["openssl", "enc", "-aes-128-cbc",
         "-K", key.hex(), "-iv", key.hex()],
        input=plaintext, capture_output=True,
    )
    if r.returncode != 0:
        die(f"AES 加密失败: {r.stderr.decode()}")
    return r.stdout


def decrypt_file(cache_dir: str, name: str) -> dict:
    """解密 cache/<name> 返回明文 dict"""
    path = os.path.join(cache_dir, name)
    if not os.path.exists(path):
        die(f"找不到文件: {path}")
    raw_b64 = open(path, "rb").read()
    data = base64.b64decode(raw_b64)
    key = read_key(cache_dir)
    pt = aes_decrypt(data, key)
    try:
        return json.loads(pt)
    except json.JSONDecodeError as e:
        die(f"明文不是合法 JSON: {e}\n前 200 字节: {pt[:200]!r}")


def encrypt_to_file(cache_dir: str, name: str, obj: dict, out_path: str | None = None):
    """加密 dict 写入文件"""
    key = read_key(cache_dir)
    pt = json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    ct = aes_encrypt(pt, key)
    b64 = base64.b64encode(ct)
    target = out_path or os.path.join(cache_dir, name)
    open(target, "wb").write(b64)
    print(f"已写入 {target} ({len(b64)} bytes)", file=sys.stderr)


def die(msg: str):
    print(f"[ERROR] {msg}", file=sys.stderr)
    sys.exit(1)


def cmd_decrypt(args):
    obj = decrypt_file(args.cache_dir, args.name)
    print(json.dumps(obj, ensure_ascii=False, indent=2))


def cmd_encrypt(args):
    if not args.json:
        die("encrypt 需要 --json 指定明文 JSON 文件")
    obj = json.load(open(args.json, "r"))
    encrypt_to_file(args.cache_dir, args.name, obj, args.out)


def cmd_edit(args):
    obj = decrypt_file(args.cache_dir, args.name)
    with tempfile.NamedTemporaryFile("w+", suffix=".json", delete=False) as f:
        json.dump(obj, f, ensure_ascii=False, indent=2)
        tmp = f.name
    editor = os.environ.get("EDITOR", "vi")
    subprocess.call([editor, tmp])
    obj2 = json.load(open(tmp, "r"))
    os.unlink(tmp)
    encrypt_to_file(args.cache_dir, args.name, obj2, args.out)
    print("已重新加密写回", file=sys.stderr)


def main():
    p = argparse.ArgumentParser(description="QoderCN cache/user & quota 加解密工具")
    p.add_argument("--cache-dir", default=DEFAULT_CACHE_DIR, help="SharedClientCache/cache 目录")
    sub = p.add_subparsers(dest="cmd", required=True)

    pd = sub.add_parser("decrypt", help="解密显示明文 JSON")
    pd.add_argument("name", choices=["user", "quota"])
    pd.set_defaults(func=cmd_decrypt)

    pe = sub.add_parser("encrypt", help="从 JSON 重新加密生成文件")
    pe.add_argument("name", choices=["user", "quota"])
    pe.add_argument("--json", required=True, help="明文 JSON 文件路径")
    pe.add_argument("--out", help="输出路径(默认写回 cache 目录)")
    pe.set_defaults(func=cmd_encrypt)

    pe2 = sub.add_parser("edit", help="解密→编辑→重新加密写回")
    pe2.add_argument("name", choices=["user", "quota"])
    pe2.add_argument("--out", help="输出路径(默认写回 cache 目录)")
    pe2.set_defaults(func=cmd_edit)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
