use super::{AdamW, Error, OptimizerConfig, Parameter, Parameters, Result, Scale};
use crate::nn::{
    AdamWGroup,
    checkpoint::{Reader, Writer},
};
use minase_core::rules::{Rules, parse_rule_set};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 学習の軌跡を定める設定。総superbatch数、時間上限、スレッド数は含めない。
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// 第1層の幅。
    pub width: usize,
    /// 教師尺度と出力の除数。
    pub scale: Scale,
    /// 教師生成と照合する規則セット。
    pub rules: Rules,
    /// 探索値と対局結果を混合する比率。
    pub lambda: f64,
    /// 初期化のシード。
    pub seed: u64,
    /// 最適化器の基準係数。
    pub optimizer: OptimizerConfig,
}
impl Settings {
    /// 教師混合比、規則の組合せ、および最適化器の係数を検査する。
    pub fn validate(&self) -> Result<()> {
        super::model::check_width(self.width)?;
        if !self.lambda.is_finite() || !(0.0..=1.0).contains(&self.lambda) {
            return Err(Error::Invalid("lambda"));
        }
        let codes = Vec::from(self.rules);
        Rules::from_codes(&codes).map_err(|_| Error::Invalid("rules"))?;
        self.optimizer.validate()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    architecture: String,
    feature_set: String,
    rules: String,
    progress: crate::nnue::train::Progress,
}

/// HalfKP_vm Simpleの全パラメータ、最適化器の状態、および再開条件。
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// 自由パラメータ。
    pub parameters: Parameters,
    /// 両モーメントと完了した更新回数。
    pub optimizer: AdamW,
    /// 再開時に照合する設定。
    pub settings: Settings,
    /// 確定した学習進捗と入力の照合情報。
    pub progress: crate::nnue::train::Progress,
}
impl Checkpoint {
    fn validate(&self) -> Result<()> {
        if !self.progress.elapsed_seconds.is_finite()
            || self.progress.elapsed_seconds < 0.0
            || self.progress.completed_steps != self.optimizer.step
        {
            return Err(Error::Invalid("checkpoint progress"));
        }
        self.parameters.validate()?;
        self.optimizer.validate()?;
        self.settings.validate()?;
        if self.optimizer.config != self.settings.optimizer
            || self.parameters.width() != self.settings.width
            || self.optimizer.m.width() != self.settings.width
        {
            return Err(Error::SettingsMismatch);
        }
        Ok(())
    }
    /// 名前、型、形状を固定した無圧縮NPZへ保存する。
    /// weights、m、vの各接頭辞でParameterの全配列をf32の行優先で記録する。
    /// メタデータは構造、特徴集合、規則を持ち、残りの設定と更新数は数値配列に保存する。
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let s = &self.settings;
        let mut w = Writer::with_metadata(
            "simple",
            &Metadata {
                architecture: "Simple".into(),
                feature_set: super::SPEC.arch_name().into(),
                rules: s.rules.to_string(),
                progress: self.progress.clone(),
            },
        )?;
        w.scalar("width", s.width as u64);
        w.scalar("teacher_scale", s.scale.teacher());
        w.scalar("fv_scale", s.scale.divisor() as u64);
        w.scalar("lambda", s.lambda);
        w.scalar("seed", s.seed);
        w.scalar("step", self.optimizer.step);
        w.values(
            "optimizer_groups",
            &[2, 2],
            &[
                s.optimizer.ft.lr,
                s.optimizer.ft.weight_decay,
                s.optimizer.head.lr,
                s.optimizer.head.weight_decay,
            ],
        );
        for (prefix, params) in [
            ("weights", &self.parameters),
            ("m", &self.optimizer.m),
            ("v", &self.optimizer.v),
        ] {
            for p in Parameter::ALL {
                w.values(
                    &format!("{prefix}.{}", p.name()),
                    &p.dimensions(s.width),
                    &params[p],
                );
            }
        }
        Ok(w.encode()?)
    }
    /// 配列の集合、型、形状、数値、および設定の整合性を厳密に検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut r, meta): (_, Metadata) = Reader::with_metadata(bytes, "simple")?;
        let width = usize::try_from(r.scalar::<u64>("width")?)
            .map_err(|_| Error::Invalid("checkpoint width"))?;
        super::model::check_width(width)?;
        if meta.architecture != "Simple" || meta.feature_set != super::SPEC.arch_name() {
            return Err(Error::Invalid("checkpoint architecture, features or width"));
        }
        let scale = Scale::new(r.scalar("teacher_scale")?)?;
        if r.scalar::<u64>("fv_scale")? != scale.divisor() as u64 {
            return Err(Error::Invalid("checkpoint output divisor"));
        }
        let rules = Rules::from_codes(
            &parse_rule_set(&meta.rules).map_err(|_| Error::Invalid("checkpoint rules"))?,
        )
        .map_err(|_| Error::Invalid("checkpoint rules"))?;
        let lambda = r.scalar("lambda")?;
        let seed = r.scalar("seed")?;
        let step = r.scalar("step")?;
        let groups = r.values::<f64>("optimizer_groups", &[2, 2])?;
        let config = OptimizerConfig {
            ft: AdamWGroup {
                lr: groups[0],
                weight_decay: groups[1],
            },
            head: AdamWGroup {
                lr: groups[2],
                weight_decay: groups[3],
            },
        };
        let mut read = |prefix| -> Result<Parameters> {
            let mut params = Parameters::zeros(width)?;
            for p in Parameter::ALL {
                params[p].copy_from_slice(
                    &r.values::<f32>(&format!("{prefix}.{}", p.name()), &p.dimensions(width))?,
                );
            }
            Ok(params)
        };
        let parameters = read("weights")?;
        let m = read("m")?;
        let v = read("v")?;
        r.finish()?;
        let checkpoint = Self {
            progress: meta.progress,
            parameters,
            optimizer: AdamW { m, v, step, config },
            settings: Settings {
                width,
                scale,
                rules,
                lambda,
                seed,
                optimizer: config,
            },
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    /// 再開条件だけを比較する。実行予算はこの型の入力に含まれない。
    pub fn check_settings(&self, current: &Settings) -> Result<()> {
        current.validate()?;
        if self.settings != *current {
            return Err(Error::SettingsMismatch);
        }
        Ok(())
    }
    /// 一時ファイルからの改名で原子的に書き込む。
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        Ok(crate::nn::atomic_write(path.as_ref(), &self.encode()?)?)
    }
    /// ファイルを読み、decodeと同じ検査を行う。
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::decode(&std::fs::read(path)?)
    }
}
