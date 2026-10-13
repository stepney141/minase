use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
};

use memmap2::{Mmap, MmapOptions};
use minase::training::{
    provenance::{self, ResultOrigin, SearchCondition, StartOrigin},
    records::{self, HEADER_LEN, Header, Outcome, RECORD_LEN, Record},
};
use serde::{Deserialize, Serialize};

use super::{
    Error, Lookahead, Result, compute_lookahead, estimate_k, is_validation,
    validate_lambda_override,
};
use crate::nn::{hex, sha256};

/// 教師用Kを共有する分類。
///
/// 来歴の`teacher`の5項目と、`result_origin`、`start_origin`の計7項目で由来を区別する。
/// 5項目は、小文字へ正規化した40桁16進の`generation_commit`、MNSDヘッダの32バイトの
/// `network_checksum`、`nodes`、`rule_set`、および`search_condition`である。
/// `search_condition`は`"in-game"`または`"standalone"`、`result_origin`は`"selfplay"`または`"human"`、
/// `start_origin`は`"random"`または`"human-game"`である。
/// 先読み条件`lookahead`と実際に適用した混合比の上書き`lambda_override`も分類に含める。
/// `lambda_override`は自己対局の分類だけに適用し、人間対局では`None`とする。
/// 入力順に初めて現れた分類から0始まりで番号を付け、記録数0のファイルは分類を作らない。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TeacherClass {
    pub(crate) generation_commit: String,
    pub(crate) network_checksum: [u8; 32],
    pub(crate) nodes: u32,
    pub(crate) rule_set: String,
    pub(crate) search_condition: SearchCondition,
    pub(crate) result_origin: ResultOrigin,
    pub(crate) start_origin: StartOrigin,
    pub(crate) lookahead: Option<Lookahead>,
    pub(crate) lambda_override: Option<f64>,
}

impl TeacherClass {
    /// 生成に使った評価関数の検査和を返す。
    pub fn network_checksum(&self) -> &[u8; 32] {
        &self.network_checksum
    }

    /// 教師探索のノード上限を返す。
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// 先読みの条件を返す。省略時は元の探索値を使う。
    pub fn lookahead(&self) -> Option<Lookahead> {
        self.lookahead
    }
}

/// 通し番号に対応するファイル番号とバイトオフセット。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordLocation {
    /// 入力したファイル列の0始まりの番号。
    pub file: usize,
    /// ヘッダを含むファイル先頭からのバイト数。
    pub byte_offset: usize,
}

struct MappedFile {
    path: PathBuf,
    map: Mmap,
    header: Header,
    class: u32,
    scores: Vec<f32>,
    outcomes: Vec<u8>,
    game_ids: Option<HashMap<u32, String>>,
}

/// 学習側が読む来歴JSON。
///
/// `format`は`"minase-provenance"`、`version`は1、`mnsd_sha256`はMNSD全体のSHA-256の64桁16進である。
/// `teacher`は`generation_commit`（40桁16進）、`network_checksum`（64桁16進）、`nodes`（u32）、
/// `rule_set`、`search_condition`の5欄を持つ。
/// `rule_set`はUTF-8で1〜32バイト、NULを含まず、`search_condition`は`"in-game"`または`"standalone"`である。
/// `result_origin`、`start_origin`、有限な0以上1以下の`lambda`、および`games`も必須である。
/// 由来の文字列は[`TeacherClass`]の定義に従う。
///
/// `games`は乱数開始の自己対局では明示的な`null`、それ以外では`{"game": 対局番号, "id": "棋譜ID"}`の配列であり、省略を拒否する。
/// 対局番号は1以上で重複せず、棋譜IDは空でない。
/// `start_origin = "human-game"`では各要素に非負整数の開始手数`ply`を必須とする。
/// それ以外の由来では`ply`の有無と値を検査しない。
/// 学習側は未知の追加欄と任意の有効なλを受け付ける。
/// 生成側の[`Provenance`](minase::training::provenance::Provenance)がλを由来ごとの値
/// （自己対局0.75、実戦0.0）に固定する契約とは独立である。
#[derive(Deserialize)]
struct InputProvenance {
    format: String,
    version: u32,
    mnsd_sha256: String,
    teacher: InputTeacher,
    result_origin: ResultOrigin,
    start_origin: StartOrigin,
    lambda: f64,
    #[serde(deserialize_with = "deserialize_games")]
    games: Option<Vec<InputGame>>,
}

