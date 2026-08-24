//! 中将棋エンジン本体の実行ファイル。プロトコルと規則を指定して起動する。

use std::error::Error;
use std::io::{self, BufRead};
use std::process;
use std::sync::mpsc;
use std::thread;

use clap::{Parser, ValueEnum};
use minase::RuleCode;
use minase::core::rules::parse_rule_set;
use minase::protocol::{CecpProtocol, Engine, UsiProtocol};

/// グローバルアロケータ。benchの実測（docs/plans/search.md 実施状況）に基づきmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 中将棋エンジンのプロトコル入口。
#[derive(Parser)]
#[command(name = "minase")]
struct Arguments {
    /// 使用する通信プロトコル。
    #[arg(long, value_enum, required = true)]
    protocol: ProtocolKind,
    /// 採用するローカルルールコード列。
    #[arg(long, required = true, value_parser = parse_rule_set_argument)]
    rules: RuleSetArgument,
}

/// 解析済みの`--rules`引数。
#[derive(Clone)]
struct RuleSetArgument(Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input).map(RuleSetArgument)
}

/// 対応する通信プロトコル。
#[derive(Clone, Copy, ValueEnum)]
enum ProtocolKind {
    /// lishogi系拡張を含むUSI。
    Usi,
    /// CECP(XBoard)。
    Cecp,
}

/// エラーを標準エラーへ報告して終了コード1で終わる入口。
fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 指定プロトコルでセッションを実行する。
///
/// USIとCECPは探索中もコマンドを受けるため、標準入力を
/// reader threadで読んでチャネル経由で処理する。
fn run() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let mut engine = Engine::new(arguments.rules.0)
        .map_err(|reason| format!("invalid --rules value: {reason}"))?;

    match arguments.protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::new(&engine);
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                let mut line = String::new();
                loop {
                    line.clear();
                    match input.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if sender.send(Ok(line.trim_end().to_owned())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
            });
            let stdout = io::stdout();
            protocol.run_channel(&mut engine, &receiver, &mut stdout.lock())?;
            Ok(())
        }
        ProtocolKind::Cecp => {
            let mut protocol = CecpProtocol::new(&engine);
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                let mut line = String::new();
                loop {
                    line.clear();
                    match input.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if sender.send(Ok(line.trim_end().to_owned())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
            });
            let stdout = io::stdout();
            protocol.run_channel(&mut engine, &receiver, &mut stdout.lock())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use minase::Rules;

    use super::*;

    #[test]
    fn cli_requires_explicit_protocol_and_rules() {
        // PL「モジュールと名称」・完了条件: --protocol usi|cecpと--rulesの明示指定を必須とし、
        // 既定値と自動判別を設けない（D6-CLI-01）。
        assert!(Arguments::try_parse_from(["minase"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--protocol", "usi"]).is_err());
        assert!(Arguments::try_parse_from(["minase", "--rules", "R1"]).is_err());
        // --protocolの値はusiとcecpの2値のみ。
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "xboard", "--rules", "R1"]).is_err()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", "R1"]).is_ok()
        );
        assert!(
            Arguments::try_parse_from(["minase", "--protocol", "cecp", "--rules", "R1"]).is_ok()
        );
    }

    #[test]
    fn rules_argument_shares_the_wire_value_grammar() {
        // PL「規則オプション」（同じ値文法を--rulesにも適用、解析は共通関数parse_rule_set）・
        // R33第5・6項（engine-default=L0+P0+R1+E0、
        // lishogi=L1+L2+P0+P3+R1+E1+E3、大小非区別・併記拒否）
        // （D6-CLI-02〜04、D6-CLI-05のminase側接続確認）。
        let parse = |value: &str| {
            Arguments::try_parse_from(["minase", "--protocol", "usi", "--rules", value])
                .map(|arguments| arguments.rules.0)
        };

        assert_eq!(
            parse("engine-default").unwrap(),
            parse_rule_set("L0,P0,R1,E0").unwrap()
        );
        assert_eq!(
            parse("Engine-Default").unwrap(),
            parse("engine-default").unwrap()
        );
        assert_eq!(
            parse("lishogi").unwrap(),
            parse("L1,L2,P0,P3,R1,E1,E3").unwrap()
        );
        assert_eq!(parse("LISHOGI").unwrap(), parse("lishogi").unwrap());
        // 値は大文字小文字を区別せず、コード列は同じ規則集合へ解決される。
        assert_eq!(
            Rules::from_codes(&parse("p0,r1,l1,e0").unwrap()).unwrap(),
            Rules::from_codes(&parse("L1,P0,R1,E0").unwrap()).unwrap()
        );

        assert!(parse("XX9").is_err());
        // R33第5項: MinaseはR0を選択可能な規則コードとして提供しない。
        assert!(parse("R0").is_err());
        // PL 2026-08-11追記: standardという名前は受理しない。
        assert!(parse("standard").is_err());
        assert!(parse("lishogi,engine-default").is_err());
        // プリセットとコードの併記には専用エラーを返す（PLフェーズ4追補）。
        let error = parse("lishogi,P1").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("preset 'lishogi' must be specified alone")
        );

        // 4群のいずれかを欠く列は値文法としては解析できるが、エンジン構築時に拒否される
        // ため、不正値での起動成功はあり得ない（D6-CLI-02境界）。
        assert!(Engine::new(parse("L1,P0,E1,E0").unwrap()).is_err());
    }
}
