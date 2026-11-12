//! MNUE版5の符号化と復号を、実ファイルと公開APIを通して検査する。

use minase::eval::features::{FEATURE_COUNT, PIECE_STATE_COUNT};
use minase::eval::nnue::feature_set::{FeatureSet, HalfKpHmB8, P};
use minase::eval::nnue::head::{Head, PairwiseL16L32};
use minase::eval::nnue::{EncodeInput, Network, architecture, encode, evaluate};
use minase_core::{PieceKind, Position};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;

struct NetworkFile(PathBuf);

impl Drop for NetworkFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

fn round_trip<F: FeatureSet>() {
    let width = 8;
    let mut piece_values = [100; PIECE_STATE_COUNT];
    piece_values[PieceKind::King.index()] = 200;
    piece_values[PieceKind::CrownPrince.index()] = 200;
    let head = PairwiseL16L32::new(
        width,
        &[1024, 1024, 1024, 1024, -2048, -2048, -2048, -2048],
        4096,
        &[12; 15 * 8],
        [0; 15],
        &[4; 32 * 30],
        [0; 32],
        [2; 32],
        true,
    )
    .unwrap();
    let bytes = encode::<F, _>(&EncodeInput {
        width: width as u32,
        cp_per_logit: 7.9375,
        rule_set: "JCSA",
        piece_values: &piece_values,
        linear: &vec![[0; 2]; FEATURE_COUNT],
        weights: &vec![0; F::DIMENSIONS * width],
        biases: &[128; 8],
        head: &head,
    })
    .unwrap();
    let file = NetworkFile(std::env::temp_dir().join(format!(
        "minase-nnue-format-{}-{}.bin",
        std::process::id(),
        F::NAME,
    )));
    {
        let mut writer = std::fs::File::create_new(&file.0).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    let read = std::fs::read(&file.0).unwrap();
    assert_eq!(read, bytes);
    let network = Network::<F, PairwiseL16L32>::decode(&read).unwrap();
    assert_eq!(network.width(), width);
    assert_eq!(network.cp_per_logit(), 7.9375);
    assert_eq!(network.rule_set(), "JCSA");
    assert_eq!(
        network.checksum(),
        &<[u8; 32]>::from(Sha256::digest(&read[192..]))
    );
    assert_eq!(
        architecture(&read).unwrap(),
        format!(
            "features={};ft=8;linear=tapered2;head=pairwise-l16-l32",
            F::NAME,
        )
    );
    // s=32、z=3,072、u=(1を15個, 12を15個)、y=3。
    // out=−131,072+32×2×3+4,096=−126,784、scale=1なので−2 cpとなる。
    assert_eq!(head.output(&[128; 8], &[128; 8]), -126_784);
    assert_eq!(evaluate(&network, &Position::initial()), -2);
    let offset =
        192 + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4 + F::DIMENSIONS * 2 * width + 4 * width;
    let decoded_head = PairwiseL16L32::decode(&read[offset..], width, true).unwrap();
    assert_eq!(decoded_head.to_bytes(), head.to_bytes());
}

#[test]
fn p_network_round_trips_through_a_file() {
    round_trip::<P>();
}

#[test]
fn half_kp_network_round_trips_through_a_file() {
    round_trip::<HalfKpHmB8>();
}
