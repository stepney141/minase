//! 基準MNPTから初期値の照合用PSTと書き出し用の固定値を読む。

use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT},
    pst::Pst,
};
use std::{error::Error, path::Path};

const HEADER_BYTES: usize = 80;

/// 基準Mが保持する線形重みと探索用の固定値。
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
    // Pst::decodeがヘッダ、固定長、FM節、検査和を検証済み。
    // 公開APIに線形重みと規則名のgetterがないため、その2項目だけを読む。
    let middle_end = HEADER_BYTES + FEATURE_COUNT * 2;
    let end_end = middle_end + FEATURE_COUNT * 2;
    let linear = bytes[HEADER_BYTES..middle_end]
        .as_chunks::<2>()
        .0
        .iter()
        .zip(bytes[middle_end..end_end].as_chunks::<2>().0)
        .map(|(middle, end)| {
            [
                i16::from_le_bytes([middle[0], middle[1]]),
                i16::from_le_bytes([end[0], end[1]]),
            ]
        })
        .collect();
    Ok(Base {
        linear,
        piece_values: std::array::from_fn(|state| pst.piece_value_of_state(state)),
        k: pst.k(),
        rule_set: std::str::from_utf8(&bytes[16..48])?
            .trim_end_matches('\0')
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::v0_piece_values;
    use minase::eval::pst::FM_RANK;
    use sha2::{Digest, Sha256};

    /// tools/train/src/minase_train/data/mnpt.pyの版3の配置で作る。
    /// 両端点、規則名、Kは基準の埋め込み値と異なる既知の値を使う。
    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MNPT");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&(FEATURE_COUNT as u32).to_le_bytes());
        bytes.extend_from_slice(&317.25_f32.to_le_bytes());
        bytes.extend_from_slice(b"L1,P0,R1,E0");
        bytes.resize(80, 0);
        for endpoint in [123_i16, -456_i16] {
            for _ in 0..FEATURE_COUNT {
                bytes.extend_from_slice(&endpoint.to_le_bytes());
            }
        }
        for value in v0_piece_values() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&(FM_RANK as u32).to_le_bytes());
        bytes.extend_from_slice(&7_u32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(1_u8, FM_RANK));
        bytes.resize(bytes.len() + FEATURE_COUNT * FM_RANK * 2, 0);
        rehash(&mut bytes);
        bytes
    }

    fn rehash(bytes: &mut [u8]) {
        let checksum = Sha256::digest(&bytes[80..]);
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
