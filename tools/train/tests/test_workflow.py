"""明示した設定と完了記録だけで学習工程を進める契約を検証する。"""

from __future__ import annotations

import hashlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stdout
from pathlib import Path
from unittest.mock import patch

import numpy as np

import minase_train.workflow as workflow
from helpers import python_probe, write_mnsd, write_provenance, write_rescore
from minase_train.checksum import sha256_file
from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import initial_piece_values, read_mnpt, write_mnpt


CONFIG = '''[run]
directory = "data/run"
base_commit = "0000000000000000000000000000000000000000"
data = ["data/old.bin"]
[generate]
seeds = [100, 110]
games = 10
nodes = 100000
concurrency = 1
max_ply = 600
hash_mb = 16
random_moves = 0
[train]
model = "single"
k = 1072.6529541015625
learning_rate = 3
epochs = 10
batch = 16384
seed = 1
removal_penalty = 0
device = "cpu"
validation_sample = 10000
[diagnose]
sample_size = 10000
seed = 1
'''


class WorkflowTest(unittest.TestCase):

    def setUp(self) -> None:
        self.enterContext(patch.dict("os.environ", {"PYTHONPATH": str(workflow.SOURCES.parent)}))
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "data").mkdir()
        self.config_path = self.root / "config.toml"
        self.config_path.write_text(CONFIG)
        self.config = workflow.load_config(self.config_path, self.root)
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=[1, 10])

    def test_explicit_config_resolves_paths_and_accepts_touching_seed_ranges(self) -> None:
        self.assertEqual(self.config["run"]["directory"], str(self.root / "data/run"))
        self.assertEqual(self.config["run"]["data"], [str(self.root / "data/old.bin")])
        self.assertEqual(self.config["generate"]["seeds"], [100, 110])
        self.assertEqual(self.config["train"]["device"], "cpu")
        self.assertEqual(self.config["train"]["model"], "single")
        # 生成シードの空配列は、既存データだけで学習する明示的な指定として受理する。
        self.config_path.write_text(CONFIG.replace("seeds = [100, 110]", "seeds = []"))
        self.assertEqual(workflow.load_config(self.config_path, self.root)["generate"]["seeds"], [])

    def test_missing_and_unknown_fields_are_rejected(self) -> None:
        # 出力Kと追加損失の係数は、省略せず明示する契約。
        for content in (CONFIG.replace("k = 1072.6529541015625\n", ""),
                        CONFIG.replace("removal_penalty = 0\n", ""),
                        CONFIG.replace("epochs = 10\n", ""),
                        CONFIG.replace("epochs = 10", "epochs = 10\nepoch = 10"),
                        CONFIG + "\n[extra]\nvalue = 1\n"):
            with self.subTest(content=content):
                self.config_path.write_text(content)
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_positive_removal_penalty_requires_mirrored_model(self) -> None:
        for model in ("single", "tapered", "mirrored"):
            with self.subTest(model=model):
                self.config_path.write_text(CONFIG.replace(
                    'model = "single"', f'model = "{model}"').replace(
                    "removal_penalty = 0", "removal_penalty = 2.5"))
                if model == "mirrored":
                    config = workflow.load_config(self.config_path, self.root)
                    self.assertEqual(config["train"]["removal_penalty"], 2.5)
                else:
                    with self.assertRaisesRegex(ValueError, "removal_penalty.*mirrored"):
                        workflow.load_config(self.config_path, self.root)

    def test_removal_penalty_rejects_negative_values(self) -> None:
        self.config_path.write_text(CONFIG.replace(
            'model = "single"', 'model = "mirrored"').replace(
            "removal_penalty = 0", "removal_penalty = -0.01"))
        with self.assertRaisesRegex(ValueError, "removal_penalty"):
            workflow.load_config(self.config_path, self.root)

    def test_example_explicitly_disables_unselected_removal_penalty(self) -> None:
        """未選定の係数を設定例で仮定せず、必須項目をすべて明示する。"""
        example = (Path(__file__).resolve().parents[1] / "pst.example.toml").read_text()
        self.config_path.write_text(example.replace("REPLACE_WITH_FULL_COMMIT_HASH", "0" * 40))
        config = workflow.load_config(self.config_path, self.root)
        self.assertEqual(config["train"]["model"], "mirrored")
        self.assertEqual(config["train"]["removal_penalty"], 0)

    def test_invalid_numbers_seed_ranges_and_paths_are_rejected(self) -> None:
        changes = [
            ("games = 10", "games = 0"),
            ("epochs = 10", "epochs = true"),
            ("batch = 16384", "batch = -1"),
            ("learning_rate = 3", "learning_rate = nan"),
            ("learning_rate = 3", "learning_rate = inf"),
            ("learning_rate = 3", "learning_rate = 0"),
            ("k = 1072.6529541015625", "k = 0"),
            ("k = 1072.6529541015625", "k = -1"),
            ("k = 1072.6529541015625", "k = nan"),
            ("k = 1072.6529541015625", "k = inf"),
            ("k = 1072.6529541015625", "k = true"),
            ('model = "single"', 'model = "dual"'),
            ("sample_size = 10000", "sample_size = 0"),
            ("seeds = [100, 110]", "seeds = [100, 100]"),
            ("seeds = [100, 110]", "seeds = [100, 109]"),
            ("seeds = [100, 110]", "seeds = [18446744073709551615]"),
            ('directory = "data/run"', 'directory = "../outside"'),
            ('data = ["data/old.bin"]', 'data = ["data/old.bin", "data/../data/old.bin"]'),
        ]
        for before, after in changes:
            with self.subTest(after=after):
                self.config_path.write_text(CONFIG.replace(before, after))
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_new_seed_ranges_must_not_overlap_recorded_history(self) -> None:
        self.config["generate"]["seeds"] = [10]
        files = workflow.check_existing_data(self.config)
        self.assertEqual(len(files), 1)
        self.assertEqual(files[0]["sha256"], hashlib.sha256(
            (self.root / "data/old.bin").read_bytes()).hexdigest())
        self.config["generate"]["seeds"] = [9]
        with self.assertRaises(ValueError):
            workflow.check_existing_data(self.config)

    def test_prepare_pins_generator_to_base_and_probe_to_training_tools_commit(self) -> None:
        """生成基準が古くても、診断器はprepare時のHEADから別にビルドする。"""
        for case, lookahead in enumerate((None, {"gamma": .9, "plies": 40})):
            run = self.root / f"data/run-{case}"
            self.config["run"]["directory"] = str(run)
            base = "0" * 40
            tools_commit = "1" * 40
            checkouts = {}
            builds = {}
            source = self.root / "data/old.bin"
            sidecar = self.root / "prepare-rescore.bin"
            if lookahead is None:
                write_rescore(sidecar, source, [(1, 0, 20, 1, 100)] * workflow.read_header(source).record_count)
            self.config["train"]["rescore"][0] = str(sidecar) if lookahead is None else "-"
            if lookahead is not None:
                self.config["train"]["lookahead"] = lookahead
                self.config["train"]["lambda_override"] = 1.0

            def fake_git(repository, *arguments):
                if arguments == ("rev-parse", base + "^{commit}"):
                    return base
                if arguments == ("rev-parse", "HEAD"):
                    return tools_commit
                raise AssertionError(arguments)

            def fake_command(run, label, command, cwd):
                if command[:3] == ["git", "worktree", "add"]:
                    destination = Path(command[-2])
                    checkouts[destination] = command[-1]
                    (destination / "crates/minase/nets").mkdir(parents=True)
                    weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
                    write_mnpt(destination / "crates/minase/nets/pst.bin", weights, weights,
                               initial_piece_values(), 1000)
                elif command[:2] == ["cargo", "build"]:
                    names = [command[index + 1] for index, value in enumerate(command) if value == "--bin"]
                    builds[cwd] = command
                    target = Path(command[command.index("--target-dir") + 1]) / "release"
                    target.mkdir(parents=True)
                    for name in names:
                        (target / name).write_bytes(f"{checkouts[cwd]}:{name}".encode())
                else:
                    raise AssertionError(command)

            with patch.object(workflow, "ROOT", self.root), \
                    patch.object(workflow, "load_config", return_value=self.config), \
                    patch.object(workflow, "git", side_effect=fake_git), \
                    patch.object(workflow, "run_command", side_effect=fake_command):
                workflow.prepare(self.config_path)
            state = json.loads((run / "prepared.json").read_text())
            self.assertEqual(state["rescores"], [{"path": str(sidecar), "sha256": sha256_file(sidecar).hex()}] if lookahead is None else [])
            provenance = Path(str(source) + ".provenance.json")
            self.assertEqual(state["existing_data"][0]["provenance"]["sha256"], sha256_file(provenance).hex())
            self.assertEqual(checkouts, {run / "generator": base, run / "probe": tools_commit})
            self.assertEqual(builds, {
                run / name: ["cargo", "build", "--release", "--locked", "--target-dir",
                             str(run / name / "target"), "--bin", "minase"]
                for name in ("generator", "probe")
            })
            self.assertEqual(state["config"]["run"]["base_commit"], base)
            self.assertEqual(state["config"]["train"]["removal_penalty"], 0)
            self.assertEqual(state["probe_commit"], tools_commit)
            self.assertEqual(state["generator_sha256"], sha256_file(run / "generator/target/release/minase").hex())
            self.assertEqual(state["probe_sha256"], sha256_file(run / "probe/target/release/minase").hex())
            self.assertEqual(state["lookahead"], lookahead)
            if lookahead is not None:
                self.assertEqual(state["lambda_override"], 1.0)
                with patch.object(workflow, "ROOT", self.root):
                    workflow.load_prepared(run)
                    state["config"]["train"]["lookahead"]["gamma"] = .7
                    (run / "prepared.json").write_text(json.dumps(state))
                    with self.assertRaisesRegex(ValueError, "lookahead"):
                        workflow.load_prepared(run)

    def prepared(self) -> tuple[Path, ExitStack]:
        """外部git操作だけを置換し、完了記録とファイル検証は実際に通す。"""
        run = self.root / "data/run"
        run.mkdir()
        binary = run / "generator/target/release/minase"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture executable")
        probe = run / "probe/target/release/minase"
        probe.parent.mkdir(parents=True)
        probe.write_bytes(b"fixture probe")
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(run / "pst-base.bin", zeros, zeros, initial_piece_values(), 1000)
        state = {
            "config": self.config,
            "lookahead": self.config["train"].get("lookahead"),
            "lambda_override": self.config["train"].get("lambda_override"),
            "existing_data": workflow.check_existing_data(self.config),
            "rescores": [{"path": p, "sha256": sha256_file(Path(p)).hex()} for p in self.config["train"]["rescore"] if p != "-"],
            "sources": workflow.source_hashes(),
            "base_sha256": sha256_file(run / "pst-base.bin").hex(),
            "base_piece_values_sha256": hashlib.sha256(
                (run / "pst-base.bin").read_bytes()[-workflow.PIECE_VALUE_BYTES:]).hexdigest(),
            "generator_sha256": sha256_file(binary).hex(),
            "probe_commit": "1" * 40,
            "probe_sha256": sha256_file(probe).hex(),
            "repository": str(self.root),
        }
        (run / "prepared.json").write_text(json.dumps(state))
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(workflow, "ROOT", self.root))
        stack.enter_context(patch.object(workflow, "git", side_effect=lambda repo, *args:
                                        ("1" if repo == run / "probe" else "0") * 40
                                        if args == ("rev-parse", "HEAD") else ""))
        return run, stack

    def test_lookahead_config_domains_and_rescore_rejection(self):
        for value in ('{ gamma = 0.9, plies = 40 }', '{ gamma = 0.7, plies = 1 }'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlookahead = ' + value))
            self.assertIn('lookahead', workflow.load_config(self.config_path, self.root)['train'])
        for value in ('{ gamma = 0.9 }',
                      '{ gamma = 0.9, plies = 40, extra = 1 }', 'false'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlookahead = ' + value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                workflow.load_config(self.config_path, self.root)
        self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlookahead = { gamma = 0.9, plies = 40 }\nrescore = ["a", "-", "-"]'))
        with self.assertRaisesRegex(ValueError, 'lookahead.*rescore'):
            workflow.load_config(self.config_path, self.root)

    def test_lookahead_cpu_training_diagnosis_and_receipt_checks(self):
        source = self.root / 'data/old.bin'
        write_mnsd(source, seed=0, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(), scores=[100, 200, -400] * 80)
        self.config['generate']['seeds'] = []
        self.config['train'].update(rescore=['-'], epochs=1, batch=64, validation_sample=16,
                                    lookahead={'gamma': .9, 'plies': 3})
        run, _ = self.prepared()
        with redirect_stdout(io.StringIO()):
            workflow.train(run)
        inputs = json.loads((run / 'training/inputs.json').read_text())
        trained = json.loads((run / 'training/pst.training.json').read_text())
        self.assertEqual(inputs['lookahead'], {'gamma': .9, 'plies': 3})
        self.assertEqual(trained['lookahead'], inputs['lookahead'])
        self.assertEqual(inputs['teacher_classes'], trained['teacher_classes'])
        self.assertEqual(trained['teacher_classes'][0]['lookahead_gamma'], .9)
        commands = [json.loads(line) for line in (run / 'commands.jsonl').read_text().splitlines()]
        argv = next(row['argv'] for row in commands if 'argv' in row)
        self.assertEqual(argv[argv.index('--lookahead-gamma') + 1], '0.9')
        self.assertEqual(argv[argv.index('--lookahead-plies') + 1], '3')
        with patch.object(workflow, 'diagnose_probe', return_value=python_probe):
            workflow.diagnose(run)
        report = json.loads((run / 'diagnostics/report.json').read_text())
        self.assertEqual(report['lookahead'], inputs['lookahead'])
        from minase_train.data.mnsd import Dataset
        from minase_train.diagnostics.comparison import Weights
        dataset = Dataset([source])
        model = Weights(run / 'training/pst.bin')
        expected_scores = np.tile([-560 / 1.9, 400, -400], 80)
        for band in report['bands']:
            indices = np.load(run / 'diagnostics' / band['indices_file'])
            if not len(indices):
                continue
            predicted = model.evaluate(dataset.gather(indices))
            self.assertAlmostEqual(band['candidate']['mae_raw_cp'],
                                   float(np.mean(np.abs(predicted - expected_scores[indices]))))
        for name, record in (('inputs.json', inputs), ('pst.training.json', trained)):
            changed = dict(record, lookahead={'gamma': .7, 'plies': 3})
            path = run / 'training' / name
            path.write_text(json.dumps(changed))
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'lookahead'):
                workflow.load_prepared(run)
            path.write_text(json.dumps(record))

    def test_lambda_override_config_validation(self):
        self.assertNotIn('lambda_override', self.config['train'])
        for value in ('0', '0.5', '1.0'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlambda_override = ' + value))
            config = workflow.load_config(self.config_path, self.root)
            self.assertEqual(config['train']['lambda_override'], float(value))
        self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlambda_override = -0.01'))
        with self.assertRaises(ValueError):
            workflow.load_config(self.config_path, self.root)

    def test_lambda_override_prepare_mismatch_is_rejected(self):
        self.config['train']['lambda_override'] = 0
        run, _ = self.prepared()
        original = (run / 'prepared.json').read_text()
        changes = [lambda state: state.update(lambda_override=None),
                   lambda state: state['config']['train'].pop('lambda_override'),
                   lambda state: state['config']['train'].update(lambda_override=1)]
        for change in changes:
            state = json.loads(original)
            change(state)
            (run / 'prepared.json').write_text(json.dumps(state))
            with self.assertRaisesRegex(ValueError, 'lambda_override'):
                workflow.load_prepared(run)
        (run / 'prepared.json').write_text(original)
        self.assertEqual(workflow.load_prepared(run)['lambda_override'], 0)

    def test_lambda_override_training_diagnosis_and_record_mismatches(self):
        from minase_train.data.mnsd import Dataset
        from helpers import write_provenance
        from minase_train.pst.teacher import estimate_generation_ks
        source = self.root / 'data/old.bin'
        write_mnsd(source, seed=0, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(), scores=[100, 200, -400] * 80)
        human = self.root / 'data/human.bin'
        write_mnsd(human, seed=1, checksum=b'b' * 32, games=list(range(1, 81)))
        write_provenance(human, result_origin='human', **{'lambda': 0,
                         'games': [{'game': i, 'id': str(i)} for i in range(1, 81)]})
        self.config['run']['data'].append(str(human))
        self.config['generate']['seeds'] = []
        self.config['train'].update(rescore=['-', '-'], epochs=1, batch=64, validation_sample=16,
                                    lookahead={'gamma': .9, 'plies': 3}, lambda_override=1)
        run, _ = self.prepared()
        with redirect_stdout(io.StringIO()):
            workflow.train(run)
        with patch.object(workflow, 'diagnose_probe', return_value=python_probe):
            workflow.diagnose(run)
        paths = [run / 'training/inputs.json', run / 'training/pst.training.json',
                 run / 'diagnostics/report.json']
        reference = Dataset([source, human], lookahead={'gamma': .9, 'plies': 3})
        expected_ks, _ = estimate_generation_ks(reference, indices=reference.training_indices)
        for path in paths:
            record = json.loads(path.read_text())
            self.assertEqual(record['lambda_override'], 1)
            self.assertEqual([c['lambda_override'] for c in record['teacher_classes']], [1, None])
            self.assertEqual([c['lambda'] for c in record['teacher_classes']], [1, 0])
            self.assertEqual(record['teacher_classes'][0]['lookahead_gamma'], .9)
            self.assertEqual(record['teacher_ks'], [expected_ks[0], None])
            original = path.read_text()
            changes = [lambda item: item.update(lambda_override=0),
                       lambda item: item['teacher_classes'][0].update(lambda_override=None),
                       lambda item: item['teacher_classes'][0].update(**{'lambda': .75})]
            if path.name == 'inputs.json':
                changes.append(lambda item: item['options'].update(lambda_override=0))
            for change in changes:
                changed = json.loads(original)
                change(changed)
                path.write_text(json.dumps(changed))
                with self.subTest(path=path), self.assertRaises(ValueError):
                    workflow.load_prepared(run)
            path.write_text(original)
        workflow.load_prepared(run)
        report = json.loads(paths[-1].read_text())
        self.assertEqual(report['outcome_metrics']['candidate']['records'],
                         len(reference.validation_indices))
        commands = [json.loads(line) for line in (run / 'commands.jsonl').read_text().splitlines()]
        argv = next(row['argv'] for row in commands if 'argv' in row)
        self.assertEqual(argv[argv.index('--lambda-override') + 1], '1')

    def test_human_games_do_not_reserve_generator_seed_ranges(self):
        from helpers import write_provenance
        write_provenance(self.root / "data/old.bin", result_origin="human", **{
            "lambda": 0, "games": [{"game": 1, "id": "a"}, {"game": 10, "id": "b"}]})
        self.config["generate"]["seeds"] = [0]
        self.assertEqual(len(workflow.check_existing_data(self.config)), 1)

    def test_provenance_change_blocks_generate_train_and_diagnose(self):
        run, stack = self.prepared()
        path = Path(str(self.root / "data/old.bin") + ".provenance.json")
        value = json.loads(path.read_text())
        value["lambda"] = 0.5
        path.write_text(json.dumps(value))
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        for command in (lambda: workflow.generate(run, 100), lambda: workflow.train(run), lambda: workflow.diagnose(run)):
            with self.assertRaisesRegex(ValueError, "checksum changed"):
                command()
        execute.assert_not_called()

    def test_rescore_change_blocks_all_later_stages(self):
        from helpers import write_rescore
        source = self.root / "data/old.bin"
        sidecar = self.root / "replacement.bin"
        count = workflow.read_header(source).record_count
        write_rescore(sidecar, source, [(1, 0, 10, 1, 20)] * count)
        self.config["train"]["rescore"][0] = str(sidecar)
        run, stack = self.prepared()
        raw = bytearray(sidecar.read_bytes())
        raw[242] ^= 1
        sidecar.write_bytes(raw)
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        for command in (lambda: workflow.generate(run, 100), lambda: workflow.train(run), lambda: workflow.diagnose(run)):
            with self.assertRaisesRegex(ValueError, "checksum changed"):
                command()
        execute.assert_not_called()

    def test_generation_and_inspection_use_minase_subcommands(self) -> None:
        """設計書の新しい呼び出しと生成条件を引数列全体で検査する。"""
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        checksum = (run / "pst-base.bin").read_bytes()[48:80]

        def execute(run, label, command, cwd):
            if label == "generate-100":
                write_mnsd(output, seed=100, checksum=checksum, games=[0])

        command = stack.enter_context(patch.object(workflow, "run_command", side_effect=execute))
        workflow.generate(run, 100)
        binary = str(run / "generator/target/release/minase")
        self.assertEqual(command.call_args_list, [
            unittest.mock.call(run, "generate-100", [
                binary, "data", "selfplay", "generate", "--output", str(output), "--seed", "100",
                "--games", "10", "--nodes", "100000", "--random-moves", "0",
                "--concurrency", "1", "--max-ply", "600", "--hash-mb", "16",
            ], run / "generator"),
            unittest.mock.call(run, "inspect-100", [
                binary, "data", "selfplay", "inspect", str(output),
            ], run / "generator"),
        ])
        receipt = json.loads((run / "generated-100.json").read_text())
        self.assertEqual(receipt["sha256"], sha256_file(output).hex())
        self.assertEqual(receipt["records"], 1)

    def test_generation_rejects_modified_minase_binary(self) -> None:
        run, stack = self.prepared()
        (run / "generator/target/release/minase").write_bytes(b"modified generator")
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        with self.assertRaisesRegex(ValueError, "checksum changed"):
            workflow.generate(run, 100)
        execute.assert_not_called()

    def test_existing_output_without_completion_is_not_reused(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        output.write_bytes(b"interrupted data")
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        with self.assertRaises(ValueError):
            workflow.generate(run, 100)
        execute.assert_not_called()
        self.assertEqual(output.read_bytes(), b"interrupted data")
        self.assertFalse((run / "generated-100.json").exists())

    def test_modified_completed_output_is_rejected(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        write_mnsd(output, seed=100,
                   checksum=(run / "pst-base.bin").read_bytes()[48:80], games=[0])
        (run / "generated-100.json").write_text(json.dumps(workflow.input_receipt(output)))
        modified = bytearray(output.read_bytes())
        modified[136 + 147] ^= 1  # 探索値だけを変え、MNSD構造は有効に保つ。
        output.write_bytes(modified)
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        with self.assertRaises(ValueError):
            workflow.generate(run, 100)
        execute.assert_not_called()

    def test_failed_generation_does_not_create_completion_record(self) -> None:
        run, stack = self.prepared()

        def fail(run, label, command, cwd):
            (run / "generated-100.bin").write_bytes(b"partial output")
            raise subprocess.CalledProcessError(1, command)

        stack.enter_context(patch.object(workflow, "run_command", side_effect=fail))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.generate(run, 100)
        self.assertFalse((run / "generated-100.json").exists())

    def test_completed_valid_generation_is_reused_without_running_engine(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        checksum = (run / "pst-base.bin").read_bytes()[48:80]
        write_mnsd(output, seed=100, checksum=checksum, games=[0])
        (run / "generated-100.json").write_text(json.dumps(workflow.input_receipt(output)))
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        workflow.generate(run, 100)
        execute.assert_not_called()

    def completed_training(self, run: Path) -> Path:
        training = run / "training"
        training.mkdir()
        candidate = training / "pst.bin"
        candidate.write_bytes((run / "pst-base.bin").read_bytes())
        float_path = training / "pst-float.npz"
        float_path.write_bytes(b"fixture")
        (training / "complete.json").write_text(json.dumps({
            "sha256": sha256_file(candidate).hex(), "float_sha256": sha256_file(float_path).hex()}))
        return candidate

    def test_diagnosis_rejects_modified_completed_weights(self) -> None:
        run, _ = self.prepared()
        candidate = self.completed_training(run)
        candidate.write_bytes(candidate.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            workflow.diagnose(run)
        self.assertFalse((run / "diagnostics").exists())

    def test_diagnosis_rejects_modified_probe_binary(self) -> None:
        run, _ = self.prepared()
        self.completed_training(run)
        (run / "probe/target/release/minase").write_bytes(b"modified probe")
        with patch.object(workflow, "diagnose_probe") as probe, self.assertRaises(ValueError):
            workflow.diagnose(run)
        probe.assert_not_called()
        self.assertFalse((run / "diagnostics").exists())

    def test_diagnosis_rejects_modified_probe_worktree(self) -> None:
        run, _ = self.prepared()
        self.completed_training(run)
        for head, status in (("0" * 40, ""), ("1" * 40, " M crates/minase/src/bin/minase/pst_probe.rs")):
            with self.subTest(head=head, status=status), \
                    patch.object(workflow, "git", side_effect=lambda repo, *args:
                                 head if args == ("rev-parse", "HEAD") else status), \
                    patch.object(workflow, "diagnose_probe") as probe, self.assertRaises(ValueError):
                workflow.diagnose(run)
            probe.assert_not_called()
            self.assertFalse((run / "diagnostics").exists())

    def test_cpu_training_and_diagnostics_preserve_scale_and_outputs(self) -> None:
        """指定した出力Kを量子化後も保持し、混合推定値は参考記録に限る。"""
        games = list(range(1, 81))
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=games)
        self.config["generate"]["seeds"] = [100, 200]
        self.config["train"]["rescore"] = ["-"] * 3
        self.config["generate"]["games"] = 80
        self.config["train"].update(epochs=1, batch=32, validation_sample=16)
        self.config["diagnose"]["sample_size"] = 16
        run, _ = self.prepared()
        checksum = (run / "pst-base.bin").read_bytes()[48:80]
        for seed in (100, 200):
            output = run / f"generated-{seed}.bin"
            write_mnsd(output, seed=seed, checksum=checksum, games=games)
            (run / f"generated-{seed}.json").write_text(
                json.dumps(workflow.input_receipt(output)))

        stdout = io.StringIO()
        with patch('minase_train.pst.teacher.estimate_mixed_k', return_value=321.25), redirect_stdout(stdout):
            workflow.train(run)
        candidate = run / "training/pst.bin"
        inputs = json.loads((run / "training/inputs.json").read_text())
        completion = json.loads((run / "training/complete.json").read_text())
        middlegame, endgame, piece_values, saved_k = read_mnpt(candidate)
        np.testing.assert_array_equal(middlegame, endgame)
        np.testing.assert_array_equal(piece_values, initial_piece_values())
        self.assertEqual(completion["float_sha256"], sha256_file(run / "training/pst-float.npz").hex())
        # 設定値は段階7の基準MNPTヘッダと同じfloat32の正確な値である。
        self.assertEqual(saved_k, 1072.6529541015625)
        self.assertEqual(inputs["k"], self.config["train"]["k"])
        self.assertEqual(inputs["mixed_k"], 321.25)
        self.assertIn("321.25", stdout.getvalue())
        self.assertIn("reference", stdout.getvalue())
        self.assertEqual(completion["sha256"], sha256_file(candidate).hex())
        self.assertEqual(inputs["training_records"] + inputs["validation_records"], 240)
        self.assertEqual(inputs["options"]["device"], "cpu")
        self.assertEqual(inputs["options"]["removal_penalty"], 0)

        with patch.object(workflow, "diagnose_probe", return_value=python_probe) as probe:
            workflow.diagnose(run)
        probe.assert_called_once_with(run / "probe/target/release/minase")
        report = json.loads((run / "diagnostics/report.json").read_text())
        self.assertEqual(len(report["bands"]), 10)
        validation = inputs["validation_records"]
        self.assertEqual(sum(entry["samples"] for entry in report["bands"]), validation)
        for entry in report["bands"]:
            self.assertTrue((run / "diagnostics" / entry["indices_file"]).is_file())
        original = candidate.read_bytes()
        with self.assertRaises(ValueError):
            workflow.train(run)
        self.assertEqual(candidate.read_bytes(), original)

    def test_training_forwards_and_records_mirrored_penalty_and_explicit_k(self) -> None:
        """指定した係数を保存してCLIへ渡し、教師尺度は訓練集合だけから推定する。"""
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=list(range(1, 81)))
        self.config["generate"]["seeds"] = []
        self.config["train"]["rescore"] = ["-"]
        self.config["train"].update(model="mirrored", k=1500.5, removal_penalty=2.5, lambda_override=0)
        run, stack = self.prepared()
        generation_ks = stack.enter_context(patch(
            'minase_train.pst.teacher.estimate_generation_ks', return_value=(np.array([np.nan]), None)))
        mixed_k = stack.enter_context(patch('minase_train.pst.teacher.estimate_mixed_k', return_value=888.0))
        execute = stack.enter_context(patch.object(
            workflow, "run_command", side_effect=subprocess.CalledProcessError(1, "train")))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.train(run)
        command = execute.call_args.args[2]
        self.assertEqual(command[:4], [sys.executable, "-m", "minase_train.pst.train", "train"])
        self.assertEqual(command[command.index("--lambda-override") + 1], "0")
        self.assertEqual(command[command.index("--model") + 1], "mirrored")
        self.assertEqual(command[command.index("--k") + 1], "1500.5")
        self.assertEqual(command[command.index("--removal-penalty") + 1], "2.5")
        inputs = json.loads((run / "training/inputs.json").read_text())
        self.assertEqual(inputs["options"], self.config["train"])
        self.assertEqual(inputs["options"]["removal_penalty"], 2.5)
        self.assertEqual(inputs["lambda_override"], 0)
        self.assertEqual(inputs["teacher_ks"], [None])
        self.assertEqual(inputs["teacher_classes"][0]["lambda"], 0)
        workflow.load_prepared(run)
        for estimate in (generation_ks, mixed_k):
            dataset = estimate.call_args.args[0]
            np.testing.assert_array_equal(estimate.call_args.kwargs["indices"],
                                          dataset.training_indices)

    def test_source_hashes_include_nested_sources_and_distinct_relative_paths(self):
        package = self.root / "tools/train/src/minase_train"
        contents = {
            "__init__.py": b"",
            "data/__init__.py": b"",
            "data/shared.py": b"value = 1\n",
            "pst/shared.py": b"value = 2\n",
            "data/test_source.py": b"value = 3\n",
            "notes.txt": b"not a Python source\n",
        }
        for name, content in contents.items():
            path = package / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        expected = {name: hashlib.sha256(content).hexdigest()
                    for name, content in contents.items() if name.endswith(".py")}
        with patch.object(workflow, "SOURCES", package):
            self.assertEqual(workflow.source_hashes(), expected)

    def test_source_changes_additions_and_deletions_block_all_later_stages(self):
        package = self.root / "tools/train/src/minase_train"
        source = package / "data/mnsd.py"
        source.parent.mkdir(parents=True)
        source.write_text("value = 1\n")
        (package / "__init__.py").write_text("")
        with patch.object(workflow, "SOURCES", package):
            run, _ = self.prepared()
            added = package / "data/added.py"
            for change in ("modify", "add", "delete"):
                with self.subTest(change=change):
                    if change == "modify":
                        source.write_text("value = 2\n")
                    elif change == "add":
                        added.write_text("value = 3\n")
                    else:
                        source.unlink()
                    for operation, args in ((workflow.generate, (run, None)),
                                            (workflow.train, (run,)), (workflow.diagnose, (run,))):
                        with self.subTest(operation=operation.__name__):
                            with self.assertRaisesRegex(ValueError, "training tools changed"):
                                operation(*args)
                    source.write_text("value = 1\n")
                    if added.exists():
                        added.unlink()
                    workflow.load_prepared(run)
            tests = self.root / "tools/train/tests"
            tests.mkdir()
            test_source = tests / "test_mnsd.py"
            for content in ("value = 1\n", "value = 2\n"):
                test_source.write_text(content)
                workflow.load_prepared(run)
            test_source.unlink()
            workflow.load_prepared(run)
            self.assertFalse((run / "training").exists())
            self.assertFalse((run / "diagnostics").exists())
            self.assertFalse((run / "generated-100.bin").exists())

    def test_root_matches_actual_worktree(self):
        expected = Path(subprocess.check_output(
            ["git", "-C", str(Path(__file__).resolve().parent), "rev-parse", "--show-toplevel"],
            text=True,
        ).strip()).resolve()
        self.assertEqual(workflow.ROOT, expected)
        self.assertTrue((workflow.ROOT / "Cargo.toml").is_file())

    def test_import_rejects_root_without_cargo_manifest(self):
        import runpy
        misplaced = self.root / "tools/train/src/minase_train/workflow.py"
        misplaced.parent.mkdir(parents=True)
        misplaced.write_text(Path(workflow.__file__).read_text())
        with self.assertRaisesRegex(RuntimeError, "Cargo.toml"):
            runpy.run_path(str(misplaced))

if __name__ == "__main__":
    unittest.main()
