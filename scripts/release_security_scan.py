"""Scan published release assets; never build, install or execute their contents."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from urllib.parse import quote, urlsplit


VT_API = "https://www.virustotal.com/api/v3/"
START = "<!-- evoparseq-security-scan:start -->"
END = "<!-- evoparseq-security-scan:end -->"
REPORTS = ("SECURITY-SCAN.md", "SECURITY-SCAN.json")
MAX_SIZE = 650 * 1024 * 1024


class ScanError(Exception):
    """Safe diagnostic: never include server responses, credentials or upload URLs."""


def command(args, **kwargs):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=240, **kwargs)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ScanError(f"{args[0]} could not complete") from error
    if result.returncode:
        raise ScanError(f"{args[0]} failed (exit {result.returncode})")
    return result.stdout


def github(endpoint, method="GET", payload=None):
    args = ["gh", "api", "--method", method, endpoint]
    if payload is not None:
        args += ["--input", "-"]
    return json.loads(command(args, input=json.dumps(payload) if payload is not None else None))


def safe_name(value):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,159}", value):
        raise ScanError("Unsupported release tag or asset name")
    return value


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def select_assets(release):
    assets = [a for a in release["assets"] if a["name"].lower().endswith((".exe", ".apk", ".flatpak"))]
    if not assets or len(assets) > 20:
        raise ScanError("Expected between 1 and 20 EXE, APK or Flatpak release assets")
    for asset in assets:
        safe_name(asset["name"])
    if len({a["name"] for a in assets}) != len(assets):
        raise ScanError("Duplicate release asset names")
    return assets


def download(repo, tag, asset, directory):
    if not 0 < asset["size"] <= MAX_SIZE:
        raise ScanError("Asset exceeds the supported upload size or is empty")
    digest = asset.get("digest", "") or ""
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
        raise ScanError("Release asset has no valid GitHub SHA-256 digest")
    command(["gh", "release", "download", tag, "--repo", repo,
             f"--pattern={asset['name']}", "--dir", str(directory)])
    path = directory / asset["name"]
    if path.stat().st_size != asset["size"] or sha256(path) != digest[7:]:
        raise ScanError("Downloaded asset does not match GitHub size and SHA-256")
    return path


class VirusTotal:
    def __init__(self, key):
        if not re.fullmatch(r"[A-Za-z0-9_-]{20,200}", key):
            raise ScanError("VIRUSTOTAL secret is missing or invalid")
        self.key = key
        self.next_request = 0.0

    def request(self, url, file=None):
        parsed = urlsplit(url)
        if (parsed.scheme != "https" or parsed.hostname != "www.virustotal.com"
                or parsed.port not in (None, 443) or parsed.username or parsed.password or parsed.fragment):
            raise ScanError("VirusTotal returned an untrusted endpoint")
        for attempt in range(3):
            time.sleep(max(0, self.next_request - time.monotonic()))
            self.next_request = time.monotonic() + 16
            args = ["curl", "--silent", "--show-error", "--proto", "=https",
                    "--connect-timeout", "30", "--max-time", "180", "--max-redirs", "0",
                    "--config", "-", "--url", url, "--write-out", "\n%{http_code}"]
            if file is not None:
                args += ["--form", f"file=@{file};filename=release{file.suffix}"]
            # The key is sent on stdin, never in command arguments or log output.
            output = command(args, input=f'header = "x-apikey: {self.key}"\n')
            body, status = output.rsplit("\n", 1)
            if status == "429" and attempt < 2:
                time.sleep(60)
                continue
            if status not in ("200", "201"):
                raise ScanError(f"VirusTotal HTTP {status}")
            try:
                return json.loads(body)
            except json.JSONDecodeError as error:
                raise ScanError("Invalid VirusTotal JSON response") from error

    def scan(self, path, digest):
        url = VT_API + "files"
        if path.stat().st_size > 32 * 1024 * 1024:
            url = self.request(VT_API + "files/upload_url")["data"]
        analysis_id = self.request(url, file=path)["data"]["id"]
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            result = self.request(VT_API + "analyses/" + quote(analysis_id, safe=""))
            info = result.get("meta", {}).get("file_info", {})
            if info.get("sha256", digest) != digest:
                raise ScanError("VirusTotal analysis hash mismatch")
            attributes = result["data"]["attributes"]
            if attributes["status"] == "completed":
                return {"analysis_id": analysis_id, "status": "completed",
                        "date": attributes.get("date"), "stats": attributes["stats"],
                        "results": attributes["results"]}
            time.sleep(30)
        raise ScanError("VirusTotal analysis did not complete within 10 minutes")


def flatpak_payload(bundle, directory):
    repo = directory / "ostree"
    command(["ostree", f"--repo={repo}", "init", "--mode=archive-z2"])
    command(["flatpak", "build-import-bundle", str(repo), str(bundle)])
    refs = command(["ostree", f"--repo={repo}", "refs"]).splitlines()
    refs = [r for r in refs if r.startswith("app/io.github.vampywiz17.EvoParseq/")]
    if len(refs) != 1:
        raise ScanError("Flatpak must contain exactly one EvoParseq application ref")
    commit = command(["ostree", f"--repo={repo}", "rev-parse", refs[0]]).strip()
    if not re.fullmatch(r"[0-9a-f]{64}", commit):
        raise ScanError("Invalid Flatpak commit")
    checkout = directory / "checkout"
    command(["ostree", f"--repo={repo}", "checkout", "--user-mode", commit, str(checkout)])
    path = checkout / "files/bin/evoparseq-linux"
    if path.is_symlink() or not path.resolve().is_relative_to(checkout.resolve()):
        raise ScanError("Flatpak executable escapes its checkout")
    with path.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            raise ScanError("Flatpak application executable is not ELF")
    return path


def scan_file(scanner, path, name, cache):
    digest = sha256(path)
    row = {"name": name, "size": path.stat().st_size, "sha256": digest,
           "report_url": f"https://www.virustotal.com/gui/file/{digest}"}
    try:
        if digest not in cache:
            cache[digest] = scanner.scan(path, digest)
        row.update(cache[digest])
    except (ScanError, KeyError, OSError) as error:
        row.update(status="error", error=safe_error(error))
    return row


def safe_error(error):
    return str(error) if isinstance(error, ScanError) else "Incomplete response or unreadable file"


def needs_attention(row):
    stats = row.get("stats", {})
    return (row["status"] != "completed" or stats.get("malicious", 0) > 0
            or stats.get("suspicious", 0) > 0
            or stats.get("undetected", 0) + stats.get("harmless", 0) == 0)


def markdown(report):
    lines = ["## Release malware scan", "", f"Report generated: {report['generated_at']} (UTC).", "",
             "Only the release assets listed below and the named Flatpak executable were submitted.",
             "ZIP wrappers and external Flatpak runtimes/drivers are not separately scanned.", "",
             "No detection is not a safety guarantee or Google Play Protect approval.",
             "Unsupported engines, errors and timeouts are reported separately, not counted as clean.", "",
             "| File | SHA-256 / report | Status | Malicious | Suspicious | Undetected / harmless | Unsupported | Failed / timeout |",
             "| --- | --- | --- | ---: | ---: | --- | ---: | --- |"]
    for row in report["files"]:
        stats = row.get("stats", {})
        status = row["status"]
        if status == "completed":
            status = "ATTENTION" if needs_attention(row) else "Completed"
        link = f"[{row['sha256']}]({row['report_url']})" if row.get("sha256") else "Not submitted"
        lines.append(f"| {row['name']} | {link} | {status} | {stats.get('malicious', '-')} | "
                     f"{stats.get('suspicious', '-')} | {stats.get('undetected', '-')} / {stats.get('harmless', '-')} | "
                     f"{stats.get('type-unsupported', '-')} | {stats.get('failure', 0)} / "
                     f"{stats.get('timeout', 0) + stats.get('confirmed-timeout', 0)} |")
        if row.get("error"):
            lines += ["", f"{row['name']}: {row['error']}", ""]
    return "\n".join(lines) + "\n"


def replace_section(body, section):
    if START in body or END in body:
        if body.count(START) != 1 or body.count(END) != 1 or body.index(END) < body.index(START):
            raise ScanError("Ambiguous security report markers; release notes were preserved")
        before, rest = body.split(START)
        _, after = rest.split(END)
        return before + START + "\n" + section + END + after
    return body.rstrip() + "\n\n" + START + "\n" + section + END + "\n"


def asset_identity(assets):
    # Downloads change download_count; it is not part of immutable file identity.
    return [{key: asset.get(key) for key in ("id", "name", "size", "digest", "updated_at")}
            for asset in assets]


def scan_release(repo, release_id, scanner, directory, publish=False):
    endpoint = f"repos/{repo}/releases/{release_id}"
    release = github(endpoint)
    if github(f"repos/{repo}")["private"]:
        raise ScanError("Private repository files may not be submitted")
    if publish and not release["draft"]:
        raise ScanError("Publication gate requires a draft release")
    if not publish and (release["draft"] or not release.get("published_at")):
        raise ScanError("Published-release audit requires a published release")
    tag = safe_name(release["tag_name"])
    assets = select_assets(release)
    report = {"release": release["html_url"], "tag": tag,
              "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "files": []}
    cache = {}
    blocked = False
    for index, asset in enumerate(assets):
        if blocked:
            report["files"].append({"name": asset["name"], "status": "not_scanned",
                                    "error": "Stopped after a blocking scan result"})
            continue
        work = directory / str(index)
        work.mkdir()
        try:
            path = download(repo, tag, asset, work)
            report["files"].append(scan_file(scanner, path, asset["name"], cache))
            if needs_attention(report["files"][-1]):
                blocked = True
                continue
            if path.suffix.lower() == ".flatpak":
                name = asset["name"] + " / files/bin/evoparseq-linux"
                try:
                    payload = flatpak_payload(path, work)
                    report["files"].append(scan_file(scanner, payload, name, cache))
                except (ScanError, OSError) as error:
                    report["files"].append({"name": name, "status": "error", "error": safe_error(error)})
        except (ScanError, OSError) as error:
            report["files"].append({"name": asset["name"], "status": "error", "error": safe_error(error)})
        blocked = any(needs_attention(row) for row in report["files"])
    latest = github(endpoint)
    if asset_identity(select_assets(latest)) != asset_identity(assets):
        raise ScanError("Release assets changed during analysis; results were not published")
    if latest["draft"] != release["draft"]:
        raise ScanError("Release publication state changed during analysis")
    text = markdown(report)
    notes = replace_section(latest.get("body") or "", text)
    output = Path("security-scan-results")
    output.mkdir(exist_ok=True)
    (output / REPORTS[0]).write_text(text, encoding="utf-8")
    (output / REPORTS[1]).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    command(["gh", "release", "upload", tag, "--repo", repo, "--clobber",
             *(str(output / name) for name in REPORTS)])
    update = {"body": notes}
    if publish and not blocked:
        update.update(draft=False, make_latest="false" if latest.get("prerelease") else "true")
    github(endpoint, "PATCH", update)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with Path(summary).open("a", encoding="utf-8") as stream:
            stream.write(text)
    return int(blocked)


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ScanError("Invalid repository")
    release_id = int(os.environ["RELEASE_ID"])
    scanner = VirusTotal(os.environ.get("VIRUSTOTAL", ""))
    with tempfile.TemporaryDirectory(prefix="release-scan-") as work:
        return scan_release(repo, release_id, scanner, Path(work),
                            publish=os.environ.get("PUBLISH_RELEASE") == "true")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ScanError, KeyError, ValueError) as failure:
        print("Release scan failed: " + safe_error(failure))
        raise SystemExit(1) from None
