"""Contratto del piano: YAML indipendente, mutazioni e processi realmente rossi."""
from __future__ import annotations

import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import yaml

from scripts import ci_locale as ci
from scripts.ci_locale_expr import Unsupported, evaluate, render


class CiLocaleTests(unittest.TestCase):
    def setUp(self):
        (ci.ROOT / "target").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=ci.ROOT / "target")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / ".github/workflows").mkdir(parents=True)

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(yaml.dump(value, sort_keys=False), encoding="utf-8")

    def workflow(self, steps, **kwargs):
        job = {"runs-on": "windows-latest", "steps": steps, **kwargs}
        self.write(".github/workflows/pr.yml", {"on": {"pull_request": None}, "jobs": {"probe": job}})

    def assert_structure(self, root, plan):
        # Lettura indipendente dal derivatore: non confrontare derive con se stesso.
        expected, expected_steps, visited = {}, {}, set()

        def visit_steps(items, origin):
            for index, item in enumerate(items):
                expected_steps[f"{origin}/steps/{index}"] = item
                use = item.get("uses", "")
                if use.startswith("./"):
                    directory = root / use
                    action = next((p for p in (directory / "action.yml", directory / "action.yaml") if p.is_file()), None)
                    if action is not None and str(action) not in visited:
                        visited.add(str(action))
                        data = yaml.load(action.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
                        visit_steps(data.get("runs", {}).get("steps", []), action.relative_to(root).as_posix() + "/runs")

        def visit_workflow(path):
            if str(path) in visited:
                return
            visited.add(str(path))
            raw = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
            for key, job in raw.get("jobs", {}).items():
                relative = path.relative_to(root).as_posix()
                expected[(relative, key)] = job
                visit_steps(job.get("steps", []), relative + "/jobs/" + key)
                if job.get("uses", "").startswith("./"):
                    visit_workflow(root / job["uses"])

        for path in (root / ".github/workflows").glob("*.y*ml"):
            raw = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
            trigger = raw.get("on") or {}
            if trigger == "pull_request" or not isinstance(trigger, str) and "pull_request" in trigger:
                visit_workflow(path)
        actual = {(j["workflow"], j["key"]) for j in plan["jobs"]}
        self.assertEqual(actual, set(expected))
        actual_steps = {s["source"]: s["raw"] for j in plan["jobs"] for s in ci.flatten(j["steps"])}
        self.assertEqual(actual_steps, expected_steps)
        for job in plan["jobs"]:
            key = (job["workflow"], job["key"])
            if key not in expected:
                continue
            self.assertEqual(job["raw"], expected[key])
            self.assertEqual([s["raw"] for s in job["steps"]], expected[key].get("steps", []))
            self.assertEqual([s["source"] for s in job["steps"]],
                             [job["source"] + f"/steps/{i}" for i in range(len(expected[key].get("steps", [])))])

    def run_plan(self, plan):
        with patch.object(ci, "git", side_effect=lambda root, *args: "abc" if args[0] == "rev-parse" else ""), \
                patch.object(ci, "host_os", return_value="Windows"), contextlib.redirect_stdout(io.StringIO()):
            return ci.execute(plan, self.root)

    def test_repository_structural_coverage(self):
        plan = ci.derive()
        self.assert_structure(ci.ROOT, plan)
        runs = [s["run"] for j in plan["jobs"] for s in ci.flatten(j["steps"])]
        self.assertIn("python scripts/check_prove_fonte_unica.py --run", runs)
        self.assertIn("cargo fmt --all -- --check", runs)
        self.assertIn("cargo clippy --workspace --all-targets --all-features --locked -- -D warnings", runs)

    def test_missing_new_job_or_step_fails_comparison(self):
        self.workflow([{"run": "print('old')", "shell": "python"}])
        old = ci.derive(self.root)
        path = self.root / ".github/workflows/pr.yml"
        raw = ci.read_yaml(self.root, ".github/workflows/pr.yml")
        raw["jobs"]["new-job"] = {"runs-on": "windows-latest", "steps": [{"run": "new"}]}
        self.write(path.relative_to(self.root), raw)
        with self.assertRaises(AssertionError):
            self.assert_structure(self.root, old)
        self.assert_structure(self.root, ci.derive(self.root))
        del raw["jobs"]["new-job"]
        raw["jobs"]["probe"]["steps"].append({"run": "new-step"})
        self.write(path.relative_to(self.root), raw)
        with self.assertRaises(AssertionError):
            self.assert_structure(self.root, old)
        self.assert_structure(self.root, ci.derive(self.root))

    def test_matrix_include_exclude_semantics(self):
        matrix = {"os": ["windows-latest", "ubuntu-latest"], "version": ["a", "b"],
                  "exclude": [{"os": "ubuntu-latest", "version": "b"}],
                  "include": [{"os": "windows-latest", "extra": "x"}, {"os": "macos-latest", "version": "c"}]}
        self.workflow([{"run": "echo ${{ matrix.version }}", "env": {"OS": "${{ matrix.os }}"}}],
                      **{"strategy": {"matrix": matrix}, "runs-on": "${{ matrix.os }}"})
        jobs = ci.derive(self.root)["jobs"]
        self.assertEqual([j["matrix"] for j in jobs], [
            {"os": "windows-latest", "version": "a", "extra": "x"},
            {"os": "windows-latest", "version": "b", "extra": "x"},
            {"os": "ubuntu-latest", "version": "a"}, {"os": "macos-latest", "version": "c"}])
        self.assertEqual([j["steps"][0]["run"] for j in jobs], ["echo a", "echo b", "echo a", "echo c"])

    def test_local_reusable_and_nested_composite(self):
        self.write(".github/workflows/pr.yml", {"on": "pull_request", "jobs": {
            "call": {"uses": "./.github/workflows/shared.yml", "with": {"message": "hello"}}}})
        self.write(".github/workflows/shared.yml", {"on": {"workflow_call": {"inputs": {"message": {"default": "default"}}}},
            "jobs": {"child": {"runs-on": "windows-latest", "env": {"OUTER": "outer"}, "steps": [
                {"uses": "./.github/actions/probe", "with": {"text": "${{ inputs.message }}"}, "if": "false"}]}}})
        self.write(".github/actions/probe/action.yml", {"inputs": {"text": {}}, "runs": {"using": "composite", "steps": [
            {"shell": "python", "run": "print('${{ inputs.text }}')", "env": {"INNER": "inner"}}]}})
        plan = ci.derive(self.root)
        self.assert_structure(self.root, plan)
        self.assertEqual(len(plan["jobs"]), 2)
        child = plan["jobs"][1]["steps"][0]["children"][0]
        self.assertEqual(child["run"], "print('hello')")
        self.assertEqual(child["env"], {"OUTER": "outer", "INNER": "inner"})
        self.assertEqual(child["parent_conditions"], ["false"])
        self.assertIn(".github/actions/probe/action.yml", plan["sources"])
        _, receipt = self.run_plan(plan)
        self.assertNotIn("PASS", [s["status"] for s in receipt["steps"]])
        action_path = ".github/actions/probe/action.yml"
        action = ci.read_yaml(self.root, action_path)
        action["runs"]["steps"].append({"run": "print('new')", "shell": "python"})
        self.write(action_path, action)
        with self.assertRaises(AssertionError):
            self.assert_structure(self.root, plan)
        self.assert_structure(self.root, ci.derive(self.root))

    def test_failure_never_becomes_pass_and_always_runs(self):
        self.workflow([{"shell": "python", "run": "import sys; sys.exit(7)", "continue-on-error": True},
                       {"shell": "python", "run": "print('later')", "if": "always()"}])
        path, receipt = self.run_plan(ci.derive(self.root))
        saved = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual([s["status"] for s in saved["steps"]], ["FAIL", "PASS"])
        self.assertEqual(receipt["steps"][0]["exit_code"], 7)
        self.assertEqual(receipt["jobs"][0]["status"], "FAIL")
        self.assertEqual(receipt["steps"][0]["environment"]["CARGO_BUILD_JOBS"], "4")
        self.assertEqual(receipt["steps"][0]["environment"]["CARGO_INCREMENTAL"], "0")

    def test_default_condition_does_not_hide_missing_prerequisite(self):
        self.workflow([{"uses": "unknown/action@pin"}, {"shell": "python", "run": "print('must not pass')"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual([s["status"] for s in receipt["steps"]], ["NON ESEGUITO", "NON ESEGUITO"])

    def test_live_release_and_platform_are_explicit(self):
        self.workflow([{"run": "docker run database"}, {"run": "cargo publish"}, {"run": "gh release create tag"}])
        job = ci.derive(self.root)["jobs"][0]
        with patch.object(ci, "host_os", return_value="Windows"):
            for step in job["steps"]:
                self.assertEqual(ci.classify(job, step, self.root)[0], "NON ESEGUITO")
            self.assertEqual(ci.classify(job, job["steps"][1], self.root, True)[0], "NON ESEGUITO")
        with patch.object(ci, "host_os", return_value="Linux"):
            self.assertEqual(ci.classify(job, job["steps"][0], self.root)[0], "SALTATO")

    def test_mapping_table_and_versions(self):
        self.assertEqual(set(ci.ACTION_MAP.values()), {"checkout", "python", "rust", "cache"})
        self.assertEqual(set(ci.ACTION_INPUTS), set(ci.ACTION_MAP.values()))
        (self.root / ".python-version").write_text("0.1", encoding="utf-8")
        self.workflow([{"uses": "actions/setup-python@pin", "with": {"python-version-file": ".python-version"}}])
        plan = ci.derive(self.root)
        self.assertEqual(plan["jobs"][0]["steps"][0]["toolchain"]["python"], "0.1")
        _, receipt = self.run_plan(plan)
        self.assertEqual(receipt["steps"][0]["status"], "FAIL")
        self.workflow([{"uses": "actions/checkout@pin", "with": {"repository": "remote/repo"}},
                       {"uses": "Swatinem/rust-cache@pin"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual([s["status"] for s in receipt["steps"]], ["NON ESEGUITO", "NON ESEGUITO"])
        self.workflow([{"uses": "actions-rust-lang/setup-rust-toolchain@pin", "with": {
            "toolchain": "1.98.1", "components": "clippy, rustfmt"}}])
        step = ci.derive(self.root)["jobs"][0]["steps"][0]
        self.assertEqual(ci.action_command(step, self.root, "abc")[-3:], ["1.98.1", "clippy", "rustfmt"])

    def test_unmapped_action_inputs_not_accepted_as_equivalent(self):
        self.workflow([{"uses": "actions/setup-python@pin", "with": {"python-version": "3.14", "architecture": "arm64"}}])
        step = ci.derive(self.root)["jobs"][0]["steps"][0]
        self.assertIn("opzioni action non mappate", step["reason"])

    def test_unknown_action_with_false_condition_does_not_block(self):
        self.workflow([{"uses": "unknown/action@pin", "if": "false"}, {"run": "print('next')", "shell": "python"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual([s["status"] for s in receipt["steps"]], ["NON ESEGUITO", "PASS"])

    def test_defaults_env_filters_and_unknown_expressions(self):
        self.write(".github/workflows/pr.yml", {"on": {"pull_request": {"paths": ["crates/**"]}},
            "defaults": {"run": {"shell": "python", "working-directory": "scripts"}}, "env": {"X": "workflow"},
            "jobs": {"probe": {"runs-on": "windows-latest", "env": {"X": "job"}, "steps": [
                {"run": "print('${{ env.X }}')", "env": {"X": "step"}}]}}})
        job = ci.derive(self.root)["jobs"][0]
        self.assertEqual(job["filters"]["pull_request"]["paths"], ["crates/**"])
        self.assertEqual(job["steps"][0]["shell"], "python")
        self.assertEqual(job["steps"][0]["working_directory"], "scripts")
        self.assertEqual(job["steps"][0]["run"], "print('step')")
        with self.assertRaises(Unsupported):
            render("${{ secrets.TOKEN }}", {})
        self.assertEqual(evaluate("github.event_name == 'schedule' && '300' || '60'", {"github.event_name": "pull_request"}), "60")

    def test_dynamic_matrix_still_has_job_and_steps(self):
        self.workflow([{"run": "probe"}], strategy={"matrix": "${{ fromJSON(needs.other.outputs.matrix) }}"})
        plan = ci.derive(self.root)
        self.assertEqual(len(plan["jobs"][0]["steps"]), 1)
        self.assertIn("matrice dinamica", plan["jobs"][0]["reason"])

    def test_skipping_output_is_not_pass(self):
        self.workflow([{"shell": "python", "run": "print('No volume, skipping test.')"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual(receipt["steps"][0]["status"], "NON ESEGUITO")

    def test_working_directory_cannot_escape(self):
        self.workflow([{"shell": "python", "run": "print('outside')", "working-directory": ".."}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual(receipt["steps"][0]["status"], "NON ESEGUITO")

    def test_false_condition_does_not_block_next_step(self):
        self.workflow([{"shell": "python", "if": "false", "run": "raise RuntimeError()"},
                       {"shell": "python", "run": "print('executed')"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual([s["status"] for s in receipt["steps"]], ["NON ESEGUITO", "PASS"])

    def test_outputs_and_environment_are_propagated(self):
        self.workflow([{"id": "producer", "shell": "python", "run":
            "import os\nfrom pathlib import Path\n"
            "Path(os.environ['GITHUB_OUTPUT']).write_text('value=hello\\n')\n"
            "Path(os.environ['GITHUB_ENV']).write_text('CI_LOCAL_VALUE=world\\n')"},
            {"shell": "python", "run": "import os; assert '${{ steps.producer.outputs.value }}' == 'hello'; "
             "assert os.environ['CI_LOCAL_VALUE'] == 'world'"}])
        _, receipt = self.run_plan(ci.derive(self.root))
        self.assertEqual([s["status"] for s in receipt["steps"]], ["PASS", "PASS"])

    def test_empty_matrix_not_silently_lost(self):
        self.workflow([{"run": "echo probe"}], strategy={"matrix": {"os": []}})
        plan = ci.derive(self.root)
        self.assertEqual(len(plan["jobs"]), 1)
        self.assertEqual(plan["jobs"][0]["reason"], "matrice senza combinazioni attive")

    def test_timeout_and_start_error_are_fail(self):
        self.workflow([{"shell": "python", "run": "print('probe')"}])
        plan = ci.derive(self.root)
        import subprocess
        for error in (OSError("cannot start"), subprocess.TimeoutExpired("probe", 1)):
            with self.subTest(error=type(error).__name__), patch.object(ci.subprocess, "run", side_effect=error):
                _, receipt = self.run_plan(plan)
                self.assertEqual(receipt["steps"][0]["status"], "FAIL")

    def test_discovery_registers_script_and_tests(self):
        from scripts import check_prove_fonte_unica as proofs
        self.assertIn(ci.ROOT / "scripts/test_ci_locale.py", proofs.python_tests(ci.ROOT))
        data = proofs.inventory(ci.ROOT)
        self.assertIn("scripts/ci_locale.py", data["scripts"])
        self.assertIn("scripts/ci_locale_expr.py", data["scripts"])
        self.assertIn("scripts/test_ci_locale.py", data["python_tests"])

    def test_cli_exit_failure_and_read_only_plan(self):
        self.workflow([{"shell": "python", "run": "raise SystemExit(9)"}])
        with patch.object(ci, "ROOT", self.root), patch.object(ci, "host_os", return_value="Windows"), \
                patch.object(ci, "git", side_effect=lambda root, *args: "abc" if args[0] == "rev-parse" else ""), \
                contextlib.redirect_stdout(io.StringIO()):
            before = set(self.root.rglob("*"))
            self.assertEqual(ci.main(["--piano"]), 0)
            self.assertEqual(set(self.root.rglob("*")), before)
            self.assertEqual(ci.main(["--esegui", "--profilo", "pr"]), 1)

    def test_pull_request_target_not_confused_with_pull_request(self):
        self.write(".github/workflows/pr.yml", {"on": "pull_request_target", "jobs": {
            "not-pr": {"runs-on": "windows-latest", "steps": [{"run": "echo not-pr"}]}}})
        self.assertEqual(ci.derive(self.root)["jobs"], [])

    def test_empty_plan_and_empty_job_never_green(self):
        with patch.object(ci, "ROOT", self.root), patch.object(ci, "host_os", return_value="Windows"), \
                patch.object(ci, "git", side_effect=lambda root, *args: "abc" if args[0] == "rev-parse" else ""), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ci.main(["--esegui"]), 2)
            self.workflow([])
            self.assertEqual(ci.main(["--esegui"]), 2)


if __name__ == "__main__":
    unittest.main()
