//! MNSDの駒コードから局面を復元し、推論と共通の利きの面を返す。

use minase::nn::control::{CONTROL_SIZE, encode_control};
use minase::training::records;
use minase_core::{BOARD_SQUARE_COUNT, Color};

use crate::Error;

/// 平坦な駒コード列と手番列から、`[局面][64面][144升]`の疑似利きを返す。
///
/// 駒コードは[`records::decode_position`]に従い、手番は先手0、後手1とする。
/// 先獅子の升を含め、疑似利きに不要な状態は復元しない。
///
/// # Errors
/// 配列長の不一致、不正な手番・駒コード、成り権の保留、および出力の確保失敗を返す。
pub fn control_planes(boards: &[u8], sides: &[u8]) -> Result<Vec<f32>, Error> {
    let (boards, remainder) = boards.as_chunks::<BOARD_SQUARE_COUNT>();
    if !remainder.is_empty() || boards.len() != sides.len() {
        return Err(Error::InvalidControlShape);
    }
    let size = sides
        .len()
        .checked_mul(CONTROL_SIZE)
        .ok_or(Error::InvalidControlShape)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(size)
        .map_err(Error::AllocateControl)?;
    output.resize(size, 0.0);
    for (index, ((board, &side), out)) in boards
        .iter()
        .zip(sides)
        .zip(output.as_chunks_mut::<CONTROL_SIZE>().0)
        .enumerate()
    {
        let side = match side {
            0 => Color::Black,
            1 => Color::White,
            side => return Err(records::Error::InvalidSideToMove(side).into()),
        };
        if let Some(square) = board.iter().position(|code| code & 128 != 0) {
            return Err(Error::PromotionDeferred {
                position: index,
                square,
            });
        }
        let position = records::decode_position(board, side, None, false)?;
        encode_control(&position, out);
    }
    Ok(output)
}
