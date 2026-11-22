#!/usr/bin/env python3
"""Sign optional EvoParseq NGX bridge manifests offline. Never commit a private key.

Requires: pip install cryptography
Examples:
 python scripts/ngx-bridge-manifest.py keygen --private ngx-signing-key.pem
 python scripts/ngx-bridge-manifest.py sign --private ngx-signing-key.pem --channel dev \
   --sequence 1 --version 0.9.9-dev.2 --tag v0.9.9-dev.2 \
   --dll EvoParseq-NGX-Bridge.dll --out ngx-bridge/channels/dev.json
 python scripts/ngx-bridge-manifest.py verify --public-key BASE64KEY \
   --manifest ngx-bridge/channels/dev.json
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path

DLL_NAME = "EvoParseq-NGX-Bridge.dll"
ASSET_BASE = "https://github.com/vampywiz17/EvoParseq/releases/download"
MAX_SIZE = 16 * 1024 * 1024


def pack(data):
    return json.dumps(data, sort_keys=True, separators=(",", ":")).encode("utf-8")


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
    keygen = cmds.add_parser("keygen")
    keygen.add_argument("--private", required=True)
    sign = cmds.add_parser("sign")
    sign.add_argument("--private", required=True)
    sign.add_argument("--channel", required=True, choices=["dev", "stable"])
    sign.add_argument("--sequence", type=int, required=True)
    sign.add_argument("--version", required=True)
    sign.add_argument("--tag", required=True)
    sign.add_argument("--dll", type=Path, required=True)
    sign.add_argument("--out", type=Path, required=True)
    verify = cmds.add_parser("verify")
    verify.add_argument("--public-key", required=True)
    verify.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    serialization, Ed25519PrivateKey, Ed25519PublicKey = crypto()
    if args.command == "keygen":
        private = Ed25519PrivateKey.generate()
        data = private.private_bytes(serialization.Encoding.PEM,
                                     serialization.PrivateFormat.PKCS8,
                                     serialization.NoEncryption())
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        fd = os.open(args.private, flags, 0o600)
        with os.fdopen(fd, "wb") as fp:
            fp.write(data)
        public = private.public_key().public_bytes(serialization.Encoding.Raw,
                                                   serialization.PublicFormat.Raw)
        print("Public key for GitHub Actions variable NGX_MANIFEST_PUBLIC_KEY_B64:")
        print(base64.b64encode(public).decode("ascii"))
        print("Keep the private signing key OFFLINE, never in Git or artifacts.")
    elif args.command == "sign":
        if args.sequence <= 0:
            parser.error("sequence must be positive and monotonically increasing")
        if not args.version or not all(c.isalnum() or c in "._-" for c in args.version):
            parser.error("invalid version")
        if not args.tag or not all(c.isalnum() or c in "._-" for c in args.tag):
            parser.error("invalid release tag")
        data = args.dll.read_bytes()
        if not 0 < len(data) <= MAX_SIZE:
            parser.error("invalid DLL size")
        private = serialization.load_pem_private_key(Path(args.private).read_bytes(), password=None)
        if not isinstance(private, Ed25519PrivateKey):
            parser.error("signing key must be Ed25519")
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
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_bytes(pack(envelope) + b"\n")
        print(f"Signed {args.channel} manifest for {payload['sha256']}; publish only after licensing approval.")
    else:
        key = Ed25519PublicKey.from_public_bytes(base64.b64decode(args.public_key, validate=True))
        envelope = json.loads(args.manifest.read_bytes())
        payload = base64.b64decode(envelope["payload"], validate=True)
        signature = base64.b64decode(envelope["signature"], validate=True)
        key.verify(signature, payload)
        print(json.dumps(json.loads(payload), indent=2))


if __name__ == "__main__":
    main()
