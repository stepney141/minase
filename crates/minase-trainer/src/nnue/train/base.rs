//! 基準MNPTから初期値の照合用PSTと書き出し用の固定値を読む。

use minase::eval::{features::PIECE_STATE_COUNT, pst::Pst};
use std::{error::Error, path::Path};

/// 基準MNPTが保持する線形重みと探索用の固定値。
#[derive(Debug)]
pub struct Base {
    /// 特徴番号順の序中盤・終盤重み。単位は1/8センチポーン。
    pub linear: Vec<[i16; 2]>,
    /// 駒状態番号順の探索用駒価値。単位はセンチポーン。
    pub piece_values: [i32; PIECE_STATE_COUNT],
    /// 出力のロジットをセンチポーンへ換算する尺度。
    pub k: f32,
    /// 元のMNPTが指定する規則セット名。
    pub rule_set: String,
}

/// MNPT全体をminaseの読み手で検証し、NNUEで引き継ぐ値を返す。
pub fn read(path: &Path) -> Result<Base, Box<dyn Error>> {
    decode(&std::fs::read(path)?)
}

fn decode(bytes: &[u8]) -> Result<Base, Box<dyn Error>> {
    let pst = Pst::decode(bytes)?;
    Ok(Base {
        linear: pst.weights().to_vec(),
        piece_values: std::array::from_fn(|state| pst.piece_value_of_state(state)),
        k: pst.k(),
        rule_set: pst.rule_set().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nn::sha256;
    use crate::nnue::reference::v0_piece_values;
    use minase::eval::{
        features::FEATURE_COUNT,
        pst::{FM_RANK, FmWeights},
    };

    /// MNPT版3の配置で作る。
    /// 両端点、規則名、Kは基準の埋め込み値と異なる既知の値を使う。
    fn fixture() -> Vec<u8> {
        Pst::from_weights_with_fm(
            &vec![[123, -456]; FEATURE_COUNT],
            v0_piece_values(),
            317.25,
            "L1,P0,R1,E0",
            FmWeights {
                embeddings: &vec![[0; FM_RANK]; FEATURE_COUNT],
                signs: &[1; FM_RANK],
                exponent: 7,
            },
        )
        .unwrap()
        .encode()
    }

    fn rehash(bytes: &mut [u8]) {
        let checksum = sha256(&bytes[80..]);
        bytes[48..80].copy_from_slice(&checksum);
    }

    #[test]
    fn mnpt_base_preserves_both_endpoints_and_fixed_metadata() {
        let mut bytes = fixture();
        // 特徴ごとに2列を対応づけ、符号とi16の境界を保つ。
        bytes[80 + 17 * 2..80 + 18 * 2].copy_from_slice(&i16::MIN.to_le_bytes());
        let end_offset = 80 + FEATURE_COUNT * 2 + 17 * 2;
        bytes[end_offset..end_offset + 2].copy_from_slice(&i16::MAX.to_le_bytes());
        rehash(&mut bytes);
        let base = decode(&bytes).unwrap();
        let mut expected = vec![[123, -456]; FEATURE_COUNT];
        expected[17] = [i16::MIN, i16::MAX];
        assert_eq!(base.linear, expected);
        assert_eq!(base.piece_values, v0_piece_values());
        assert_eq!(base.k.to_bits(), 317.25_f32.to_bits());
        assert_eq!(base.rule_set, "L1,P0,R1,E0");
    }

    #[test]
    fn mnpt_base_rejects_damage_in_the_unused_fm_section() {
        let mut bytes = fixture();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(matches!(
            decode(&bytes).unwrap_err().downcast_ref(),
            Some(minase::eval::pst::Error::ChecksumMismatch)
        ));
    }

    #[test]
    fn mnpt_base_rejects_truncation_and_trailing_data() {
        let bytes = fixture();
        for length in [0, 79, 80, bytes.len() - 1] {
            assert!(decode(&bytes[..length]).is_err(), "length={length}");
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(decode(&extra).is_err());
    }

    #[test]
    fn mnpt_base_rejects_invalid_output_scale_and_rule_padding() {
        for k in [0.0_f32, -1.0, f32::INFINITY, f32::NAN] {
            let mut bytes = fixture();
            bytes[12..16].copy_from_slice(&k.to_le_bytes());
            assert!(decode(&bytes).is_err(), "K={k}");
        }
        let mut bytes = fixture();
        bytes[47] = b'X';
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn mnpt_base_rejects_invalid_piece_values_even_with_valid_checksum() {
        let mut bytes = fixture();
        let values_offset = 80 + FEATURE_COUNT * 4;
        bytes[values_offset..values_offset + 4].copy_from_slice(&(-1_i32).to_le_bytes());
        rehash(&mut bytes);
        assert!(matches!(
            decode(&bytes).unwrap_err().downcast_ref(),
            Some(minase::eval::pst::Error::PieceValueOutOfRange { .. })
        ));
    }
}