#[derive(Deserialize)]
struct InputTeacher {
    generation_commit: String,
    network_checksum: String,
    nodes: u32,
    rule_set: String,
    search_condition: SearchCondition,
}

#[derive(Deserialize)]
struct InputGame {
    game: u32,
    id: String,
    ply: Option<serde_json::Value>,
}

fn deserialize_games<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<InputGame>>, D::Error> {
    Option::<Vec<InputGame>>::deserialize(deserializer)
}

impl InputProvenance {
    fn read(path: &Path) -> Result<Self> {
        let value: Self =
            serde_json::from_reader(File::open(path)?).map_err(provenance::Error::Json)?;
        let invalid_field = |name| Error::Provenance(provenance::Error::InvalidField(name));
        if value.format != "minase-provenance" || value.version != 1 {
            return Err(invalid_field("format/version"));
        }
        for (name, text, length) in [
            ("mnsd_sha256", &value.mnsd_sha256, 64),
            ("generation_commit", &value.teacher.generation_commit, 40),
            ("network_checksum", &value.teacher.network_checksum, 64),
        ] {
            if text.len() != length || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid_field(name));
            }
        }
        let rule = &value.teacher.rule_set;
        if rule.is_empty() || rule.len() > 32 || rule.contains('\0') {
            return Err(invalid_field("rule_set"));
        }
        validate_lambda_override(Some(value.lambda))?;
        let random_selfplay = value.result_origin == ResultOrigin::Selfplay
            && value.start_origin == StartOrigin::Random;
        if random_selfplay == value.games.is_some() {
            return Err(invalid_field("games"));
        }
        if let Some(games) = &value.games {
            let mut numbers = HashSet::new();
            for game in games {
                if game.game == 0 || game.id.is_empty() || !numbers.insert(game.game) {
                    return Err(invalid_field("game mapping"));
                }
                if value.start_origin == StartOrigin::HumanGame
                    && !game.ply.as_ref().is_some_and(|ply| ply.as_u64().is_some())
                {
                    return Err(invalid_field("human-game start ply"));
                }
            }
        }
        Ok(value)
    }

    fn validation_games(&self) -> Option<HashMap<u32, bool>> {
        self.games.as_ref().map(|games| {
            games
                .iter()
                .map(|game| {
                    let digest = sha256(game.id.as_bytes());
                    let prefix = u64::from_le_bytes(digest[..8].try_into().unwrap());
                    (game.game, prefix.is_multiple_of(20))
                })
                .collect()
        })
    }
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::Dataset {
        path: path.to_owned(),
        reason,
    }
}

impl MappedFile {
    unsafe fn open(path: &Path) -> Result<(Self, InputProvenance)> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        if length < HEADER_LEN as u64 {
            return Err(invalid(path, "MNSD header is shorter than 136 bytes"));
        }
        // SAFETY: 呼び手は、この呼び出し中と写像の生存中に元ファイルを変更・切り詰めしない。
        let map = unsafe { MmapOptions::new().map(&file)? };
        let header = Header::decode(&map[..HEADER_LEN])?;
        let expected = header
            .record_count()
            .checked_mul(RECORD_LEN as u64)
            .and_then(|bytes| bytes.checked_add(HEADER_LEN as u64))
            .ok_or(records::Error::FileLengthOverflow)?;
        if length != expected {
            return Err(records::Error::FileLengthMismatch {
                expected,
                actual: length,
            }
            .into());
        }

        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".provenance.json");
        let provenance = InputProvenance::read(Path::new(&sidecar))?;
        let checksum = hex(&sha256(&map));
        if !provenance.mnsd_sha256.eq_ignore_ascii_case(&checksum) {
            return Err(invalid(path, "provenance MNSD SHA-256 mismatch"));
        }
        let network = header
            .network_checksum()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let teacher = &provenance.teacher;
        if !teacher.network_checksum.eq_ignore_ascii_case(&network)
            || !teacher
                .generation_commit
                .eq_ignore_ascii_case(header.generation_commit())
            || teacher.nodes != header.teacher_nodes()
            || teacher.rule_set != header.rule_set()
        {
            return Err(invalid(
                path,
                "provenance teacher does not match the MNSD header",
            ));
        }
        Ok((
            Self {
                path: path.to_owned(),
                map,
                header,
                class: 0,
                scores: Vec::new(),
                outcomes: Vec::new(),
                game_ids: provenance
                    .games
                    .as_ref()
                    .map(|games| games.iter().map(|g| (g.game, g.id.clone())).collect()),
            },
            provenance,
        ))
    }

    fn record(&self, local: usize) -> Result<Record> {
        let start = HEADER_LEN + local * RECORD_LEN;
        Ok(Record::decode(&self.map[start..start + RECORD_LEN])?)
    }
}

