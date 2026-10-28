//! MNSD版2の項目フラグだけを読み、手順を再生せずに局面数を数える。
//!
//! 選別はフラグの下位3ビットが0の項目であり、検証の分割は対局ごとに決まる。
//! 候補は長さだけを読んで飛ばす。着手と候補の合法性などは後続の再生で検査する。

use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

use minase::training::records::{self, Origin, Reader, Section, TeacherKind};

use crate::{Error, FileInfo, is_validation};

fn input<const N: usize>(source: &mut impl Read, section: Section) -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    source.read_exact(&mut bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Format(records::Error::Truncated(section))
        } else {
            Error::Io(error)
        }
    })?;
    Ok(bytes)
}

fn skip_candidates(source: &mut impl BufRead, mut length: usize) -> Result<(), Error> {
    while length != 0 {
        let available = source.fill_buf().map_err(Error::Io)?;
        if available.is_empty() {
            return Err(records::Error::Truncated(Section::Candidate).into());
        }
        let consumed = length.min(available.len());
        source.consume(consumed);
        length -= consumed;
    }
    Ok(())
}

/// ヘッダと全対局番号を読み、選別後の訓練局面数と検証局面数を返す。
///
/// 盤面の復元と合法手生成を行わず、保持するのは対局番号だけである。
/// 完全な形式検査は`read_selected`が行うので、計数だけでは学習へ進めない。
///
/// # Errors
/// 入出力、ヘッダ、由来、番号順、手数、項目フラグ、件数、末尾の誤りを返す。
pub fn scan_file(path: &Path) -> Result<FileInfo, Error> {
    let reader = Reader::new(BufReader::new(File::open(path).map_err(Error::Io)?))?;
    let mut info = FileInfo::new(reader.header().clone());
    let mut source = reader.into_inner();
    let mut entries = 0_u64;
    for _ in 0..info.header.game_count() {
        let number = u32::from_le_bytes(input(&mut source, Section::GameHeader)?);
        if info
            .game_numbers
            .last()
            .is_some_and(|&previous| previous >= number)
        {
            return Err(records::Error::GameNumberNotIncreasing.into());
        }
        let [origin, _, _, reserved] = input(&mut source, Section::GameHeader)?;
        if reserved != 0 {
            return Err(records::Error::NonzeroReservedBytes.into());
        }
        info.add_game(number, Origin::from_code(origin)?)?;
        let ply = u16::from_le_bytes(input(&mut source, Section::GameHeader)?);
        let count = u32::from_le_bytes(input(&mut source, Section::GameHeader)?);
        if count == 0 {
            return Err(records::Error::EmptyGame.into());
        }
        if u64::from(ply) + u64::from(count) - 1 > u64::from(u16::MAX) {
            return Err(records::Error::PlyOverflow.into());
        }
        entries = entries
            .checked_add(u64::from(count))
            .ok_or(records::Error::CountOverflow)?;
        if entries > info.header.entry_count() {
            return Err(records::Error::EntryCountMismatch.into());
        }
        // 盤の144バイトと、手番、先獅子、麒麟成りの各1バイトを読む。
        input::<147>(&mut source, Section::GameHeader)?;
        let validation = is_validation(info.header.seed(), number);
        for _ in 0..count {
            let entry = input::<7>(&mut source, Section::Entry)?;
            if info.header.teacher_kind() == TeacherKind::Mcts {
                input::<4>(&mut source, Section::Entry)?;
            }
            let flags = entry[4];
            if flags & 0xf0 != 0 {
                return Err(records::Error::InvalidEntryFlags(flags).into());
            }
            if flags & 4 != 0 && entry[2..4] != [0, 0] {
                return Err(records::Error::NonzeroAbsentValue.into());
            }
            if flags & 7 == 0 {
                if validation {
                    info.validation_count += 1;
                } else {
                    info.training_count += 1;
                }
            }
            let candidates = u16::from_le_bytes([entry[5], entry[6]]);
            skip_candidates(&mut source, usize::from(candidates) * 4)?;
        }
    }
    if entries != info.header.entry_count() {
        return Err(records::Error::EntryCountMismatch.into());
    }
    if !source.fill_buf().map_err(Error::Io)?.is_empty() {
        return Err(records::Error::TrailingBytes.into());
    }
    Ok(info)
}
