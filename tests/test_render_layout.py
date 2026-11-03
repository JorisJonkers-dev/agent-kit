from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path
from types import ModuleType
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
RENDERER_PATH = ROOT / "scripts" / "render_agent_kit.py"


def load_renderer() -> ModuleType:
    spec = importlib.util.spec_from_file_location("render_agent_kit_layout", RENDERER_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    # dataclasses resolve the module through sys.modules.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


renderer = load_renderer()


def by_destination(root: Path, *, include_self: bool = False) -> dict[str, Any]:
    return {item.relative_path.as_posix(): item for item in renderer.template_files(root, include_self=include_self)}


def test_a_repo_skill_renders_its_claude_and_codex_variants(tmp_path: Path) -> None:
    files = by_destination(tmp_path)

    claude = files[".claude/skills/council/SKILL.md"]
    codex = files[".agents/skills/council/SKILL.md"]

    assert claude.source == ROOT / "skills/repo/council/claude/SKILL.md"
    assert codex.source == ROOT / "skills/repo/council/codex/SKILL.md"
    assert not claude.committed and not codex.committed


def test_agents_render_into_claude_agents(tmp_path: Path) -> None:
    item = by_destination(tmp_path)[".claude/agents/codex-med.md"]

    assert item.source == ROOT / "agents/codex-med.md"
    assert not item.committed


def test_only_the_runner_runtime_is_committed(tmp_path: Path) -> None:
    committed = {path for path, item in by_destination(tmp_path, include_self=True).items() if item.committed}

    assert committed
    assert all(path.startswith("runner/manifests/runtime/") for path in committed)


def test_the_council_context_renders_only_for_this_checkout(tmp_path: Path) -> None:
    assert ".council/context/pack.json" not in by_destination(tmp_path)
    item = by_destination(tmp_path, include_self=True)[".council/context/pack.json"]

    assert item.source == ROOT / "council/context/pack.json"


def test_no_rendered_destination_is_tracked_outside_the_runner_runtime() -> None:
    listing = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True)
    tracked = set(listing.stdout.split())
    local = {path for path, item in by_destination(ROOT, include_self=True).items() if not item.committed}

    assert local
    assert not local & tracked


def test_check_ignores_local_output_that_was_never_rendered(tmp_path: Path) -> None:
    renderer.render(tmp_path)
    for surface in (".claude", ".agents"):
        for path in sorted((tmp_path / surface).rglob("*"), reverse=True):
            path.unlink() if path.is_file() else path.rmdir()

    assert renderer.check(tmp_path) == 0


def test_check_fails_when_the_committed_runtime_drifts(tmp_path: Path) -> None:
    renderer.render(tmp_path)
    (tmp_path / "runner/manifests/runtime/entrypoint.sh").write_text("drifted\n")

    assert renderer.check(tmp_path) == 1


def test_a_stale_local_render_is_a_doctor_warning(tmp_path: Path, monkeypatch: Any) -> None:
    renderer.render(tmp_path, include_self=True)
    (tmp_path / ".claude/skills/council/SKILL.md").write_text("stale\n")
    monkeypatch.setattr(renderer, "REPOSITORY_ROOT", tmp_path)

    check = renderer.render_doctor_check("local-render", committed=False, ok="ok", problem="warn")

    assert check.status == "warn"
    assert "drifted .claude/skills/council/SKILL.md" in check.detail
    assert "--write" in check.detail


def test_nothing_rendered_locally_is_not_a_doctor_warning(tmp_path: Path, monkeypatch: Any) -> None:
    monkeypatch.setattr(renderer, "REPOSITORY_ROOT", tmp_path)

    check = renderer.render_doctor_check("local-render", committed=False, ok="ok", problem="warn")

    assert check.status == "ok"
    assert "not rendered" in check.detail


def test_output_renders_skills_without_this_checkouts_council_context(tmp_path: Path) -> None:
    result = subprocess.run(
        [sys.executable, str(RENDERER_PATH), "--output", str(tmp_path)],
        capture_output=True,
        text=True,
        check=True,
    )

    assert "rendered" in result.stdout
    assert (tmp_path / ".claude/skills/council/SKILL.md").is_file()
    assert not (tmp_path / ".council").exists()


def manifest_skill_targets() -> list[dict[str, str]]:
    validator_path = ROOT / "scripts/validate_manifest.py"
    validator_spec = importlib.util.spec_from_file_location("validate_manifest_layout", validator_path)
    assert validator_spec is not None and validator_spec.loader is not None
    validator = importlib.util.module_from_spec(validator_spec)
    validator_spec.loader.exec_module(validator)
    return [target for skill in validator.load_manifest()["skills"] for target in skill["targets"]]


def test_manifest_skill_targets_keep_their_consumer_destination_and_name_their_source(tmp_path: Path) -> None:
    rendered = by_destination(tmp_path)

    for target in manifest_skill_targets():
        assert target["path"].startswith((".claude/", ".agents/")), target
        assert rendered[target["path"]].source == ROOT / target["source"], target


def test_rendered_council_context_does_not_promise_to_keep_local_edits() -> None:
    for fragment in sorted((ROOT / "council/context").glob("*.md")):
        text = " ".join(fragment.read_text().split())
        assert "preserve local edits" not in text, fragment.name
        assert "scripts/render_agent_kit.py --write" in text, fragment.name