/// 複数のMNSDをmmapで保持し、同じ通し番号で前処理とバッチを参照する。
///
/// 元ファイルはこの型の生存中に変更・切り詰めしないこと。盤面全体は保持せず、教師探索値
/// （f32）と結果（u8）、訓練・検証の通し番号を保持する。
pub struct Dataset {
    files: Vec<MappedFile>,
    offsets: Vec<u64>,
    training_indices: Vec<u64>,
    validation_indices: Vec<u64>,
    classes: Vec<TeacherClass>,
    lambdas: Vec<f64>,
}

impl Dataset {
    /// ファイル長と来歴を検証し、全記録を復号して分割と教師探索値を作る。
    /// 先読みを指定した場合はファイルごとに対局・手数の順序も検証する。
    /// λは来歴から読み、`lambda_override`を指定すると自己対局の分類だけ上書きする。
    /// `paths`は1つ以上必要で、入力順に記録を連結して0始まりの通し番号を付ける。
    /// 各MNSDには、正規化したパスの末尾へ`.provenance.json`を付けた来歴ファイルが必要である。
    /// ファイル長が136バイトと記録数×160バイトの和に一致することを検査する。
    /// MNSD全体のSHA-256と来歴の`mnsd_sha256`、およびヘッダと来歴の`teacher`の
    /// `network_checksum`、`generation_commit`、`nodes`、`rule_set`の一致も要求する。
    /// 16進表現の比較では大文字と小文字を区別しない。
    /// 正規化したパスの重複、MNSD内容のSHA-256の重複、および乱数開始の自己対局
    /// （`result_origin = selfplay`かつ`start_origin = random`）のファイル間の生成シードの重複を拒否する。
    ///
    /// `lambda_override`は有限な0以上1以下とし、人間対局の分類では来歴のλを使う。
    /// 同じ[`TeacherClass`]に異なる実効λを持つ入力は拒否する。
    /// 来歴の`games`が`null`なら[`is_validation`]にヘッダのシードと対局番号を渡して検証集合を決める。
    /// `games`が配列なら、対応する棋譜IDのUTF-8表現のSHA-256の先頭8バイトを
    /// リトルエンディアンu64として読み、20で割った余りが0の対局を検証集合とする。
    /// 同じ棋譜とその棋譜から始めた自己対局は、ファイルが異なっても同じ集合に属する。
    /// 記録に現れる対局番号が`games`にない場合は拒否する。
    /// 先読みを指定した場合は、ファイルごとに[`compute_lookahead`]で計算した値をf32で保持し、MNSDへは書き戻さない。
    ///
    /// MNSDをmemmap2で写すため、呼び手に元ファイルの不変性を要求する。
    /// 理由は`docs/adr/allow-unsafe-only-in-the-trainer-crate.md`にある。
    ///
    /// # Safety
    ///
    /// 呼び手は、この呼び出し中と返された`Dataset`の生存中に、渡したファイルを外部プロセスも含めて変更も切り詰めもしないことを保証しなければならない。
    pub unsafe fn open(
        paths: &[PathBuf],
        lookahead: Option<Lookahead>,
        lambda_override: Option<f64>,
    ) -> Result<Self> {
        validate_lambda_override(lambda_override)?;
        if paths.is_empty() {
            return Err(invalid(Path::new(""), "at least one MNSD file is required"));
        }
        let mut dataset = Self {
            files: Vec::new(),
            offsets: vec![0],
            training_indices: Vec::new(),
            validation_indices: Vec::new(),
            classes: Vec::new(),
            lambdas: Vec::new(),
        };
        let mut seeds = HashSet::new();
        let mut unique_paths = HashSet::new();
        let mut checksums = HashSet::new();
        for path in paths {
            let path = path.canonicalize()?;
            if !unique_paths.insert(path.clone()) {
                return Err(invalid(
                    &path,
                    "the same MNSD file was specified more than once",
                ));
            }
            // SAFETY: 元ファイルの不変性はDataset::openの呼び手が保証する。
            let (mut file, provenance) = unsafe { MappedFile::open(&path)? };
            if !checksums.insert(provenance.mnsd_sha256.to_ascii_lowercase()) {
                return Err(invalid(
                    &path,
                    "the same MNSD file content was specified more than once",
                ));
            }
            if provenance.result_origin == ResultOrigin::Selfplay
                && provenance.start_origin == StartOrigin::Random
                && !seeds.insert(file.header.seed())
            {
                return Err(invalid(&path, "random-selfplay file seeds must be unique"));
            }
            let start = dataset.record_count();
            let end = start
                .checked_add(file.header.record_count())
                .filter(|&count| count <= i64::MAX as u64)
                .ok_or_else(|| invalid(&path, "combined record count exceeds i64"))?;
            let count = usize::try_from(file.header.record_count())
                .map_err(|_| invalid(&path, "record count exceeds address space"))?;
            let applied_override = if provenance.result_origin == ResultOrigin::Selfplay {
                lambda_override
            } else {
                None
            };
            let mix = match applied_override {
                Some(value) => value,
                None => provenance.lambda,
            };
            if count > 0 {
                let class = TeacherClass {
                    generation_commit: provenance.teacher.generation_commit.to_ascii_lowercase(),
                    network_checksum: *file.header.network_checksum(),
                    nodes: provenance.teacher.nodes,
                    rule_set: provenance.teacher.rule_set.clone(),
                    search_condition: provenance.teacher.search_condition,
                    result_origin: provenance.result_origin,
                    start_origin: provenance.start_origin,
                    lookahead,
                    lambda_override: applied_override,
                };
                let class_index = match dataset.classes.iter().position(|item| item == &class) {
                    Some(index) => {
                        if dataset.lambdas[index] != mix {
                            return Err(invalid(
                                &path,
                                "the same teacher class has conflicting lambda values",
                            ));
                        }
                        index
                    }
                    None => {
                        dataset.classes.push(class);
                        dataset.lambdas.push(mix);
                        dataset.classes.len() - 1
                    }
                };
                file.class = u32::try_from(class_index)
                    .map_err(|_| invalid(&path, "teacher class count exceeds u32"))?;
            }
            let validation_games = provenance.validation_games();
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            let mut scores = Vec::with_capacity(count);
            file.outcomes.reserve(count);
            for local in 0..count {
                let record = file.record(local)?;
                games.push(record.game_number());
                plies.push(record.ply());
                scores.push(f64::from(record.score()));
                file.outcomes.push(outcome_code(record.outcome()));
                let validation = match &validation_games {
                    Some(mapping) => *mapping
                        .get(&record.game_number())
                        .ok_or_else(|| invalid(&path, "missing game ID mapping"))?,
                    None => is_validation(file.header.seed(), record.game_number()),
                };
                let indices = if validation {
                    &mut dataset.validation_indices
                } else {
                    &mut dataset.training_indices
                };
                indices.push(start + local as u64);
            }
            let scores = match lookahead {
                Some(options) => compute_lookahead(&games, &plies, &scores, options)?,
                None => scores,
            };
            file.scores = scores.into_iter().map(|value| value as f32).collect();
            dataset.files.push(file);
            dataset.offsets.push(end);
        }
        Ok(dataset)
    }

