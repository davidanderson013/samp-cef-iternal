#!/usr/bin/env python3
"""
Packs a folder of CEF UI assets (the old plaintext `cef/assets/*.html`,
images, fonts, etc.) into a single encrypted container ("ui.bin") that the
patched samp-cef client decrypts only in memory (see
client/src/browser/asset_bundle.rs). The server never ships the plaintext
files to players anymore: it ships ui.bin, and hands out the decryption key
over the already-authenticated CEF socket channel after a player joins (see
server-core/src/server.rs::handle_auth and messages/AssetManifest).

Usage:
    pip install cryptography
    python3 scripts/pack_ui_assets.py <source_dir> <output_dir> [--key-file KEY_HEX_FILE]

Produces, inside <output_dir>:
    ui.bin         the encrypted container — this goes on the SERVER only,
                   next to the plugin (default path: cef/ui.bin). Never put
                   it in the client installer/package.
    ui_key.hex     64 hex chars (32-byte AES-256 key). SERVER-ONLY secret —
                   put it at cef/ui_key.hex next to ui.bin (or wherever
                   server.cfg's `cef_ui_key` points to), and keep it out of
                   version control / the client package. It is sent to
                   players automatically over the network each time they
                   join; it is never written to the client's disk as a
                   standalone file.
    ui.version     plain integer, bumped automatically every run. Purely
                   informational (shown in server logs); the client's real
                   "is my cache stale" check is the sha256 in the manifest.

Re-run this script every time Iternal_*.html (or the icon/font assets next
to them) change, then restart or re-load the plugin on the server so it
picks up the new ui.bin/ui.version. Players pick up the change automatically
the next time they connect, because the sha256 in the manifest will no
longer match what they have cached.
"""

from __future__ import annotations

import argparse
import json
import os
import secrets
import struct
import sys
from pathlib import Path

try:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
except ImportError:
    sys.exit(
        "Missing dependency: pip install cryptography\n"
        "(pip install --break-system-packages cryptography if you're on a "
        "system that blocks global installs)"
    )

MAGIC = b"UIB1"
INDEX_AAD = b"idx"

# Mirrors content_type_for_path() in client/src/browser/assets_scheme.rs.
# Not strictly required for packing (the client guesses MIME type from the
# extension on its own), but used here to skip obviously-irrelevant files
# (.DS_Store, Thumbs.db, editor swap files, ...).
SKIP_NAMES = {".ds_store", "thumbs.db", "desktop.ini"}
SKIP_SUFFIXES = {".tmp", ".swp", ".bak"}


def iter_source_files(source_dir: Path):
    for path in sorted(source_dir.rglob("*")):
        if not path.is_file():
            continue
        if path.name.lower() in SKIP_NAMES:
            continue
        if path.suffix.lower() in SKIP_SUFFIXES:
            continue
        yield path


def pack(source_dir: Path, key: bytes) -> bytes:
    aes = AESGCM(key)
    index = []
    blob = bytearray()

    files = list(iter_source_files(source_dir))
    if not files:
        sys.exit(f"No files found under {source_dir}")

    for path in files:
        rel = path.relative_to(source_dir).as_posix()
        nonce = secrets.token_bytes(12)
        ciphertext = aes.encrypt(nonce, path.read_bytes(), rel.encode("utf-8"))

        index.append(
            {
                "p": rel,
                "o": len(blob),
                "s": len(ciphertext),
                "n": nonce.hex(),
            }
        )
        blob += ciphertext

        print(f"  + {rel} ({len(ciphertext)} bytes encrypted)")

    index_bytes = json.dumps(index, separators=(",", ":")).encode("utf-8")
    index_nonce = secrets.token_bytes(12)
    index_ciphertext = aes.encrypt(index_nonce, index_bytes, INDEX_AAD)

    container = bytearray()
    container += MAGIC
    container += index_nonce
    container += struct.pack("<I", len(index_ciphertext))
    container += index_ciphertext
    container += blob

    return bytes(container)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source_dir", type=Path, help="folder with the plain UI assets (e.g. cef/assets)")
    parser.add_argument("output_dir", type=Path, help="folder to write ui.bin / ui_key.hex / ui.version into")
    parser.add_argument(
        "--key-file",
        type=Path,
        default=None,
        help="reuse an existing key file instead of generating a new one "
        "(keeps previously-cached client bundles valid; omit to rotate the key)",
    )
    args = parser.parse_args()

    source_dir: Path = args.source_dir
    output_dir: Path = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    if args.key_file is not None:
        key_hex = args.key_file.read_text().strip()
        if len(key_hex) != 64:
            sys.exit(f"{args.key_file} must contain exactly 64 hex characters (32-byte key)")
        key = bytes.fromhex(key_hex)
        print(f"Reusing existing key from {args.key_file}")
    else:
        key = secrets.token_bytes(32)
        print("Generated a new random AES-256 key (old cached client bundles will be re-downloaded once)")

    print(f"Packing {source_dir} ...")
    container = pack(source_dir, key)

    bin_path = output_dir / "ui.bin"
    key_path = output_dir / "ui_key.hex"
    version_path = output_dir / "ui.version"

    bin_path.write_bytes(container)
    key_path.write_text(key.hex())

    version = 1
    if version_path.exists():
        try:
            version = int(version_path.read_text().strip()) + 1
        except ValueError:
            version = 1
    version_path.write_text(str(version))

    print()
    print(f"Wrote {bin_path} ({len(container)} bytes)")
    print(f"Wrote {key_path}  -- SERVER SECRET, do not ship to clients or commit to a public repo")
    print(f"Wrote {version_path} (version {version})")
    print()
    print("Deploy: copy ui.bin + ui_key.hex (+ ui.version, optional) to the server,")
    print("next to the samp-cef plugin, at the paths server.cfg's cef_ui_bundle /")
    print("cef_ui_key point to (default: cef/ui.bin and cef/ui_key.hex).")
    print("Then restart the server (or reload the plugin) to pick it up.")


if __name__ == "__main__":
    main()
