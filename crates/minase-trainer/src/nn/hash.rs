use sha2::{Digest, Sha256};

/// バイト列のSHA-256を計算する。
/// SSEとAVXの混在によるペナルティを避け、呼び出し境界で`vzeroupper`を挿入させるため、インライン展開しない。
#[inline(never)]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// 分割した入力のSHA-256を計算する。既定値は入力が空の状態。
/// 更新と確定の呼び出し境界を保ち、SSEとAVXの混在によるペナルティを避ける。
#[derive(Default)]
pub struct Sha256Hasher(Sha256);

impl Sha256Hasher {
    /// 入力を追加する。SSEとAVXの混在を避ける`vzeroupper`が呼び出し境界に入るよう、インライン展開しない。
    #[inline(never)]
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// 検査和を確定する。SSEとAVXの混在を避ける`vzeroupper`が呼び出し境界に入るよう、インライン展開しない。
    #[inline(never)]
    pub fn finalize(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

/// 各バイトを小文字の16進数2桁で表す。
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 期待値はPython標準ライブラリのhashlib.sha256でも照合した。
    const VECTORS: &[(&[u8], &str)] = &[
        (b"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1",
        ),
    ];

    #[test]
    fn sha256_matches_reference_checksums() {
        for &(bytes, expected) in VECTORS {
            assert_eq!(hex(&sha256(bytes)), expected);
        }
    }

    #[test]
    fn streamed_sha256_preserves_checksums_across_chunk_boundaries() {
        for &(bytes, expected) in VECTORS {
            for size in [1, 7, 55, 56, 63, 64, 65, 128] {
                let mut hash = Sha256Hasher::default();
                hash.update(b"");
                for chunk in bytes.chunks(size) {
                    hash.update(chunk);
                }
                hash.update(b"");
                assert_eq!(hex(&hash.finalize()), expected, "chunk size: {size}");
            }
        }
    }
}