    /// 全ファイルを連結した記録数を返す。
    pub fn record_count(&self) -> u64 {
        self.offsets[self.offsets.len() - 1]
    }

    /// 入力順の絶対パスを返す。
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    /// 入力順の復号済みヘッダを返す。
    pub fn headers(&self) -> impl Iterator<Item = &Header> {
        self.files.iter().map(|file| &file.header)
    }

    /// 訓練局面の通し番号を元のファイル順で返す。
    pub fn training_indices(&self) -> &[u64] {
        &self.training_indices
    }

    /// 検証局面の通し番号を元のファイル順で返す。
    pub fn validation_indices(&self) -> &[u64] {
        &self.validation_indices
    }

    /// 分類番号順の教師情報を返す。
    pub fn teacher_classes(&self) -> &[TeacherClass] {
        &self.classes
    }

    /// 分類番号順の、上書き後の教師の混合比λを返す。
    pub fn teacher_lambdas(&self) -> &[f64] {
        &self.lambdas
    }

    fn locate(&self, index: u64) -> Result<(usize, usize)> {
        if index >= self.record_count() {
            return Err(Error::RecordIndex {
                index,
                count: self.record_count(),
            });
        }
        let file = self.offsets[1..].partition_point(|&end| end <= index);
        Ok((file, (index - self.offsets[file]) as usize))
    }

