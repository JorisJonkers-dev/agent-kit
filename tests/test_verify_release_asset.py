"""Tests for scripts/verify_release_asset.py."""

import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from verify_release_asset import (  # noqa: E402
    RELEASE_TAG_PLACEHOLDER,
    check_checksum_file,
    check_version,
    extract_version,
    parse_sha256sum_line,
    sha256_of,
    stamp_version,
)

SCRIPT = (
    "#!/usr/bin/env bash\n# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.\nset -uo pipefail\n"
    f"AGENT_KIT_RELEASE_TAG='{RELEASE_TAG_PLACEHOLDER}'\n"
)


class TestStampAndExtractVersion:
    def test_stamp_inserts_token_after_shebang(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        lines = stamped.splitlines()
        assert lines[0] == "#!/usr/bin/env bash"
        assert lines[1] == "# agent-kit v4.1.0"
        assert lines[2:-1] == SCRIPT.splitlines()[1:-1]

    def test_stamp_sets_the_release_tag_the_script_fetches_its_bundle_by(self):
        stamped = stamp_version(SCRIPT, "v4.1.0")
        assert stamped.splitlines()[-1] == "AGENT_KIT_RELEASE_TAG='v4.1.0'"
        assert RELEASE_TAG_PLACEHOLDER not in stamped

    def test_stamp_rejects_a_script_without_the_release_tag_placeholder(self):
        with pytest.raises(ValueError, match="placeholder"):
            stamp_version("#!/usr/bin/env bash\necho hi\n", "v4.1.0")

    def test_stamp_rejects_a_tag_that_would_break_the_quoting(self):
        with pytest.raises(ValueError, match="tag"):
            stamp_version(SCRIPT, "v4.1.0'; rm -rf /; '")

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

    def test_fails_when_the_release_tag_does_not_match(self):
        stamped = stamp_version(SCRIPT, "v4.1.0").replace("TAG='v4.1.0'", "TAG='v4.0.0'")
        with pytest.raises(SystemExit, match="release tag mismatch"):
            check_version(stamped, "v4.1.0")


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
