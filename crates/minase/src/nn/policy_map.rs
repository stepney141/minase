//! ONNXのメタデータを検査し、元の方策ラベルから詰めた後の添字を引く。

use super::{NnError, POLICY_SIZE};

/// 厳密な昇順、範囲、および出力幅との一致を検査済みの方策の逆引き表。
#[derive(Debug)]
pub(super) struct PolicyMap {
    indices: Vec<Option<usize>>,
    width: usize,
}

impl PolicyMap {
    /// カンマ区切りの10進ラベルを検査し、欠けたラベルを`None`で表す逆引き表を作る。
    ///
    /// # Errors
    /// メタデータの欠落、空の表、整数解析、順序、範囲、または要素数の違反を
    /// それぞれ対応する[`NnError`]で返す。
    pub(super) fn from_metadata(labels: Option<&str>, width: usize) -> Result<Self, NnError> {
        let labels = labels.ok_or(NnError::MissingPolicyLabels)?;
        if labels.is_empty() {
            return Err(NnError::EmptyPolicyLabels);
        }
        let mut indices = vec![None; POLICY_SIZE];
        let mut previous = None;
        let mut count = 0;
        for (index, value) in labels.split(',').enumerate() {
            let label = value
                .parse::<usize>()
                .map_err(|source| NnError::InvalidPolicyLabel {
                    index,
                    value: value.to_owned(),
                    source,
                })?;
            if label >= POLICY_SIZE {
                return Err(NnError::PolicyLabelOutOfRange { index, label });
            }
            if let Some(previous) = previous
                && label <= previous
            {
                return Err(NnError::PolicyLabelsNotAscending {
                    index,
                    previous,
                    label,
                });
            }
            indices[label] = Some(index);
            previous = Some(label);
            count += 1;
        }
        if count != width {
            return Err(NnError::PolicyLabelCount {
                expected: width,
                actual: count,
            });
        }
        Ok(Self { indices, width })
    }

    /// ネットの方策の出力幅を返す。
    pub(super) fn width(&self) -> usize {
        self.width
    }

    /// 元のラベルから詰めた後の添字を返し、表にないラベルには`None`を返す。
    pub(super) fn packed_index(&self, label: usize) -> Option<usize> {
        self.indices.get(label).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::num::IntErrorKind;

    // メタデータの契約: 昇順のラベルを詰めた順に対応付け、元のラベル範囲の両端を受理する。
    #[test]
    fn maps_labels_to_packed_indices_and_preserves_absence() {
        let map = PolicyMap::from_metadata(Some("0,65,35855"), 3).unwrap();
        assert_eq!(map.width(), 3);
        assert_eq!(map.packed_index(0), Some(0));
        assert_eq!(map.packed_index(65), Some(1));
        assert_eq!(map.packed_index(35855), Some(2));
        assert_eq!(map.packed_index(64), None);
        assert_eq!(map.packed_index(35856), None);
        let single = PolicyMap::from_metadata(Some("65"), 1).unwrap();
        assert_eq!(single.width(), 1);
        assert_eq!(single.packed_index(65), Some(0));
    }

    // メタデータの契約: 鍵の欠落と空の表を別々のエラーで拒む。
    #[test]
    fn rejects_missing_and_empty_metadata() {
        assert!(matches!(
            PolicyMap::from_metadata(None, 1),
            Err(NnError::MissingPolicyLabels)
        ));
        for width in [0, 1] {
            assert!(matches!(
                PolicyMap::from_metadata(Some(""), width),
                Err(NnError::EmptyPolicyLabels)
            ));
        }
    }

    // メタデータの契約: 空の要素、空白、非ASCII、負数、非10進表記、桁あふれを解析エラーにする。
    #[test]
    fn rejects_invalid_integers_without_losing_the_parse_error() {
        for (token, kind) in [
            ("", IntErrorKind::Empty),
            (" 1", IntErrorKind::InvalidDigit),
            ("1\n", IntErrorKind::InvalidDigit),
            ("１", IntErrorKind::InvalidDigit),
            ("-1", IntErrorKind::InvalidDigit),
            ("0x10", IntErrorKind::InvalidDigit),
            ("1.0", IntErrorKind::InvalidDigit),
            ("18446744073709551616", IntErrorKind::PosOverflow),
        ] {
            let metadata = format!("0,{token}");
            let error = PolicyMap::from_metadata(Some(&metadata), 2).unwrap_err();
            assert!(error.source().unwrap().is::<std::num::ParseIntError>());
            assert!(
                matches!(error, NnError::InvalidPolicyLabel { index: 1, value, source }
                if value == token && *source.kind() == kind)
            );
        }
        for (metadata, expected) in [(",1", 0), ("0,,2", 1)] {
            assert!(matches!(PolicyMap::from_metadata(Some(metadata), 3),
                Err(NnError::InvalidPolicyLabel { index, .. }) if index == expected));
        }
    }

    // メタデータの契約: 重複と降順を許さず、違反した添字と前後のラベルを返す。
    #[test]
    fn rejects_duplicate_and_descending_labels() {
        for (metadata, expected) in [("0,65,65", 65), ("0,65,64", 64)] {
            assert!(matches!(PolicyMap::from_metadata(Some(metadata), 3),
                Err(NnError::PolicyLabelsNotAscending { index: 2, previous: 65, label })
                    if label == expected));
        }
    }

    // ラベルの仕様: 249面×144升の範囲は0〜35,855であり、上端の次の値を拒む。
    #[test]
    fn rejects_labels_outside_the_full_policy() {
        for metadata in ["0,35856".to_owned(), format!("0,{}", usize::MAX)] {
            assert!(matches!(
                PolicyMap::from_metadata(Some(&metadata), 2),
                Err(NnError::PolicyLabelOutOfRange { index: 1, .. })
            ));
        }
    }

    // メタデータの契約: 表の要素数と出力幅の過不足を拒み、幅0では構築できない。
    #[test]
    fn rejects_counts_that_differ_from_the_output_width() {
        for expected in [0, 1, 3, usize::MAX] {
            assert!(matches!(PolicyMap::from_metadata(Some("0,65"), expected),
                Err(NnError::PolicyLabelCount { expected: actual_width, actual: 2 })
                    if actual_width == expected));
        }
    }
}
