//! dump-batchのCPU経路が再開可能な状態と3更新の照合資料を出力することを確認する。
mod common;
use common::*;
use nnue_trainer::{
    data::InputFeatureSet,
    model::{Checkpoint, Parameter},
};

#[test]
fn dump_reference_writes_warmed_checkpoint_three_batches_gradients_and_states() {
    let fixture = Fixture::new(&positions());
    let initial = fixture.root.join("pst-float.npz");
    std::fs::write(&initial, npz_bytes(&vec![[0.0; 2]; 13680], true)).unwrap();
    for (set, name) in [
        (InputFeatureSet::P, "P"),
        (InputFeatureSet::HalfKpHmB8, "HalfKP-hm-b8"),
    ] {
        let directory = fixture.root.join(name);
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_nnue-trainer"))
            .args(["dump-batch", "--feature-set", name, "--data"])
            .arg(fixture.root.join("records.bin"))
            .args(["--indices", "0,1,2", "--output"])
            .arg(&directory)
            .arg("--init-pst")
            .arg(&initial)
            .args([
                "--width",
                "4",
                "--k",
                "1000",
                "--seed",
                "71",
                "--steps",
                "3",
                "--lookahead-gamma",
                "0.9",
                "--lookahead-plies",
                "40",
                "--reference-batch",
                "2,0,1",
                "--reference-batch",
                "1,2",
                "--reference-batch",
                "0,1,2",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved = Checkpoint::read(directory.join("checkpoint.bin")).unwrap();
        assert_eq!(saved.model.shape().feature_set(), set);
        assert_eq!(saved.optimizer.step(), 3);
        assert!(
            saved.model.parameters()[Parameter::OutputWeights]
                .iter()
                .any(|&x| x != 0.0)
        );
        assert!(
            saved.optimizer.m()[Parameter::OutputWeights]
                .iter()
                .any(|&x| x != 0.0)
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("reference.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["steps"].as_array().unwrap().len(), 3);
        for (i, step) in manifest["steps"].as_array().unwrap().iter().enumerate() {
            let after = Checkpoint::read(directory.join(step["after"].as_str().unwrap())).unwrap();
            assert_eq!(after.optimizer.step(), 4 + i as u64);
            let bytes = std::fs::read(directory.join(step["gradients"].as_str().unwrap())).unwrap();
            assert_eq!(
                bytes.len(),
                4 * Parameter::ALL
                    .iter()
                    .map(|&p| saved.model.shape().len(p))
                    .sum::<usize>()
            );
            assert!(step["losses"]["bce"].as_f64().unwrap() > 0.0);
            let batch: serde_json::Value = serde_json::from_slice(
                &std::fs::read(directory.join(step["batch"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            assert_eq!(batch["lookahead"]["gamma"], 0.9);
            assert_eq!(batch["record_indices"][0], [2, 1, 0][i]);
        }
    }
}
