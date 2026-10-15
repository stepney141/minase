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
