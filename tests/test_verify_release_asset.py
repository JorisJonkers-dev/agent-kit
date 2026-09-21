"""Tests for scripts/verify_release_asset.py."""

import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from verify_release_asset import (  # noqa: E402
    check_checksum_file,
    check_version,
    extract_version,
    parse_sha256sum_line,
    sha256_of,
    stamp_version,
)

SCRIPT = "#!/usr/bin/env bash\n# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.\nset -uo pipefail\n"


class TestStampAndExtractVersion:
    def test_stamp_inserts_token_after_shebang(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        lines = stamped.splitlines()
        assert lines[0] == "#!/usr/bin/env bash"
        assert lines[1] == "# agent-kit v4.1.0"
        assert lines[2:] == SCRIPT.splitlines()[1:]

    def test_extract_reads_back_what_stamp_wrote(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        assert extract_version(stamped) == "v4.1.0"

    def test_extract_returns_none_when_unstamped(self):
        assert extract_version(SCRIPT) is None

    def test_stamp_rejects_missing_shebang(self):
        with pytest.raises(ValueError, match="shebang"):
            stamp_version("echo hi\n", "v4.1.0")


class TestCheckVersion:
    def test_passes_on_match(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        check_version(stamped, "v4.1.0")  # no raise

    def test_fails_on_mismatch(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        with pytest.raises(SystemExit, match="version token mismatch"):
            check_version(stamped, "v4.2.0")

    def test_fails_when_unstamped(self):
        with pytest.raises(SystemExit, match="version token mismatch"):
            check_version(SCRIPT, "v4.1.0")


class TestParseSha256sumLine:
    def test_parses_binary_mode_line(self):
        digest, name = parse_sha256sum_line("abc123  agent-kit-skills.tar.gz\n")
        assert digest == "abc123"
        assert name == "agent-kit-skills.tar.gz"

    def test_strips_leading_asterisk(self):
        # sha256sum prefixes the name with `*` in binary mode on some platforms.
        _, name = parse_sha256sum_line("abc123  *agent-kit-skills.tar.gz")
        assert name == "agent-kit-skills.tar.gz"

    def test_rejects_malformed_line(self):
        with pytest.raises(ValueError, match="not a sha256sum line"):
            parse_sha256sum_line("not-a-checksum-line")


class TestCheckChecksumFile:
    def test_passes_when_hash_and_name_match(self, tmp_path: Path):
        tarball = tmp_path / "agent-kit-skills.tar.gz"
        tarball.write_bytes(b"fake tarball contents")
        digest = sha256_of(tarball)
        checksum_text = f"{digest}  agent-kit-skills.tar.gz\n"
        check_checksum_file(checksum_text, tarball)  # no raise

    def test_fails_on_name_mismatch(self, tmp_path: Path):
        tarball = tmp_path / "agent-kit-skills.tar.gz"
        tarball.write_bytes(b"fake tarball contents")
        digest = sha256_of(tarball)
        checksum_text = f"{digest}  wrong-name.tar.gz\n"
        with pytest.raises(SystemExit, match="checksum file names"):
            check_checksum_file(checksum_text, tarball)

    def test_fails_on_hash_mismatch(self, tmp_path: Path):
        tarball = tmp_path / "agent-kit-skills.tar.gz"
        tarball.write_bytes(b"fake tarball contents")
        checksum_text = f"{'0' * 64}  agent-kit-skills.tar.gz\n"
        with pytest.raises(SystemExit, match="checksum mismatch"):
            check_checksum_file(checksum_text, tarball)
