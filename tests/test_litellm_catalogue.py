"""litellm_catalogue: the live LiteLLM model catalogue, fetched by setup and read with a snapshot fallback."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from collections.abc import Sequence
from datetime import UTC, datetime
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("litellm_catalogue", KIT_ROOT / "scripts" / "litellm_catalogue.py")
assert spec and spec.loader
cat = importlib.util.module_from_spec(spec)
sys.modules["litellm_catalogue"] = cat
spec.loader.exec_module(cat)

NOW = datetime(2026, 11, 14, 12, 0, tzinfo=UTC)


def _model(id_: str, status: str, *, free: bool = False, reason: str = "") -> dict:
    return {"id": id_, "metadata": {"status": status, "reason": reason, "free": free, "modality": "chat"}}


CATALOGUE = {
    "version": 1,
    "updated_at": "2026-11-14T11:17:01Z",
    "providers": {
        "vibe": {
            "models": [
                _model("vibe/zai-glm-5-3", "usable"),
                _model("vibe/labs-leanstral-1-5", "blocked", reason="Model labs-leanstral-1-5 is a Labs model."),
            ]
        },
        "openrouter": {"models": [_model("openrouter/google/gemma-4-31b-it:free", "usable", free=True)]},
        "google": {"models": [_model("google/gemini-3.5-flash", "usable")]},
    },
    "groups": {
        "glm-5.3": {
            "modality": "chat",
            "public": False,
            "deployments": [{"route": "vibe", "upstream_id": "zai-glm-5-3", "order": 1, "weight": 2}],
        },
        "gemma-4-31b-it": {
            "modality": "chat",
            "public": False,
            "deployments": [{"route": "openrouter", "upstream_id": "google/gemma-4-31b-it:free", "order": 1}],
        },
        "public/gemini-3.5-flash": {
            "modality": "chat",
            "public": True,
            "deployments": [{"route": "google", "upstream_id": "gemini-3.5-flash", "order": 1}],
        },
        "opencode-free/beam": {
            "modality": "chat",
            "public": True,
            "access_groups": ["opencode-free-public"],
            "deployments": [{"route": "opencode-go", "upstream_id": "beam-free", "order": 1}],
        },
    },
}


def _write(path: Path, data: object) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))
    return path


def test_the_live_copy_lives_in_the_cache_home() -> None:
    assert cat.live_catalogue_path({"XDG_CACHE_HOME": "/x/cache", "HOME": "/h"}) == Path(
        "/x/cache/agent-kit/litellm-catalogue.json"
    )
    assert cat.live_catalogue_path({"XDG_CACHE_HOME": "rel", "HOME": "/h"}) == Path(
        "/h/.cache/agent-kit/litellm-catalogue.json"
    )
    assert cat.live_catalogue_path({"HOME": "/h"}) == Path("/h/.cache/agent-kit/litellm-catalogue.json")


def test_a_fresh_live_copy_is_read_without_a_warning(tmp_path: Path) -> None:
    live = _write(tmp_path / "live.json", CATALOGUE)
    loaded = cat.load_catalogue(live, tmp_path / "none.json", NOW)
    assert (loaded.source, loaded.warnings) == ("live", ())


def test_a_stale_live_copy_is_used_with_a_warning(tmp_path: Path) -> None:
    live = _write(tmp_path / "live.json", {**CATALOGUE, "updated_at": "2026-11-10T12:00:00Z"})
    loaded = cat.load_catalogue(live, tmp_path / "none.json", NOW)
    assert loaded.source == "live"
    assert len(loaded.warnings) == 1 and "stale" in loaded.warnings[0] and "96 hours" in loaded.warnings[0]


def test_an_undated_live_copy_warns_that_its_age_is_unknown(tmp_path: Path) -> None:
    live = _write(tmp_path / "live.json", {**CATALOGUE, "updated_at": "yesterday"})
    assert "age is unknown" in cat.load_catalogue(live, tmp_path / "none.json", NOW).warnings[0]


def test_a_missing_live_copy_falls_back_to_the_snapshot_with_a_warning(tmp_path: Path) -> None:
    snapshot = _write(tmp_path / "snap.json", CATALOGUE)
    loaded = cat.load_catalogue(tmp_path / "missing.json", snapshot, NOW)
    assert loaded.source == "snapshot"
    assert "run setup to fetch it" in loaded.warnings[0] and "using the snapshot" in loaded.warnings[1]


def test_a_broken_live_copy_falls_back_too(tmp_path: Path) -> None:
    snapshot = _write(tmp_path / "snap.json", CATALOGUE)
    (tmp_path / "live.json").write_text("{not json")
    _write(tmp_path / "shape.json", {"updated_at": "x", "providers": {"vibe": {}}})
    assert cat.load_catalogue(tmp_path / "live.json", snapshot, NOW).source == "snapshot"
    loaded = cat.load_catalogue(tmp_path / "shape.json", snapshot, NOW)
    assert "provider vibe has no models list" in loaded.warnings[0]


def test_nothing_readable_is_an_empty_catalogue_never_an_error(tmp_path: Path) -> None:
    loaded = cat.load_catalogue(tmp_path / "a.json", tmp_path / "b.json", NOW)
    assert loaded.source == "none" and len(loaded.warnings) == 2
    assert cat.provider_models(loaded, "vibe") == [] and cat.canonical_groups(loaded, ["canonical"], []) == []


def test_the_shipped_snapshot_is_a_catalogue() -> None:
    assert cat.validate(json.loads(cat.SNAPSHOT.read_text())) is None


def test_provider_models_keep_status_reason_and_free_mark() -> None:
    catalogue = cat.Catalogue(CATALOGUE, "live")
    vibe = cat.provider_models(catalogue, "vibe")
    assert [(m.id, m.status, m.usable) for m in vibe] == [
        ("zai-glm-5-3", "usable", True),
        ("labs-leanstral-1-5", "blocked", False),
    ]
    assert vibe[1].reason == "Model labs-leanstral-1-5 is a Labs model."
    assert cat.provider_models(catalogue, "openrouter")[0].free is True
    assert cat.usable_ids(catalogue, "vibe") == ["zai-glm-5-3"]
    assert cat.provider_models(catalogue, "nowhere") == []


def test_a_canonical_key_reaches_the_canonical_groups_only() -> None:
    catalogue = cat.Catalogue(CATALOGUE, "live")
    groups = cat.canonical_groups(catalogue, ["canonical"], ["google-free-public"])
    assert [(g.name, g.free, g.routes) for g in groups] == [
        ("gemma-4-31b-it", True, ("openrouter",)),
        ("glm-5.3", False, ("vibe",)),
    ]


def test_a_public_group_needs_a_public_only_access_group() -> None:
    catalogue = cat.Catalogue(CATALOGUE, "live")
    names = [
        g.name for g in cat.canonical_groups(catalogue, ["google-free-public", "canonical"], ["google-free-public"])
    ]
    assert names == ["gemma-4-31b-it", "glm-5.3", "public/gemini-3.5-flash"]
    assert all(g.free for g in cat.canonical_groups(catalogue, ["google-free-public"], ["google-free-public"]))


def test_a_group_naming_its_own_access_groups_is_gated_by_them() -> None:
    catalogue = cat.Catalogue(CATALOGUE, "live")
    assert [g.name for g in cat.canonical_groups(catalogue, ["opencode-free-public"], [])] == ["opencode-free/beam"]
    assert "opencode-free/beam" not in [g.name for g in cat.canonical_groups(catalogue, ["canonical"], ["x"])]


def test_the_public_only_access_groups_come_from_routes_json(tmp_path: Path) -> None:
    routes = _write(
        tmp_path / "routes.json",
        {
            "routes": [
                {"agent": "vibe", "data_scope": "any", "models": ["canonical"]},
                {"agent": "public", "data_scope": "public", "models": ["google-free-public", "canonical"]},
                {"agent": "openrouter", "data_scope": "public", "models": ["openrouter-free"]},
            ]
        },
    )
    assert cat.public_access_groups(routes) == frozenset({"google-free-public", "openrouter-free"})
    assert cat.public_access_groups(tmp_path / "missing.json") == frozenset()
    assert "google-free-public" in cat.public_access_groups()


def _runner(code: int, stdout: str = "", stderr: str = ""):
    calls: list[Sequence[str]] = []

    def run(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(list(argv), code, stdout, stderr)

    return run, calls


def test_fetch_writes_the_published_file_verbatim(tmp_path: Path) -> None:
    text = json.dumps(CATALOGUE, indent=2)
    run, calls = _runner(0, text)
    dest = tmp_path / "cache" / "agent-kit" / "litellm-catalogue.json"
    assert cat.fetch(dest, run) is None
    assert dest.read_text() == text
    assert calls[0][:2] == ["gh", "api"] and calls[0][-1].endswith("litellm-model-catalog/contents/model-catalog.json")


def test_a_failed_fetch_keeps_the_old_copy(tmp_path: Path) -> None:
    dest = _write(tmp_path / "live.json", CATALOGUE)
    before = dest.read_text()
    assert "HTTP 404" in (cat.fetch(dest, _runner(1, stderr="gh: Not Found (HTTP 404)\n")[0]) or "")
    assert "not JSON" in (cat.fetch(dest, _runner(0, "<html>")[0]) or "")
    assert "not a catalogue" in (cat.fetch(dest, _runner(0, "[]")[0]) or "")

    def broken(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
        raise FileNotFoundError("gh")

    assert "cannot run gh" in (cat.fetch(dest, broken) or "")
    assert dest.read_text() == before


def test_show_prints_each_model_with_its_status(tmp_path: Path, monkeypatch, capsys) -> None:
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path))
    _write(tmp_path / "agent-kit" / "litellm-catalogue.json", {**CATALOGUE, "updated_at": "2020-01-01T00:00:00Z"})
    assert cat.main(["show", "--provider", "vibe"]) == 0
    out = capsys.readouterr()
    assert "vibe/labs-leanstral-1-5\tblocked: Model labs-leanstral-1-5 is a Labs model." in out.out
    assert "stale" in out.err


def _setup(tmp_path: Path, gh_body: str):
    from tests.setup_harness import setup_run

    return setup_run(
        tmp_path,
        "Linux",
        stubs={"gh": gh_body, "python3": f'exec "{sys.executable}" "$@"'},
    )


def test_setup_fetches_the_live_catalogue_into_the_cache(tmp_path: Path) -> None:
    published = tmp_path / "published.json"
    published.write_text(json.dumps(CATALOGUE))
    run = _setup(tmp_path, f'[ "$1" = api ] && case "$*" in *litellm-model-catalog*) cat "{published}" ;; esac')
    live = run.home / ".cache" / "agent-kit" / "litellm-catalogue.json"
    assert live.read_text() == published.read_text(), run.stdout + run.stderr
    assert "LiteLLM catalogue written to" in run.stdout + run.stderr


def test_a_failed_fetch_is_a_setup_warning_not_a_failure(tmp_path: Path) -> None:
    run = _setup(tmp_path, 'echo "gh: Not Found (HTTP 404)" >&2; exit 1')
    assert not (run.home / ".cache" / "agent-kit" / "litellm-catalogue.json").exists()
    assert "LiteLLM catalogue: not refreshed" in run.stdout + run.stderr
    assert "HTTP 404" in run.stdout + run.stderr
