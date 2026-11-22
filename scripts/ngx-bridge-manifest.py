#!/usr/bin/env python3
"""Offline Ed25519 signing of the optional EvoParseq NGX Bridge manifest.

Input/output locations are deliberately fixed: commands cannot be induced by
untrusted CLI strings to read or overwrite arbitrary filesystem paths.
No private signing key is stored in the repository or CI artifacts.

Prerequisite: python -m pip install cryptography
Usage:
    python scripts/ngx-bridge-manifest.py keygen
    python scripts/ngx-bridge-manifest.py sign --channel dev --sequence 1 \
        --version 0.9.9-dev.2 --tag v0.9.9-dev.2
    python scripts/ngx-bridge-manifest.py verify --channel dev --public-key BASE64KEY

The release operator places the reviewed, separately distributed DLL in
dist/ngx-bridge/EvoParseq-NGX-Bridge.dll before signing.
"""
import argparse
import base64
import hashlib
import json
import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DLL_NAME = "EvoParseq-NGX-Bridge.dll"
DLL_PATH = ROOT / "dist" / "ngx-bridge" / DLL_NAME
CHANNEL_DIR = ROOT / "ngx-bridge" / "channels"
KEY_PATH = Path.home() / ".config" / "evoparseq" / "ngx-signing-key.pem"
ASSET_BASE = "https://github.com/vampywiz17/EvoParseq/releases/download"
MAX_SIZE = 16 * 1024 * 1024
RELEASE_TOKEN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}\Z")


def pack(data):
    return json.dumps(data, sort_keys=True, separators=(",", ":")).encode("utf-8")


def manifest_path(channel):
    if channel not in ("dev", "stable"):
        raise ValueError("Unexpected release channel")
    return CHANNEL_DIR / f"{channel}.json"


def crypto():
    try:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey, Ed25519PublicKey,
        )
    except ImportError as exc:
        raise SystemExit("Install cryptography: python -m pip install cryptography") from exc
    return serialization, Ed25519PrivateKey, Ed25519PublicKey


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    cmds = parser.add_subparsers(dest="command", required=True)
    cmds.add_parser("keygen")
    sign = cmds.add_parser("sign")
    sign.add_argument("--channel", required=True, choices=["dev", "stable"])
    sign.add_argument("--sequence", type=int, required=True)
    sign.add_argument("--version", required=True)
    sign.add_argument("--tag", required=True)
    verify = cmds.add_parser("verify")
    verify.add_argument("--channel", required=True, choices=["dev", "stable"])
    verify.add_argument("--public-key", required=True)
    args = parser.parse_args()
    serialization, Ed25519PrivateKey, Ed25519PublicKey = crypto()
    if args.command == "keygen":
        KEY_PATH.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        private = Ed25519PrivateKey.generate()
        data = private.private_bytes(serialization.Encoding.PEM,
                                     serialization.PrivateFormat.PKCS8,
                                     serialization.NoEncryption())
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        fd = os.open(KEY_PATH, flags, 0o600)
        with os.fdopen(fd, "wb") as fp:
            fp.write(data)
        public = private.public_key().public_bytes(serialization.Encoding.Raw,
                                                   serialization.PublicFormat.Raw)
        print("Set repository variable NGX_MANIFEST_PUBLIC_KEY_B64:")
        print(base64.b64encode(public).decode("ascii"))
        print(f"Private key created outside Git: {KEY_PATH}")
    elif args.command == "sign":
        if args.sequence <= 0:
            parser.error("sequence must be positive and monotonically increasing")
        if not RELEASE_TOKEN.fullmatch(args.version):
            parser.error("invalid version")
        if not RELEASE_TOKEN.fullmatch(args.tag):
            parser.error("invalid tag")
        # Only the reviewed artifact at a fixed, project-local location is signed.
        # Reject symbolic links that escape the repository staging area.
        if not DLL_PATH.resolve(strict=True).is_relative_to(ROOT):
            parser.error("NGX DLL staging path leaves the repository")
        data = DLL_PATH.read_bytes()
        if not 0 < len(data) <= MAX_SIZE:
            parser.error("invalid DLL size")
        private = serialization.load_pem_private_key(KEY_PATH.read_bytes(), password=None)
        if not isinstance(private, Ed25519PrivateKey):
            parser.error("signing key must be Ed25519")
        target = manifest_path(args.channel)
        if target.exists():
            previous = json.loads(target.read_bytes())
            old_payload = base64.b64decode(previous["payload"], validate=True)
            old_signature = base64.b64decode(previous["signature"], validate=True)
            private.public_key().verify(old_signature, old_payload)
            if args.sequence <= json.loads(old_payload)["sequence"]:
                parser.error("sequence must be greater than current signed manifest")
        payload = {
            "schema": 1, "channel": args.channel, "sequence": args.sequence,
            "version": args.version, "abi": 3,
            "sha256": hashlib.sha256(data).hexdigest(), "size": len(data),
            "url": f"{ASSET_BASE}/{args.tag}/{DLL_NAME}",
        }
        raw = pack(payload)
        envelope = {
            "payload": base64.b64encode(raw).decode("ascii"),
            "signature": base64.b64encode(private.sign(raw)).decode("ascii"),
        }
        target.parent.mkdir(parents=True, exist_ok=True)
        # Restrict writes to fixed channel-specific manifests.
        target.write_bytes(pack(envelope) + b"\n")
        print(f"Signed {args.channel} manifest for {payload['sha256']}.")
        print("Publish it only after NVIDIA redistribution and Release approval.")
    else:
        key = Ed25519PublicKey.from_public_bytes(
            base64.b64decode(args.public_key, validate=True))
        envelope = json.loads(manifest_path(args.channel).read_bytes())
        payload = base64.b64decode(envelope["payload"], validate=True)
        signature = base64.b64decode(envelope["signature"], validate=True)
        key.verify(signature, payload)
        print(json.dumps(json.loads(payload), indent=2))


if __name__ == "__main__":
    main()