    /// 通し番号をファイル番号と記録先頭のバイトオフセットへ写す。
    pub fn location(&self, index: u64) -> Result<RecordLocation> {
        let (file, local) = self.locate(index)?;
        Ok(RecordLocation {
            file,
            byte_offset: HEADER_LEN + local * RECORD_LEN,
        })
    }

    /// 指定された記録を`Record::decode`で復号する。
    pub fn record(&self, index: u64) -> Result<Record> {
        let (file, local) = self.locate(index)?;
        self.files[file].record(local)
    }

    /// 先読み変換後の探索値を返す。先読みなしの場合は元の探索値。
    pub fn teacher_score(&self, index: u64) -> Result<f32> {
        let (file, local) = self.locate(index)?;
        Ok(self.files[file].scores[local])
    }

    /// 通し番号に対応する教師の分類番号を返す。
    pub fn teacher_class(&self, index: u64) -> Result<u32> {
        let (file, _) = self.locate(index)?;
        Ok(self.files[file].class)
    }

    /// 診断用に元の探索値から先読みを再計算し、f32へ丸めずに返す。
    pub(crate) fn diagnostic_teacher_scores(&self) -> Result<Vec<f64>> {
        let mut scores = Vec::with_capacity(self.record_count() as usize);
        for file in &self.files {
            let count = file.header.record_count() as usize;
            if count == 0 {
                continue;
            }
            let mut raw = Vec::with_capacity(count);
            let mut games = Vec::with_capacity(count);
            let mut plies = Vec::with_capacity(count);
            for local in 0..count {
                let record = file.record(local)?;
                raw.push(f64::from(record.score()));
                games.push(record.game_number());
                plies.push(record.ply());
            }
            match self.classes[file.class as usize].lookahead {
                Some(options) => scores.extend(compute_lookahead(&games, &plies, &raw, options)?),
                None => scores.extend(raw),
            }
        }
        Ok(scores)
    }

    /// 実戦棋譜IDは全入力で共通、ランダム自己対局はseedと対局番号で区別する。
    pub(crate) fn game_key(&self, index: u64) -> Result<GameKey<'_>> {
        let (file, local) = self.locate(index)?;
        let file = &self.files[file];
        let game = file.record(local)?.game_number();
        Ok(match &file.game_ids {
            Some(ids) => GameKey::Human(&ids[&game]),
            None => GameKey::Random(file.header.seed(), game),
        })
    }

    /// 分類ごとに訓練集合だけを使って教師用Kを推定する。
    /// 保持したf32の教師探索値をf64へ広げて校正し、標本は1分類ずつ確保する。
    /// λが0の分類は探索値を使わないのでNaNを返し、JSONの記録ではnullになる。
    /// 先読みを指定した場合は先読み値を使い、校正、学習、検証で同じ探索値を参照する。
    /// 推定の目標は訓練集合の終局結果だけであり、混合後の教師確率は使わない。
    /// λが0でない分類に訓練局面が1つもなければ[`Error::EmptyTeacherClass`]を返す。
    pub fn estimate_teacher_ks(&self) -> Result<Vec<f64>> {
        let mut values = Vec::with_capacity(self.classes.len());
        for class in 0..self.classes.len() {
            if self.lambdas[class] == 0.0 {
                values.push(f64::NAN);
                continue;
            }
            let mut scores = Vec::new();
            let mut outcomes = Vec::new();
            for (file_index, file) in self.files.iter().enumerate() {
                if file.class as usize != class {
                    continue;
                }
                let start = self.offsets[file_index];
                let end = self.offsets[file_index + 1];
                let first = self
                    .training_indices
                    .partition_point(|&index| index < start);
                let last = self.training_indices.partition_point(|&index| index < end);
                for &index in &self.training_indices[first..last] {
                    let local = (index - start) as usize;
                    scores.push(f64::from(file.scores[local]));
                    outcomes.push(file.outcomes[local]);
                }
            }
            if scores.is_empty() {
                return Err(Error::EmptyTeacherClass { class });
            }
            values.push(estimate_k(&scores, &outcomes)?);
        }
        Ok(values)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GameKey<'a> {
    Human(&'a str),
    Random(u64, u32),
}

pub(crate) fn outcome_code(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Loss => 0,
        Outcome::Draw => 1,
        Outcome::Win => 2,
    }
}
