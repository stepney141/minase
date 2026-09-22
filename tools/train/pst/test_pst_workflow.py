"""明示した設定と完了記録だけで学習工程を進める契約を検証する。"""

from contextlib import ExitStack, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from features import FEATURE_COUNT
import pst_workflow as workflow
from test_pst_diagnostics import python_probe
from test_train_pst import write_mnsd
from train_pst import initial_piece_values, read_mnpt, write_mnpt


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
        for content in (CONFIG.replace("epochs = 10\n", ""),
                        CONFIG.replace("epochs = 10", "epochs = 10\nepoch = 10"),
                        CONFIG + "\n[extra]\nvalue = 1\n"):
            with self.subTest(content=content):
                self.config_path.write_text(content)
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_output_k_must_be_explicit(self) -> None:
        """段階7の出力K固定契約に従い、省略した設定は推定前に拒否する。"""
        self.config_path.write_text(CONFIG.replace("k = 1072.6529541015625\n", ""))
        with self.assertRaises(ValueError):
            workflow.load_config(self.config_path, self.root)

    def test_removal_penalty_must_be_explicit_for_every_model(self) -> None:
        """追加損失を使わないモデルでも、係数0の明示を要求する。"""
        for model in ("single", "tapered", "mirrored"):
            with self.subTest(model=model):
                self.config_path.write_text(CONFIG.replace(
                    'model = "single"', f'model = "{model}"').replace(
                    "removal_penalty = 0\n", ""))
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_zero_removal_penalty_is_accepted_for_every_model(self) -> None:
        for model in ("single", "tapered", "mirrored"):
            for zero in ("0", "0.0"):
                with self.subTest(model=model, zero=zero):
                    self.config_path.write_text(CONFIG.replace(
                        'model = "single"', f'model = "{model}"').replace(
                        "removal_penalty = 0", f"removal_penalty = {zero}"))
                    config = workflow.load_config(self.config_path, self.root)
                    self.assertEqual(config["train"]["removal_penalty"], 0)

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

    def test_removal_penalty_rejects_negative_nonfinite_and_nonnumeric_values(self) -> None:
        for value in ("-0.01", "nan", "inf", "-inf", "true", "false", '"0"'):
            with self.subTest(value=value):
                self.config_path.write_text(CONFIG.replace(
                    'model = "single"', 'model = "mirrored"').replace(
                    "removal_penalty = 0", f"removal_penalty = {value}"))
                with self.assertRaisesRegex(ValueError, "removal_penalty"):
                    workflow.load_config(self.config_path, self.root)

    def test_mirrored_model_is_an_explicit_choice(self) -> None:
        self.config_path.write_text(CONFIG.replace('model = "single"', 'model = "mirrored"'))
        self.assertEqual(workflow.load_config(self.config_path, self.root)["train"]["model"],
                         "mirrored")

    def test_example_explicitly_disables_unselected_removal_penalty(self) -> None:
        """未選定の係数を設定例で仮定せず、必須項目をすべて明示する。"""
        example = Path(__file__).with_name("pst.example.toml").read_text()
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
        run = self.root / "data/run"
        base = "0" * 40
        tools_commit = "1" * 40
        checkouts = {}
        builds = {}
        from test_phase4 import write_rescore
        source = self.root / "data/old.bin"
        sidecar = self.root / "prepare-rescore.bin"
        write_rescore(sidecar, source, [(1, 0, 20, 1, 100)] * workflow.read_header(source).record_count)
        self.config["train"]["rescore"][0] = str(sidecar)

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
                (destination / "nets").mkdir(parents=True)
                weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
                write_mnpt(destination / "nets/pst.bin", weights, weights,
                           initial_piece_values(), 1000)
            elif command[:2] == ["cargo", "build"]:
                names = [command[index + 1] for index, value in enumerate(command) if value == "--bin"]
                builds[cwd] = names
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
        self.assertEqual(state["rescores"], [{"path": str(sidecar), "sha256": workflow.digest(sidecar)}])
        provenance = Path(str(source) + ".provenance.json")
        self.assertEqual(state["existing_data"][0]["provenance"]["sha256"], workflow.digest(provenance))
        self.assertEqual(checkouts, {run / "generator": base, run / "probe": tools_commit})
        self.assertEqual(builds, {run / "generator": ["selfplay_gen"], run / "probe": ["pst_probe"]})
        self.assertEqual(state["config"]["run"]["base_commit"], base)
        self.assertEqual(state["config"]["train"]["removal_penalty"], 0)
        self.assertEqual(state["probe_commit"], tools_commit)
        self.assertEqual(state["generator_sha256"], workflow.digest(run / "generator/target/release/selfplay_gen"))
        self.assertEqual(state["probe_sha256"], workflow.digest(run / "probe/target/release/pst_probe"))

    def prepared(self) -> tuple[Path, ExitStack]:
        """外部git操作だけを置換し、完了記録とファイル検証は実際に通す。"""
        run = self.root / "data/run"
        run.mkdir()
        binary = run / "generator/target/release/selfplay_gen"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture executable")
        probe = run / "probe/target/release/pst_probe"
        probe.parent.mkdir(parents=True)
        probe.write_bytes(b"fixture probe")
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(run / "pst-base.bin", zeros, zeros, initial_piece_values(), 1000)
        state = {
            "config": self.config,
            "lookahead": self.config["train"].get("lookahead"),
            "lambda_override": self.config["train"].get("lambda_override"),
            "existing_data": workflow.check_existing_data(self.config),
            "rescores": [{"path": p, "sha256": workflow.digest(Path(p))} for p in self.config["train"]["rescore"] if p != "-"],
            "sources": workflow.source_hashes(),
            "base_sha256": workflow.digest(run / "pst-base.bin"),
            "base_piece_values_sha256": hashlib.sha256(
                (run / "pst-base.bin").read_bytes()[-workflow.PIECE_VALUE_BYTES:]).hexdigest(),
            "generator_sha256": workflow.digest(binary),
            "probe_commit": "1" * 40,
            "probe_sha256": workflow.digest(probe),
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
        for value in ('{ gamma = 0.9 }', '{ gamma = 1.0, plies = 40 }',
                      '{ gamma = 0.9, plies = 4097 }', '{ gamma = 0.9, plies = 1.5 }',
                      '{ gamma = 0.9, plies = 40, extra = 1 }', 'false'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlookahead = ' + value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                workflow.load_config(self.config_path, self.root)
        self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlookahead = { gamma = 0.9, plies = 40 }\nrescore = ["a", "-", "-"]'))
        with self.assertRaisesRegex(ValueError, 'lookahead.*rescore'):
            workflow.load_config(self.config_path, self.root)

    def test_prepare_records_lookahead_and_rejects_changed_setting(self):
        self.config['train']['lookahead'] = {'gamma': .9, 'plies': 40}
        self.config['train']['lambda_override'] = 1.0
        def command(run, label, argv, cwd):
            if argv[:3] == ['git', 'worktree', 'add']:
                destination = Path(argv[-2])
                (destination / 'nets').mkdir(parents=True)
                weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
                write_mnpt(destination / 'nets/pst.bin', weights, weights, initial_piece_values(), 1000)
            elif argv[:2] == ['cargo', 'build']:
                target = Path(argv[argv.index('--target-dir') + 1]) / 'release'
                target.mkdir(parents=True)
                (target / argv[argv.index('--bin') + 1]).write_bytes(b'fixture')
            else:
                raise AssertionError(argv)
        with patch.object(workflow, 'ROOT', self.root), \
                patch.object(workflow, 'load_config', return_value=self.config), \
                patch.object(workflow, 'git', return_value='0' * 40), \
                patch.object(workflow, 'run_command', side_effect=command):
            workflow.prepare(self.config_path)
            run = self.root / 'data/run'
            state = workflow.load_prepared(run)
            self.assertEqual(state['lookahead'], {'gamma': .9, 'plies': 40})
            self.assertEqual(state['lambda_override'], 1.0)
            state['config']['train']['lookahead']['gamma'] = .7
            (run / 'prepared.json').write_text(json.dumps(state))
            with self.assertRaisesRegex(ValueError, 'lookahead'):
                workflow.load_prepared(run)

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
        from mnsd import Dataset
        from pst_diagnostics import Weights
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
            for operation in (workflow.load_prepared, workflow.train, workflow.diagnose):
                with self.subTest(name=name, operation=operation), self.assertRaisesRegex(ValueError, 'lookahead'):
                    operation(run)
            path.write_text(json.dumps(record))

    def test_lambda_override_config_validation(self):
        self.assertNotIn('lambda_override', self.config['train'])
        for value in ('0', '0.5', '1.0'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlambda_override = ' + value))
            config = workflow.load_config(self.config_path, self.root)
            self.assertEqual(config['train']['lambda_override'], float(value))
        for value in ('-0.01', '1.01', 'nan', 'inf', '-inf', 'true', '"0"'):
            self.config_path.write_text(CONFIG.replace('[train]', '[train]\nlambda_override = ' + value))
            with self.subTest(value=value), self.assertRaises(ValueError):
                workflow.load_config(self.config_path, self.root)

    def test_lambda_override_prepare_mismatch_blocks_all_operations(self):
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
            for operation in (workflow.load_prepared, workflow.train, workflow.diagnose):
                with self.subTest(operation=operation), self.assertRaisesRegex(ValueError, 'lambda_override'):
                    operation(run)
        (run / 'prepared.json').write_text(original)
        self.assertEqual(workflow.load_prepared(run)['lambda_override'], 0)

    def test_lambda_override_training_diagnosis_and_record_mismatches(self):
        from mnsd import Dataset
        from test_train_pst import write_provenance
        from train_pst import estimate_generation_ks
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
                for operation in (workflow.load_prepared, workflow.train, workflow.diagnose):
                    with self.subTest(path=path, operation=operation), self.assertRaises(ValueError):
                        operation(run)
            path.write_text(original)
        workflow.load_prepared(run)
        report = json.loads(paths[-1].read_text())
        self.assertEqual(report['outcome_metrics']['candidate']['records'],
                         len(reference.validation_indices))
        commands = [json.loads(line) for line in (run / 'commands.jsonl').read_text().splitlines()]
        argv = next(row['argv'] for row in commands if 'argv' in row)
        self.assertEqual(argv[argv.index('--lambda-override') + 1], '1')

    def test_lambda_override_explicit_zero_is_forwarded_and_recorded(self):
        write_mnsd(self.root / 'data/old.bin', seed=0, checksum=b'a' * 32, games=list(range(1, 81)))
        self.config['generate']['seeds'] = []
        self.config['train'].update(rescore=['-'], lambda_override=0, epochs=1, batch=64)
        run, _ = self.prepared()
        with redirect_stdout(io.StringIO()):
            workflow.train(run)
        commands = [json.loads(line) for line in (run / 'commands.jsonl').read_text().splitlines()]
        argv = next(row['argv'] for row in commands if 'argv' in row)
        self.assertEqual(argv[argv.index('--lambda-override') + 1], '0')
        for name in ('inputs.json', 'pst.training.json'):
            record = json.loads((run / 'training' / name).read_text())
            self.assertEqual(record['lambda_override'], 0)
            self.assertEqual(record['teacher_ks'], [None])
            self.assertEqual(record['teacher_classes'][0]['lambda'], 0)
        workflow.load_prepared(run)

    def test_human_games_do_not_reserve_generator_seed_ranges(self):
        from test_train_pst import write_provenance
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
        from test_phase4 import write_rescore
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

    def test_generated_provenance_change_blocks_reuse(self):
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        write_mnsd(output, seed=100, checksum=(run / "pst-base.bin").read_bytes()[48:80], games=[1])
        (run / "generated-100.json").write_text(json.dumps(workflow.input_receipt(output)))
        provenance = Path(str(output) + ".provenance.json")
        provenance.write_text(provenance.read_text() + "\n")
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
            "sha256": workflow.digest(candidate), "float_sha256": workflow.digest(float_path)}))
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
        (run / "probe/target/release/pst_probe").write_bytes(b"modified probe")
        with patch.object(workflow, "diagnose_probe") as probe, self.assertRaises(ValueError):
            workflow.diagnose(run)
        probe.assert_not_called()
        self.assertFalse((run / "diagnostics").exists())

    def test_diagnosis_rejects_modified_probe_worktree(self) -> None:
        run, _ = self.prepared()
        self.completed_training(run)
        for head, status in (("0" * 40, ""), ("1" * 40, " M src/bin/pst_probe.rs")):
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
        with patch("train_pst.estimate_mixed_k", return_value=321.25), redirect_stdout(stdout):
            workflow.train(run)
        candidate = run / "training/pst.bin"
        inputs = json.loads((run / "training/inputs.json").read_text())
        completion = json.loads((run / "training/complete.json").read_text())
        middlegame, endgame, piece_values, saved_k = read_mnpt(candidate)
        np.testing.assert_array_equal(middlegame, endgame)
        np.testing.assert_array_equal(piece_values, initial_piece_values())
        self.assertEqual(completion["float_sha256"], workflow.digest(run / "training/pst-float.npz"))
        # 設定値は段階7の基準MNPTヘッダと同じfloat32の正確な値である。
        self.assertEqual(saved_k, 1072.6529541015625)
        self.assertEqual(inputs["k"], self.config["train"]["k"])
        self.assertEqual(inputs["mixed_k"], 321.25)
        self.assertIn("321.25", stdout.getvalue())
        self.assertIn("reference", stdout.getvalue())
        self.assertEqual(completion["sha256"], workflow.digest(candidate))
        self.assertEqual(inputs["training_records"] + inputs["validation_records"], 240)
        self.assertEqual(inputs["options"]["device"], "cpu")
        self.assertEqual(inputs["options"]["removal_penalty"], 0)

        with patch.object(workflow, "diagnose_probe", return_value=python_probe) as probe:
            workflow.diagnose(run)
        probe.assert_called_once_with(run / "probe/target/release/pst_probe")
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
        self.config["train"].update(model="mirrored", k=1500.5, removal_penalty=2.5)
        run, stack = self.prepared()
        generation_ks = stack.enter_context(patch(
            "train_pst.estimate_generation_ks", return_value=(np.array([777.0]), None)))
        mixed_k = stack.enter_context(patch("train_pst.estimate_mixed_k", return_value=888.0))
        execute = stack.enter_context(patch.object(
            workflow, "run_command", side_effect=subprocess.CalledProcessError(1, "train")))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.train(run)
        command = execute.call_args.args[2]
        self.assertEqual(command[command.index("--model") + 1], "mirrored")
        self.assertEqual(command[command.index("--k") + 1], "1500.5")
        self.assertEqual(command[command.index("--removal-penalty") + 1], "2.5")
        inputs = json.loads((run / "training/inputs.json").read_text())
        self.assertEqual(inputs["options"], self.config["train"])
        self.assertEqual(inputs["options"]["removal_penalty"], 2.5)
        for estimate in (generation_ks, mixed_k):
            dataset = estimate.call_args.args[0]
            np.testing.assert_array_equal(estimate.call_args.kwargs["indices"],
                                          dataset.training_indices)

    def test_training_forwards_explicit_zero_removal_penalty(self) -> None:
        """追加損失を無効にする0もCLI境界で省略しない。"""
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=list(range(1, 81)))
        self.config["generate"]["seeds"] = []
        self.config["train"]["rescore"] = ["-"]
        run, stack = self.prepared()
        stack.enter_context(patch(
            "train_pst.estimate_generation_ks", return_value=(np.array([777.0]), None)))
        stack.enter_context(patch("train_pst.estimate_mixed_k", return_value=888.0))
        execute = stack.enter_context(patch.object(
            workflow, "run_command", side_effect=subprocess.CalledProcessError(1, "train")))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.train(run)
        command = execute.call_args.args[2]
        self.assertEqual(command[command.index("--removal-penalty") + 1], "0")
        inputs = json.loads((run / "training/inputs.json").read_text())
        self.assertEqual(inputs["options"]["removal_penalty"], 0)

    def test_extra_config_requires_complete_consistent_options(self) -> None:
        extra = ('king_features = ["data/old.mnkf"]\nextra_columns = "0:24,62:68"\n'
                 'train_extra = "24:30"\nfreeze_pst = true\n')
        configured = CONFIG.replace("seeds = [100, 110]", "seeds = []").replace("[diagnose]", extra + "[diagnose]")
        self.config_path.write_text(configured)
        result = workflow.load_config(self.config_path, self.root)["train"]
        self.assertEqual(result["king_features"], [str(self.root / "data/old.mnkf")])
        self.assertEqual(result["extra_columns"], "0:24,62:68")
        self.assertEqual(result["train_extra"], "24:30")
        self.assertTrue(result["freeze_pst"])
        for before, after in (
            ('train_extra = "24:30"\n', ''),
            ('train_extra = "24:30"', 'train_extra = "24:31"'),
            ('extra_columns = "0:24,62:68"', 'extra_columns = "0:24,23:68"'),
            ('freeze_pst = true', 'freeze_pst = 1'),
            ('king_features = ["data/old.mnkf"]', 'king_features = []'),
            ('removal_penalty = 0', 'removal_penalty = 1'),
        ):
            with self.subTest(after=after), self.assertRaises(ValueError):
                self.config_path.write_text(configured.replace(before, after))
                workflow.load_config(self.config_path, self.root)

    def test_training_forwards_extra_options_and_records_feature_checksum(self) -> None:
        from mnsd import HEADER, map_records
        data = self.root / "data/old.bin"
        write_mnsd(data, seed=0, checksum=b"a" * 32, games=list(range(80)))
        feature_path = self.root / "data/old.mnkf"
        values = np.zeros((len(map_records(data)), 118), dtype=np.uint8)
        feature_path.write_bytes(HEADER.pack(b"MNKF", 1, 2, 118, len(values),
                                            hashlib.sha256(data.read_bytes()).digest()) + values.tobytes())
        self.config["generate"]["seeds"] = []
        self.config["train"]["rescore"] = ["-"]
        self.config["train"].update(king_features=[str(feature_path)], extra_columns="0:24,112:118",
                                    train_extra="24:30", freeze_pst=True, k=1000)
        run, stack = self.prepared()
        execute = stack.enter_context(patch.object(
            workflow, "run_command", side_effect=subprocess.CalledProcessError(1, "train")))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.train(run)
        command = execute.call_args.args[2]
        for flag, expected in (("--king-features", str(feature_path)),
                               ("--extra-columns", "0:24,112:118"), ("--train-extra", "24:30")):
            self.assertEqual(command[command.index(flag) + 1], expected)
        self.assertIn("--freeze-pst", command)
        inputs = json.loads((run / "training/inputs.json").read_text())
        self.assertEqual(inputs["king_features"], [{"path": str(feature_path),
                                                   "sha256": workflow.digest(feature_path)}])
        self.assertEqual(inputs["mnkf_definition_id"], 2)
        self.assertEqual(inputs["mnkf_column_count"], 118)
        workflow.load_prepared(run)
        for field, changed in (("mnkf_definition_id", 1), ("mnkf_column_count", 68)):
            altered = dict(inputs, **{field: changed})
            (run / "training/inputs.json").write_text(json.dumps(altered))
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "MNKF"):
                workflow.load_prepared(run)
        (run / "training/inputs.json").write_text(json.dumps(inputs))
        damaged = bytearray(feature_path.read_bytes())
        damaged[-1] = 1
        feature_path.write_bytes(damaged)
        with self.assertRaisesRegex(ValueError, "checksum"):
            workflow.load_prepared(run)


if __name__ == "__main__":
    unittest.main()
