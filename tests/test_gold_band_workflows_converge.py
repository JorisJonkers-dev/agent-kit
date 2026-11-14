"""The personal Gold Band instance's workflow templates are converged onto installer/gold-band/workflows.

Each test drives ``converge.main(["gold-band-workflows", ...])`` against a temporary
workflows directory, registry, keep file and template store, so no test reads or
writes a real ``~/.gold-band``. The running guard is driven through
``AGENT_KIT_GOLD_BAND_RUNNING``. One test runs against the kit's real files.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

REGISTRY = {"gold_band_instances": {"gold-band": {"agents": ["claude-work", "codex"]}}}


def _template(wid: str, agent: str = "claude", goal: str = "do it") -> dict[str, Any]:
    return {
        "workflow": {
            "version": "0.1",
            "id": wid,
            "entry": "one",
            "nodes": [
                {"type": "worker", "id": "one", "executionSlotId": f"slot-{wid}", "provider": None, "goal": goal}
            ],
            "edges": [],
        },
        "modelBindings": {"bindings": [{"executionSlotId": f"slot-{wid}", "agentId": agent, "modelId": "m"}]},
    }


BUILT_IN = {
    "id": "default",
    "name": "Default full workflow",
    "isBuiltIn": True,
    "workflow": {"id": "default", "nodes": []},
    "modelBindings": {"bindings": []},
    "createdAt": "1794686101Z",
    "updatedAt": "1794686101Z",
}


class Machine:
    """A workflows directory, registry, keep file and template store under ``tmp_path``."""

    def __init__(self, tmp_path: Path, sources: dict[str, dict[str, Any]], keep: list[str] | None = None) -> None:
        self.root = tmp_path
        self.workflows = tmp_path / "workflows"
        self.workflows.mkdir()
        for wid, body in sources.items():
            (self.workflows / f"{wid}.json").write_text(json.dumps(body))
        self.keep = tmp_path / "keep.yaml"
        self.keep.write_text(yaml.safe_dump({"keep": keep or []}))
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(REGISTRY))
        self.store = tmp_path / "home" / ".gold-band" / "context" / "workflows.json"

    def seed(self, store: dict[str, Any]) -> None:
        self.store.parent.mkdir(parents=True)
        self.store.write_text(json.dumps(store))

    def read(self) -> dict[str, Any]:
        return json.loads(self.store.read_text())

    def run(self, *, check: bool = False, **paths: Path) -> int:
        argv = [
            "gold-band-workflows",
            "--keep",
            str(paths.get("keep", self.keep)),
            "--store",
            str(paths.get("store", self.store)),
            "--workflows",
            str(paths.get("workflows", self.workflows)),
            "--registry",
            str(paths.get("registry", self.registry)),
        ]
        if check:
            argv.append("--check")
        return converge.main(argv)


@pytest.fixture(autouse=True)
def _not_running(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")


def _store(*templates: dict[str, Any]) -> dict[str, Any]:
    return {
        "version": "0.1",
        "lastUsedTemplateId": "default",
        "lastCreatedWorkflow": None,
        "templates": list(templates),
    }


def test_a_missing_store_is_created_owner_only_with_every_template(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha"), "beta": _template("beta")})
    assert m.run() == 0
    assert capsys.readouterr().out.splitlines() == ["added alpha", "added beta"]
    store = m.read()
    assert [t["id"] for t in store["templates"]] == ["alpha", "beta"]
    first = store["templates"][0]
    assert first["name"] == "alpha" and first["isBuiltIn"] is False
    assert (
        first["workflow"] == _template("alpha")["workflow"]
        and first["modelBindings"] == _template("alpha")["modelBindings"]
    )
    assert re.fullmatch(r"\d+Z", first["createdAt"]) and first["updatedAt"] == first["createdAt"]
    assert store["version"] == "0.1" and store["lastUsedTemplateId"] is None
    assert os.stat(m.store).st_mode & 0o777 == 0o600
    assert os.stat(m.store.parent.parent).st_mode & 0o777 == 0o700
    assert sorted(p.name for p in m.store.parent.iterdir()) == ["workflows.json"]


def test_a_second_run_writes_nothing(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    m.run()
    before = (m.store.read_bytes(), m.store.stat().st_mtime_ns)
    capsys.readouterr()
    assert m.run() == 0
    assert capsys.readouterr().out == "workflows unchanged\n"
    assert (m.store.read_bytes(), m.store.stat().st_mtime_ns) == before
    assert not m.store.with_name("workflows.json.bak").exists()


def test_the_apps_builtins_owner_templates_and_other_keys_are_left_as_they_are(tmp_path: Path) -> None:
    mine = {
        "id": "mine",
        "name": "Mine",
        "isBuiltIn": False,
        "workflow": {"id": "w-1", "nodes": []},
        "modelBindings": {},
        "createdAt": "1Z",
        "updatedAt": "2Z",
    }
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    seeded = {**_store(BUILT_IN, mine), "lastCreatedWorkflow": {"id": "x"}, "extra": {"k": 1}}
    m.seed(seeded)
    assert m.run() == 0
    store = m.read()
    assert store["templates"][:2] == [BUILT_IN, mine] and store["templates"][2]["id"] == "alpha"
    assert {k: v for k, v in store.items() if k != "templates"} == {k: v for k, v in seeded.items() if k != "templates"}
    backup = json.loads(m.store.with_name("workflows.json.bak").read_text())
    assert backup == seeded, "one backup holds what the store held before"


def test_a_drifted_template_is_replaced_in_place_keeping_its_name_and_created_at(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha", goal="new"), "beta": _template("beta")})
    held = {
        "id": "alpha",
        "name": "My alpha",
        "isBuiltIn": False,
        **_template("alpha", goal="old"),
        "createdAt": "10Z",
        "updatedAt": "20Z",
    }
    m.seed(_store(held, {"id": "other", "name": "Other", "workflow": {"id": "o"}, "modelBindings": {}}))
    assert m.run() == 0
    assert capsys.readouterr().out.splitlines() == ["changed alpha", "added beta"]
    store = m.read()
    assert [t["id"] for t in store["templates"]] == ["alpha", "other", "beta"]
    alpha = store["templates"][0]
    assert alpha["name"] == "My alpha" and alpha["createdAt"] == "10Z" and alpha["updatedAt"] != "20Z"
    assert alpha["workflow"]["nodes"][0]["goal"] == "new"


def test_a_binding_change_alone_is_drift(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha", agent="vibe")})
    m.seed(
        _store(
            {
                "id": "alpha",
                "name": "alpha",
                "isBuiltIn": False,
                **_template("alpha", agent="claude"),
                "createdAt": "1Z",
                "updatedAt": "1Z",
            }
        )
    )
    m.run()
    assert capsys.readouterr().out == "changed alpha\n"
    assert m.read()["templates"][0]["modelBindings"]["bindings"][0]["agentId"] == "vibe"


def test_a_field_the_app_drops_when_null_is_not_drift(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    saved = _template("alpha")
    del saved["workflow"]["nodes"][0]["provider"]
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    m.seed(_store({"id": "alpha", "name": "alpha", "isBuiltIn": False, **saved, "createdAt": "1Z", "updatedAt": "1Z"}))
    assert m.run() == 0
    assert capsys.readouterr().out == "workflows unchanged\n"


def test_a_workflow_the_keep_file_names_is_never_overwritten_or_added(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(
        tmp_path,
        {"alpha": _template("alpha", goal="new"), "beta": _template("beta"), "gamma": _template("gamma")},
        keep=["alpha", "beta.json"],
    )
    held = {
        "id": "alpha",
        "name": "alpha",
        "isBuiltIn": False,
        **_template("alpha", goal="owner's"),
        "createdAt": "1Z",
        "updatedAt": "1Z",
    }
    m.seed(_store(held))
    assert m.run() == 0
    assert capsys.readouterr().out.splitlines() == ["kept alpha (keep file)", "kept beta (keep file)", "added gamma"]
    store = m.read()
    assert store["templates"][0] == held
    assert [t["id"] for t in store["templates"]] == ["alpha", "gamma"], "a kept name that is absent is not added"


def test_a_workflow_bound_to_a_work_instance_agent_is_not_installed(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(
        tmp_path,
        {
            "alpha": _template("alpha"),
            "alpha-work": _template("alpha-work", agent="claude-work"),
            "beta-work": _template("beta-work", agent="codex"),
        },
    )
    assert m.run() == 0
    assert capsys.readouterr().out.splitlines() == [
        "added alpha",
        "skipped alpha-work (bound to the work instance's claude-work)",
        "skipped beta-work (bound to the work instance's codex)",
    ]
    assert [t["id"] for t in m.read()["templates"]] == ["alpha"]


def test_a_worker_provider_on_a_work_agent_also_counts(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    body = _template("alpha")
    body["workflow"]["nodes"][0]["provider"] = "codex"
    m = Machine(tmp_path, {"alpha": body})
    assert m.run() == 0
    assert "skipped alpha" in capsys.readouterr().out and not m.store.exists()


def test_check_reports_the_drift_and_writes_nothing(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha", goal="new"), "beta": _template("beta")})
    held = {
        "id": "alpha",
        "name": "alpha",
        "isBuiltIn": False,
        **_template("alpha", goal="old"),
        "createdAt": "1Z",
        "updatedAt": "1Z",
    }
    m.seed(_store(held))
    before = m.store.read_bytes()
    assert m.run(check=True) == 0
    assert capsys.readouterr().out.splitlines() == ["would change alpha", "would add beta"]
    assert m.store.read_bytes() == before
    assert not m.store.with_name("workflows.json.bak").exists()


def test_check_wording_and_no_store_created(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    assert m.run(check=True) == 0
    assert capsys.readouterr().out == "would add alpha\n"
    assert not m.store.exists() and not m.store.parent.exists()


def test_a_running_app_stops_the_run_before_anything_is_written(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    assert m.run() == converge.GOLD_BAND_RUNNING_EXIT
    assert "gold-band-workflows: Gold Band instance" in capsys.readouterr().out
    assert not m.store.exists()
    assert m.run(check=True) == 0, "--check never needs the app to be stopped"


def test_an_app_that_starts_before_the_write_stops_the_run(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    answers = iter([None, "gold-band-personal (process 5)"])
    monkeypatch.setattr(converge, "_gold_band_running", lambda *_a, **_k: next(answers))
    assert m.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "started during setup" in out and "not yet written: workflows.json" in out
    assert not m.store.exists()


def test_a_store_that_is_not_a_store_is_refused_and_left_alone(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    for text in ("{not json", json.dumps({"templates": "no"}), json.dumps([1]), json.dumps({"templates": [1]})):
        m.store.parent.mkdir(parents=True, exist_ok=True)
        m.store.write_text(text)
        assert m.run() == 3
        assert "gold-band-workflows: cannot converge:" in capsys.readouterr().out
        assert m.store.read_text() == text
    m.store.write_bytes(b"\xff\xfe")
    assert m.run() == 3


def test_an_existing_builtin_under_a_shipped_id_is_refused(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    m.seed(_store({**BUILT_IN, "id": "alpha"}))
    before = m.store.read_bytes()
    assert m.run() == 1
    assert "the existing template is a built-in; nothing was written" in capsys.readouterr().out
    assert m.store.read_bytes() == before


def test_the_template_inside_a_differently_named_slot_is_matched_by_its_workflow_id(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha", goal="new")})
    m.seed(
        _store(
            {
                "id": "copy-of-alpha",
                "name": "Copy",
                "isBuiltIn": False,
                **_template("alpha", goal="old"),
                "createdAt": "1Z",
                "updatedAt": "1Z",
            }
        )
    )
    m.run()
    assert capsys.readouterr().out == "changed alpha\n"
    assert [t["id"] for t in m.read()["templates"]] == ["copy-of-alpha"]


@pytest.mark.parametrize(
    "files, message",
    [
        ({"alpha.json": "{bad"}, "cannot be read"),
        ({"alpha.json": "[]"}, "must hold a `workflow` object"),
        ({"alpha.json": json.dumps({"workflow": {"id": "alpha"}})}, "must hold a `workflow` object"),
        ({"alpha.json": json.dumps(_template("other"))}, "must be the file's own name"),
        ({"Alpha.json": json.dumps(_template("Alpha"))}, "must be the file's own name"),
        ({"default.json": json.dumps(_template("default"))}, "built-in template of the app"),
    ],
)
def test_a_bad_source_file_refuses_the_whole_run(
    tmp_path: Path, capsys: pytest.CaptureFixture[str], files: dict[str, str], message: str
) -> None:
    m = Machine(tmp_path, {"good": _template("good")})
    for name, text in files.items():
        (m.workflows / name).write_text(text)
    assert m.run() == 3
    assert message in capsys.readouterr().out
    assert not m.store.exists()


def test_a_missing_workflows_directory_or_registry_is_refused(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    assert m.run(workflows=tmp_path / "nope") == 3
    assert m.run(registry=tmp_path / "nope.yaml") == 3
    m.registry.write_text(yaml.safe_dump({"gold_band_instances": {}}))
    assert m.run() == 3
    assert "gold_band_instances.gold-band.agents" in capsys.readouterr().out


def test_a_malformed_keep_file_stops_the_run_before_the_write(tmp_path: Path) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    m.keep.write_text("keep: 3\n")
    with pytest.raises(SystemExit):
        m.run()
    assert not m.store.exists()


def test_a_store_that_cannot_be_replaced_leaves_the_old_one_and_no_staged_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    m.seed(_store())
    before = m.store.read_bytes()

    def boom(*_a: object, **_k: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(os, "replace", boom)
    assert m.run() == 3
    assert "the old store is in place" in capsys.readouterr().out
    monkeypatch.undo()
    assert m.store.read_bytes() == before
    assert sorted(p.name for p in m.store.parent.iterdir()) == ["workflows.json"]


def test_the_default_store_is_the_personal_instances_under_gold_band_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("GOLD_BAND_HOME", str(tmp_path / "gb"))
    assert converge._gold_band_workflow_store_path() == tmp_path / "gb" / ".gold-band" / "context" / "workflows.json"
    m = Machine(tmp_path, {"alpha": _template("alpha")})
    argv = [
        "gold-band-workflows",
        "--keep",
        str(m.keep),
        "--workflows",
        str(m.workflows),
        "--registry",
        str(m.registry),
    ]
    assert converge.main(argv) == 0
    assert (tmp_path / "gb" / ".gold-band" / "context" / "workflows.json").is_file()


def test_the_kits_own_workflows_install_into_an_empty_store_and_settle(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    shipped = sorted(p.stem for p in (KIT_ROOT / "installer" / "gold-band" / "workflows").glob("*.json"))
    work = {"workflow-b-work", "workflow-c-work"}
    assert work <= set(shipped)
    store = tmp_path / "home" / ".gold-band" / "context" / "workflows.json"
    argv = ["gold-band-workflows", "--keep", str(tmp_path / "none.yaml"), "--store", str(store)]
    assert converge.main(argv) == 0
    out = capsys.readouterr().out.splitlines()
    assert [line.split()[1] for line in out if line.startswith("skipped")] == sorted(work)
    installed = [t["id"] for t in json.loads(store.read_text())["templates"]]
    assert installed == [name for name in shipped if name not in work]
    assert converge.main([*argv, "--check"]) == 0
    assert capsys.readouterr().out.splitlines()[-1] == "workflows unchanged"


def test_setup_runs_the_convergence_and_the_bundle_ships_the_workflows() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "gold_band_workflows() {" in setup and "\ngold_band_workflows\n" in setup
    assert 'converge.py" gold-band-workflows' in setup
    assert '--store "${band_home}/.gold-band/context/workflows.json"' in setup
    assert '--workflows "${KIT_ROOT}/installer/gold-band/workflows"' in setup
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    assert "installer/gold-band/workflows" in publish.split("KIT_PATHS=(")[1].split(")")[0].split()
