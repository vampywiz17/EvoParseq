"""Synthetic release and HTTP fixtures: no credentials, upload or release edits."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location(
    "release_security_scan", Path(__file__).resolve().parents[1] / "scripts/release_security_scan.py")
scan = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scan)
KEY = "a" * 64  # Synthetic placeholder, never sent to a service.
HASH = "b" * 64
STATS = {"malicious": 0, "suspicious": 0, "undetected": 60, "harmless": 0,
         "type-unsupported": 5, "timeout": 1, "confirmed-timeout": 0, "failure": 0}
DONE = {"status": "completed", "date": 123, "stats": STATS, "results": {}}


def asset(name="EvoParseq.exe", payload=b"fixture"):
    import hashlib
    return {"id": 7, "name": name, "size": len(payload),
            "digest": "sha256:" + hashlib.sha256(payload).hexdigest(), "download_count": 0}


def release(assets=None):
    return {"draft": False, "published_at": "2026-01-01", "tag_name": "v1.2.3-dev.1",
            "html_url": "https://github.com/example/repo/releases/tag/v1.2.3-dev.1",
            "body": "Existing release notes", "assets": [asset()] if assets is None else assets}


class Fixtures(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_commands_do_not_expose_output_on_failure(self):
        with patch.object(scan.subprocess, "run", return_value=Mock(returncode=0, stdout="ok")):
            self.assertEqual(scan.command(["fixture"]), "ok")
        for response in [Mock(returncode=1, stderr="SECRET"), OSError("SECRET"),
                         subprocess.TimeoutExpired("SECRET", 1)]:
            kwargs = {"side_effect": response} if isinstance(response, Exception) else {"return_value": response}
            with patch.object(scan.subprocess, "run", **kwargs):
                with self.assertRaises(scan.ScanError) as caught:
                    scan.command(["fixture"])
                self.assertNotIn("SECRET", str(caught.exception))

    def test_github_uses_structured_input(self):
        with patch.object(scan, "command", return_value='{"ok":true}') as run:
            self.assertEqual(scan.github("repos/a/b", "PATCH", {"body": "a\nb"}), {"ok": True})
            self.assertIn("--input", run.call_args.args[0])
            self.assertEqual(json.loads(run.call_args.kwargs["input"]), {"body": "a\nb"})

    def test_asset_scope_and_names(self):
        rows = [asset(n) for n in ["a.exe", "b.apk", "c.flatpak", "ignored.zip", "SECURITY-SCAN.json"]]
        self.assertEqual(len(scan.select_assets(release(rows))), 3)
        for name in ["../secret.apk", "x[1].apk", "--x.exe", "x;touch.apk"]:
            with self.assertRaises(scan.ScanError):
                scan.safe_name(name)
        for rows in [[], [asset()] * 2, [asset(f"{n}.exe") for n in range(21)]]:
            with self.assertRaises(scan.ScanError):
                scan.select_assets(release(rows))

    def test_download_verifies_publication_digest_and_size(self):
        def download(_args):
            (self.root / "EvoParseq.exe").write_bytes(b"fixture")
        with patch.object(scan, "command", side_effect=download):
            path = scan.download("a/b", "v1", asset(), self.root)
            self.assertEqual(path.read_bytes(), b"fixture")
            for changed in [{"size": 0}, {"size": scan.MAX_SIZE + 1}, {"digest": None},
                            {"size": 1}, {"digest": "sha256:" + "0" * 64}]:
                with self.assertRaises(scan.ScanError):
                    scan.download("a/b", "v1", asset() | changed, self.root)

    def test_api_key_and_upload_endpoint_validation(self):
        for key in ["", "short", "a" * 64 + '\nheader = "evil"']:
            with self.assertRaises(scan.ScanError):
                scan.VirusTotal(key)
        client = scan.VirusTotal(KEY)
        for url in ["http://www.virustotal.com/x", "https://evil.invalid/x",
                    "https://www.virustotal.com:444/x", "https://user@www.virustotal.com/x",
                    "https://www.virustotal.com/x#fragment"]:
            with self.assertRaises(scan.ScanError):
                client.request(url)

    @patch.object(scan.time, "sleep")
    def test_requests_are_throttled_and_key_is_stdin_only(self, sleep):
        client = scan.VirusTotal(KEY)
        with patch.object(scan, "command", return_value='{"data":1}\n200') as run:
            client.next_request = scan.time.monotonic() + 10
            self.assertEqual(client.request(scan.VT_API + "files", self.root / "a.apk"), {"data": 1})
            self.assertNotIn(KEY, " ".join(run.call_args.args[0]))
            self.assertIn(KEY, run.call_args.kwargs["input"])
            self.assertGreater(sleep.call_args.args[0], 0)

    @patch.object(scan.time, "sleep")
    def test_quota_backoff_and_errors_are_bounded(self, _sleep):
        client = scan.VirusTotal(KEY)
        with patch.object(scan, "command", side_effect=["{}\n429", "{}\n200"]):
            self.assertEqual(client.request(scan.VT_API + "files"), {})
        for output in ["{}\n403", "not JSON\n200", "{}\n429"]:
            with patch.object(scan, "command", return_value=output) as run:
                with self.assertRaises(scan.ScanError):
                    client.request(scan.VT_API + "files")
                self.assertLessEqual(run.call_count, 3)

    @patch.object(scan.time, "sleep")
    def test_analysis_waits_for_completion_and_preserves_findings(self, _sleep):
        path = self.root / "file.apk"
        path.write_bytes(b"fixture")
        client = scan.VirusTotal(KEY)
        response = {"data": {"attributes": DONE}, "meta": {"file_info": {"sha256": HASH}}}
        queued = {"data": {"attributes": {"status": "queued"}}}
        with patch.object(client, "request", side_effect=[{"data": {"id": "analysis"}}, queued, response]):
            self.assertEqual(client.scan(path, HASH)["stats"], STATS)
        with patch.object(client, "request", side_effect=[{"data": {"id": "analysis"}}, response]):
            with self.assertRaisesRegex(scan.ScanError, "hash mismatch"):
                client.scan(path, "0" * 64)
        with patch.object(scan.time, "monotonic", side_effect=[0, 601]), \
                patch.object(client, "request", return_value={"data": {"id": "analysis"}}):
            with self.assertRaisesRegex(scan.ScanError, "10 minutes"):
                client.scan(path, HASH)

    @patch.object(scan.time, "sleep")
    def test_large_files_use_the_dedicated_upload_endpoint(self, _sleep):
        path = Mock(suffix=".apk")
        path.stat.return_value.st_size = 33 * 1024 * 1024
        client = scan.VirusTotal(KEY)
        with patch.object(client, "request", side_effect=[{"data": scan.VT_API + "large"},
                          {"data": {"id": "analysis"}}, {"data": {"attributes": DONE}}]) as request:
            client.scan(path, HASH)
            self.assertEqual(request.call_args_list[1].args[0], scan.VT_API + "large")

    def test_same_hash_scanned_once_and_failures_do_not_turn_green(self):
        path = self.root / "a.exe"
        path.write_bytes(b"fixture")
        client = Mock()
        client.scan.return_value = DONE
        cache = {}
        first = scan.scan_file(client, path, "a.exe", cache)
        scan.scan_file(client, path, "b.exe", cache)
        self.assertEqual(client.scan.call_count, 1)
        self.assertFalse(scan.needs_attention(first))
        client.scan.side_effect = scan.ScanError("API unavailable")
        self.assertTrue(scan.needs_attention(scan.scan_file(client, path, "a.exe", {})))
        client.scan.side_effect = KeyError("sensitive response")
        row = scan.scan_file(client, path, "a.exe", {})
        self.assertNotIn("sensitive", row["error"])
        for stats in [STATS | {"malicious": 1}, STATS | {"suspicious": 1}, {"type-unsupported": 70}]:
            self.assertTrue(scan.needs_attention({"status": "completed", "stats": stats}))

    def test_reports_preserve_notes_and_do_not_label_unsupported_clean(self):
        report = {"generated_at": "fixture", "files": [
            {"name": "a.exe", "sha256": HASH, "report_url": "https://example.invalid/report", **DONE},
            {"name": "b.flatpak", "status": "completed", "stats": {"type-unsupported": 70}},
            {"name": "c.apk", "status": "error", "error": "API unavailable"}]}
        text = scan.markdown(report)
        self.assertIn("ATTENTION", text)
        self.assertIn("Not submitted", text)
        self.assertIn("API unavailable", text)
        notes = scan.replace_section("User notes", text)
        self.assertTrue(notes.startswith("User notes"))
        self.assertEqual(scan.replace_section(notes, text), notes)
        self.assertIn("suffix", scan.replace_section(notes + "suffix", "new report"))
        for malformed in [scan.START, scan.END, scan.START * 2 + scan.END, scan.END + scan.START]:
            with self.assertRaises(scan.ScanError):
                scan.replace_section(malformed, text)

    def test_flatpak_payload_is_extracted_never_executed(self):
        path = self.root / "checkout/files/bin/evoparseq-linux"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"\x7fELFfixture")
        outputs = ["", "", "app/io.github.vampywiz17.EvoParseq/x86_64/master", HASH, ""]
        with patch.object(scan, "command", side_effect=outputs) as run:
            self.assertEqual(scan.flatpak_payload(self.root / "a.flatpak", self.root), path)
            self.assertTrue(all(c.args[0][0] in ["ostree", "flatpak"] for c in run.call_args_list))
        for refs, commit, content in [("app/wrong/ref", HASH, b"\x7fELF"),
                                       (outputs[2], "wrong", b"\x7fELF"),
                                       (outputs[2], HASH, b"not ELF")]:
            path.write_bytes(content)
            with patch.object(scan, "command", side_effect=["", "", refs, commit, ""]):
                with self.assertRaises(scan.ScanError):
                    scan.flatpak_payload(self.root / "a.flatpak", self.root)
        with patch.object(scan, "command", side_effect=outputs), \
                patch.object(Path, "is_symlink", return_value=True):
            with self.assertRaisesRegex(scan.ScanError, "escapes"):
                scan.flatpak_payload(self.root / "a.flatpak", self.root)

    @unittest.skipUnless(shutil.which("flatpak") and shutil.which("ostree"), "Linux inspection tools required")
    def test_real_flatpak_roundtrip_without_runtime_installation_or_execution(self):
        tree = self.root / "tree"
        exe = tree / "files/bin/evoparseq-linux"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"\x7fELFsynthetic-not-executable")
        metadata = "[Application]\nname=io.github.vampywiz17.EvoParseq\nruntime=org.freedesktop.Platform/x86_64/25.08\n"
        (tree / "metadata").write_text(metadata)
        repo = self.root / "source-repo"
        scan.command(["ostree", f"--repo={repo}", "init", "--mode=archive-z2"])
        scan.command(["ostree", f"--repo={repo}", "commit", "--subject=Offline scanner fixture",
                      "--branch=app/io.github.vampywiz17.EvoParseq/x86_64/master",
                      f"--tree=dir={tree}", f"--add-metadata-string=xa.metadata={metadata}"])
        bundle = self.root / "fixture.flatpak"
        scan.command(["flatpak", "build-bundle", str(repo), str(bundle), "io.github.vampywiz17.EvoParseq"])
        unpack = self.root / "unpack"
        unpack.mkdir()
        self.assertEqual(scan.flatpak_payload(bundle, unpack).read_bytes(), exe.read_bytes())

    def run_release(self, initial, latest=None, client=None, extraction=None):
        reads = 0
        def api(endpoint, method="GET", payload=None):
            nonlocal reads
            if endpoint == "repos/a/b":
                return {"private": False}
            if method == "PATCH":
                self.assertIn("Existing release notes", payload["body"])
                return {}
            reads += 1
            return initial if reads == 1 else latest or initial
        def download(_repo, _tag, item, work):
            path = work / item["name"]
            path.write_bytes(b"fixture")
            return path
        client = client or Mock(scan=Mock(return_value=DONE))
        previous = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, previous)
        with patch.object(scan, "github", side_effect=api), patch.object(scan, "download", side_effect=download), \
                patch.object(scan, "command") as publish, patch.object(scan, "flatpak_payload", side_effect=extraction), \
                patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": str(self.root / "summary.md")}):
            return scan.scan_release("a/b", 1, client, self.root), publish

    def test_published_release_report_snapshot_and_live_notes(self):
        first = release()
        latest = release([asset() | {"download_count": 42}])
        result, publish = self.run_release(first, latest)
        self.assertEqual(result, 0)
        self.assertIn("--clobber", publish.call_args.args[0])
        report = json.loads((self.root / "security-scan-results/SECURITY-SCAN.json").read_text())
        self.assertEqual(report["files"][0]["stats"], STATS)
        self.assertIn("Release malware scan", (self.root / "summary.md").read_text())

    def test_private_and_draft_releases_are_never_submitted(self):
        for initial, private in [(release() | {"draft": True}, False),
                                 (release() | {"published_at": None}, False), (release(), True)]:
            with patch.object(scan, "github", side_effect=[initial, {"private": private}]):
                with self.assertRaises(scan.ScanError):
                    scan.scan_release("a/b", 1, Mock(), self.root)

    def test_changed_assets_do_not_publish_stale_results(self):
        with self.assertRaisesRegex(scan.ScanError, "changed"):
            self.run_release(release(), release([asset() | {"digest": "sha256:" + HASH}]))

    def test_scan_errors_and_flatpak_extraction_errors_are_published_as_failures(self):
        result, _ = self.run_release(release([asset("a.flatpak")]), extraction=scan.ScanError("Cannot extract"))
        self.assertEqual(result, 1)
        report = json.loads((self.root / "security-scan-results/SECURITY-SCAN.json").read_text())
        self.assertEqual(report["files"][1]["status"], "error")

    def test_flatpak_inner_elf_is_a_separate_report_row(self):
        payload = self.root / "linux-elf"
        payload.write_bytes(b"\x7fELFfixture")
        result, _ = self.run_release(release([asset("a.flatpak")]), extraction=lambda *_: payload)
        self.assertEqual(result, 0)
        report = json.loads((self.root / "security-scan-results/SECURITY-SCAN.json").read_text())
        self.assertEqual(len(report["files"]), 2)
        self.assertNotEqual(report["files"][0]["sha256"], report["files"][1]["sha256"])

    def test_main_validates_configuration_without_printing_secrets(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "a/b", "RELEASE_ID": "1", "VIRUSTOTAL": KEY}), \
                patch.object(scan, "scan_release", return_value=0) as run:
            self.assertEqual(scan.main(), 0)
            self.assertEqual(run.call_args.args[:2], ("a/b", 1))
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "../bad", "RELEASE_ID": "1"}):
            with self.assertRaises(scan.ScanError):
                scan.main()


if __name__ == "__main__":
    unittest.main()
