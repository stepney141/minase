"""学習の系列の測定をCSVに収集し、6パネルの進捗図を作る。

リポジトリの根で次のように実行する。collectはstrength.csvとvalidation.csvを
上書きし、cycles.csvには触れない。plotは3つのCSVだけを読み、progress.pngを
上書きする。系列は、サイクル0の初期ネットから続く学習のサイクルの連なりである。

    uv run --frozen --project tools/train/progress python tools/train/progress/series_progress.py collect docs/training/<系列名>
    uv run --frozen --project tools/train/progress python tools/train/progress/series_progress.py plot docs/training/<系列名> [--reference docs/training/<参照の系列名>]

series.tomlの最上位の欄はstrengthとvalidationだけで、どちらも表の配列である。
ない欄は0件を表す。strengthの各表はcycle、summary、run_name、conditionを、
validationの各表はcycle、pathを、過不足なく持つ。
cycleは0以上の整数、summaryとpathは入力JSONへのパス、run_nameは集計内の測定名、
conditionは空でない測定条件の名前である。パスと名前は文字列とする。
パスはリポジトリの根からの相対パスであり、絶対パスと存在しないファイルは拒否する。
CLIはこのスクリプトの位置から根を求め、collect関数は根を引数で受け取る。

測定条件の名前は、MCTSの側の条件の組に付ける名前である。MCTSはモンテカルロ木探索
を指し、そのコミットと1手のシミュレーション回数は集計JSONにないため、名前の意味を
系列のページの「測定の条件」に記す。相手の種類・相手の制限・測定条件の名前が
同じ組の対局は、規則と手数の上限が一致しなければならない。

CSVはUTF-8で、1行目をヘッダーとする。
改行はLFとする。
strength.csvとvalidation.csvの列の集合と
順序は以下のとおり固定する。cycles.csvは必須列の順を問わず、追加できるのは
setting_で始まる列だけである。整数は0以上、実数はElo以外では有限とする。
損失は0以上、割合と確率質量は0以上1以下とし、百分率ではなく割合を保存する。
Eloの各列は無限大も許し、CSVではinfまたは-infと書く。件数、列名、型、欠測条件の
不整合はエラーにし、測っていない値を0で埋めない。

strength.csvは対局1件を1行とし、(cycle, opponent, opponent_limit, condition)の
昇順に並べる。ペアは同じ開始局面で先後を入れ替えた2局の組であり、破棄ペアは
どちらかの対局が手数の上限に達したために集計から外したペアである。

| 列 | 型 | 意味とJSONの欄 | 空欄にできる条件 |
| --- | --- | --- | --- |
| cycle | 整数 | series.tomlのサイクルの番号 | 不可 |
| opponent | 文字列 | baseline.kind。alphabetaまたはhachu | 不可 |
| opponent_limit | 文字列 | baseline.limitの唯一の欄を名前=値で表す。例はnodes=500、depth=3 | 不可 |
| condition | 文字列 | series.tomlの測定条件の名前 | 不可 |
| rules | 文字列 | rules_source。対局の規則 | 不可 |
| max_ply | 整数 | max_ply。対局の手数の上限 | 不可 |
| run_name | 文字列 | run_name。測定名 | 不可 |
| network_sha256 | 文字列 | network_sha256。対局に使ったネットのSHA-256 | 不可 |
| attempted_pairs | 整数 | attempted_pairs。試行したペア数 | 不可 |
| valid_pairs | 整数 | valid_pairs。有効なペア数 | 不可 |
| discarded_pairs | 整数 | discarded_pairs。破棄したペア数 | 不可 |
| pentanomial_0〜pentanomial_4 | 整数5列 | valid.pentanomial。添字kはペアの得点がk/2点の件数 | 不可 |
| score_rate | 実数 | valid.score_rate。有効ペアの得点率 | 有効ペアが0件 |
| elo | 実数 | valid.elo.elo。Eloの差 | 有効ペアが0件 |
| elo_lower | 実数 | valid.elo.lower。95%信頼区間の下端 | 有効ペアが0件 |
| elo_upper | 実数 | valid.elo.upper。95%信頼区間の上端 | 有効ペアが0件 |
| elo_without_move_forfeits | 実数 | without_move_forfeits.elo.elo。拒否着手または不正着手を含むペアを除いたElo | その度数の和が0件 |
| affected_pairs | 整数 | affected_pairs。拒否着手または不正着手を含むペアと破棄したペアを合わせた件数 | 不可 |
| affected_pair_fraction | 実数 | affected_pair_fraction。上記のペアの試行ペアに対する割合 | 不可 |
| elo_discarded_as_losses | 実数 | discarded_as_losses.elo.elo。破棄ペアを2敗としたElo | 不可 |
| elo_discarded_as_wins | 実数 | discarded_as_wins.elo.elo。破棄ペアを2勝としたElo | 不可 |

拒否着手はminaseの審判が認めたMCTSの着手をHaChuが受け付けなかったもの、不正着手は
規則に反する着手である。SHA-256はファイルの内容から求める識別子である。
JSONの無限大は文字列"-inf"と"+inf"である。度数の和が0件なら、対応するelo
オブジェクト全体とscore_rateがnullでなければならず、0件でなければ値を要する。
空欄はこのnullからだけ作る。試行ペアは1件以上とし、破棄ペアを2敗・2勝とした
集計には必ず値を要する。CSVには除外後の度数を保存しないので、除外後Eloの欠測条件は
collectが元のJSONで検査する。

validation.csvは診断1件を1行とし、cycleの昇順に並べる。価値の損失は対局結果との
二値交差エントロピー、一致率は人間が指した手とネットが選んだ手の一致の割合である。
どちらも1局の中で平均してから、局面を持つ対局の間で平均した値を保存する。

| 列 | 型 | 意味とJSONの欄 | 空欄にできる条件 |
| --- | --- | --- | --- |
| cycle | 整数 | series.tomlのサイクルの番号 | 不可 |
| data_sha256 | 文字列 | data.sha256。固定の検証用データのSHA-256 | 不可 |
| game_count | 整数 | diagnostics.game_count。対局数 | 不可 |
| position_count | 整数 | diagnostics.position_count。局面数 | 不可 |
| value_result | 実数 | diagnostics.value_result。価値の損失 | 不可 |
| policy_agreement | 実数 | diagnostics.policy_agreement。一致率 | 不可 |
| value_result_1_40、policy_agreement_1_40 | 実数2列 | 1〜40手目の帯の価値の損失と一致率 | 帯のposition_countが0 |
| value_result_41_100、policy_agreement_41_100 | 実数2列 | 41〜100手目の同じ2値 | 同上 |
| value_result_101_200、policy_agreement_101_200 | 実数2列 | 101〜200手目の同じ2値 | 同上 |
| value_result_201_up、policy_agreement_201_up | 実数2列 | 201手目以降の同じ2値 | 同上 |

bandsは(first_move, last_move)が(1, 40)、(41, 100)、(101, 200)、(201, null)の
4帯をちょうど1回ずつ持つ。空の帯は両方の値がnullでなければならず、非空の帯は
両方の値を要する。CSVには帯の件数を保存しないので、欠測条件はcollectがJSONで
検査し、plotは帯の2列がともに空欄か、ともに値を持つかを検査する。

cycles.csvはサイクル1つを1行とし、サイクルの番号は1から連続する。
設定の列だけを空欄にでき、空欄は記録にその値がないことを表す。

| 列 | 型 | 意味 |
| --- | --- | --- |
| cycle | 整数 | サイクルの番号 |
| training_positions | 整数 | そのサイクルで学習用に残した局面数。検証用に分ける前の値で、手数の上限に達した対局と、生成し直しで学習から除いた投了の対局の局面を含まず、生成し直した分を含む |
| cumulative_training_positions | 整数 | サイクル1からそのサイクルまでのtraining_positionsの和 |
| games | 整数 | 対局数。手数の上限に達した対局と、生成し直した分を含む |
| plies | 整数 | 手数の合計。gamesと同じ対局が対象 |
| draws | 整数 | 規則による引き分けで終わった対局数。手数の上限に達した対局を含まない |
| max_ply_games | 整数 | 手数の上限に達した対局数 |
| resigned_games | 整数 | 投了で終わった対局数 |
| train_policy | 実数 | そのサイクルで生成したデータのうち学習に使った局面の標本での方策の損失 |
| train_value_result | 実数 | 同じ訓練の標本での、対局結果に対する価値の損失 |
| train_legal_probability_mass | 実数 | 同じ訓練の標本での、合法手に割り当てられた確率の合計 |
| validation_policy | 実数 | 同じ生成データのうち学習に使わなかった検証局面での方策の損失 |
| validation_value_result | 実数 | 同じ検証局面での、対局結果に対する価値の損失 |
| validation_legal_probability_mass | 実数 | 同じ検証局面での、合法手に割り当てられた確率の合計 |
| setting_で始まる列 | 文字列 | 設定の値。列の数と名前は系列ごとに異なる |

学習の診断は、そのサイクルの学習を終えた重みで計算した局面ごとの平均である。
方策の損失は探索が選んだ手の分布とネットの出力の食い違いを表す。
生成し直した分は、局面数、対局数、手数、および終わり方の3件数すべてに含める。
1局の手数と終わり方の割合の分母は、手数の上限に達した対局を含むgamesである。
分母を定義するため、各サイクルのgamesは1以上を要する。

collectは、入力の欠落、測定名の欠落・重複、必要な欄の欠落、同じサイクルで
同じ相手・制限・測定条件の対局の重複、同じ組でのrulesまたはmax_plyの不一致、
同じサイクルの診断の重複、系列内のdata_sha256の不一致を拒否する。
plotもCSVの型・欠測・件数・重複・条件の整合を検査し、cycleが0でもcycles.csvに
ある番号でもない対局と診断、および局面数の累計の不一致を拒否する。

進捗図は横軸を共有する6パネルを上から次の順に並べる。横軸はサイクルcの
cumulative_training_positionsであり、サイクル0では0とし、百万局面単位で表示する。
最上段の上側には、主の系列のstrength.csvかvalidation.csvに行のあるサイクルの
位置にc10のような番号を付ける。同じ位置の番号は並べて表示する。

1. 強さは相手・制限・測定条件の組ごとにサイクル順で結び、95%信頼区間を誤差棒にする。
   得点率が0.1未満または0.9超の点は白抜きにし、0.1と0.9は塗る。空欄のEloは描かない。
   信頼区間の無限大の端はパネルの端まで伸ばして矢印を描く。点推定が無限大なら端に
   三角を描き、負は下向き、正は上向きとし、反対の端が有限ならそこまで誤差棒を描く。
   縦軸は描く全系列の有限の点と区間端から余白を付けて決める。対象の行があっても
   有限値がなければエラー、行がなければno dataとする。破棄ペアを2敗・2勝としたEloは描かない。
2. 方策の損失はtrain_policyとvalidation_policyを描く。
3. 価値の損失はtrain_value_result、validation_value_result、固定の検証用データの
   value_resultを描き、y = ln 2の水平線を加える。凡例で前の2本をper position、
   固定の検証用データをper gameとし、平均の取り方を区別する。
4. 一致率はpolicy_agreementと4帯の値を百分率で描き、空欄には点を描かない。
5. 1局の手数はplies / gamesを描く。
6. 終わり方はdraws / games、max_ply_games / games、resigned_games / gamesを百分率で描く。

setting_で始まる各列について、サイクルcとc−1の値が両方とも空欄でなく、文字列と
して異なるときだけ、c−1の累計の位置に全パネルを通る縦線を描く。最上段にsetting_
を除いた名前と新しい値を名前=値で記す。同じサイクルの変更は1本の線と注記にまとめる。

参照の系列は自身のcycles.csvで横軸を定め、主の系列と組が一致する強さだけを重ねる。
固定の検証用データの価値の損失と一致率5本はdata_sha256が一致するときだけ重ね、
主のvalidation.csvが空なら重ねない。同じ組と列は同じ色を使い、参照は透明度で薄くし、
凡例にreferenceと記す。cycles.csvから描く線と設定変更線は主の系列だけを描く。
"""

import argparse
import csv
from dataclasses import dataclass
import json
import math
from pathlib import Path
import re
import tomllib

from matplotlib.colors import TABLEAU_COLORS
from matplotlib.figure import Figure
from matplotlib.ticker import PercentFormatter


CYCLE_COLUMNS = (
    "cycle", "training_positions", "cumulative_training_positions", "games", "plies",
    "draws", "max_ply_games", "resigned_games", "train_policy", "train_value_result",
    "train_legal_probability_mass", "validation_policy", "validation_value_result",
    "validation_legal_probability_mass",
)
STRENGTH_COLUMNS = (
    "cycle", "opponent", "opponent_limit", "condition", "rules", "max_ply",
    "run_name", "network_sha256", "attempted_pairs", "valid_pairs", "discarded_pairs",
    "pentanomial_0", "pentanomial_1", "pentanomial_2", "pentanomial_3", "pentanomial_4",
    "score_rate", "elo", "elo_lower", "elo_upper", "elo_without_move_forfeits",
    "affected_pairs", "affected_pair_fraction", "elo_discarded_as_losses",
    "elo_discarded_as_wins",
)
BANDS = ((1, 40, "1_40"), (41, 100, "41_100"), (101, 200, "101_200"), (201, None, "201_up"))
VALIDATION_COLUMNS = (
    "cycle", "data_sha256", "game_count", "position_count", "value_result", "policy_agreement",
    *(f"{metric}_{suffix}" for _, _, suffix in BANDS
      for metric in ("value_result", "policy_agreement")),
)
STRENGTH_OPTIONAL = {
    "score_rate", "elo", "elo_lower", "elo_upper", "elo_without_move_forfeits",
}
COLORS = tuple(TABLEAU_COLORS.values())


def _fail(where, message):
    raise ValueError(f"{where}: {message}")


def _object(value, where):
    if not isinstance(value, dict):
        _fail(where, "オブジェクトが必要です")
    return value


def _array(value, where):
    if not isinstance(value, list):
        _fail(where, "配列が必要です")
    return value


def _field(value, name, where):
    _object(value, where)
    if name not in value:
        _fail(f"{where}.{name}", "欄がありません")
    return value[name]


def _exact_fields(value, fields, where):
    _object(value, where)
    if set(value) != set(fields):
        _fail(where, f"欄が一致しません: missing={sorted(set(fields) - set(value))}, "
              f"extra={sorted(set(value) - set(fields))}")


def _text(value, where):
    if not isinstance(value, str) or value == "":
        _fail(where, "空でない文字列が必要です")
    return value


def _integer(value, where, minimum=0):
    if type(value) is not int or value < minimum:
        _fail(where, f"{minimum}以上の整数が必要です")
    return value


def _number(value, where, *, elo=False):
    if elo and isinstance(value, str) and value in ("-inf", "+inf"):
        return float(value)
    if type(value) not in (int, float):
        _fail(where, "実数が必要です")
    try:
        number = float(value)
    except OverflowError:
        _fail(where, "実数の範囲を超えています")
    if math.isnan(number) or (not elo and not math.isfinite(number)):
        _fail(where, "有限の実数が必要です（Eloだけは無限大を許します）")
    return number


def _metric(value, where, *, probability=False):
    number = _number(value, where)
    if number < 0 or (probability and number > 1):
        _fail(where, "割合は0以上1以下、損失は0以上でなければなりません")
    return number


def _nullable(value, count, where, parser):
    if count == 0:
        if value is not None:
            _fail(where, "件数が0のときはnullが必要です")
        return None
    if value is None:
        _fail(where, "件数が0でないときは値が必要です")
    return parser(value, where)


def _read_json(path):
    def reject_constant(value):
        raise ValueError(f"JSONに定義されていない数値定数です: {value}")

    try:
        return json.loads(path.read_text(encoding="utf-8"), parse_constant=reject_constant)
    except (OSError, UnicodeError, ValueError) as error:
        _fail(f"{path}: JSON", str(error))


def _relative_file(root, value, where):
    path = Path(_text(value, where))
    if path.is_absolute():
        _fail(where, "リポジトリの根からの相対パスが必要です")
    path = root / path
    if not path.is_file():
        _fail(where, f"ファイルがありません: {path}")
    return path


def _statistics(value, where, *, interval):
    """空集合のEloはオブジェクト全体がnullであることを検査する。"""
    counts = _array(_field(value, "pentanomial", where), f"{where}.pentanomial")
    if len(counts) != 5:
        _fail(f"{where}.pentanomial", "度数は5個必要です")
    for index, count in enumerate(counts):
        _integer(count, f"{where}.pentanomial[{index}]")
    count = sum(counts)
    raw = _field(value, "elo", where)
    names = ("elo", "lower", "upper") if interval else ("elo",)
    if count == 0:
        if raw is not None:
            _fail(f"{where}.elo", "度数の和が0のときはnullが必要です")
        estimates = {name: None for name in names}
    else:
        estimates = {
            name: _number(_field(raw, name, f"{where}.elo"), f"{where}.elo.{name}", elo=True)
            for name in names
        }
        if interval and not estimates["lower"] <= estimates["elo"] <= estimates["upper"]:
            _fail(f"{where}.elo.lower/elo/upper", "信頼区間が点推定を含んでいません")
    if interval:
        score = _nullable(_field(value, "score_rate", where), count, f"{where}.score_rate",
                          lambda value, label: _metric(value, label, probability=True))
        estimates["score_rate"] = score
    return counts, estimates


def _strength_row(entry, root, where):
    path = _relative_file(root, entry["summary"], f"{where}.summary")
    records = _array(_read_json(path), f"{path}: summary")
    name = _text(entry["run_name"], f"{where}.run_name")
    selected = []
    for index, record in enumerate(records):
        run_name = _text(_field(record, "run_name", f"{path}: [{index}]"),
                         f"{path}: [{index}].run_name")
        if run_name == name:
            selected.append((record, f"{path}: [{index}]"))
    if len(selected) != 1:
        _fail(f"{path}: run_name", f"{name!r}はちょうど1件必要です。該当={len(selected)}")
    record, source = selected[0]
    baseline = _field(record, "baseline", source)
    opponent = _text(_field(baseline, "kind", f"{source}.baseline"), f"{source}.baseline.kind")
    if opponent not in ("alphabeta", "hachu"):
        _fail(f"{source}.baseline.kind", "alphabetaまたはhachuが必要です")
    limit = _object(_field(baseline, "limit", f"{source}.baseline"), f"{source}.baseline.limit")
    if len(limit) != 1:
        _fail(f"{source}.baseline.limit", "制限の欄はちょうど1個必要です")
    limit_name, limit_value = next(iter(limit.items()))
    _text(limit_name, f"{source}.baseline.limit")
    _integer(limit_value, f"{source}.baseline.limit.{limit_name}", 1)
    row = {
        "cycle": entry["cycle"], "opponent": opponent,
        "opponent_limit": f"{limit_name}={limit_value}", "condition": entry["condition"],
        "rules": _text(_field(record, "rules_source", source), f"{source}.rules_source"),
    }
    for name in ("max_ply", "attempted_pairs", "valid_pairs", "discarded_pairs", "affected_pairs"):
        row[name] = _integer(_field(record, name, source), f"{source}.{name}")
    for name in ("run_name", "network_sha256"):
        row[name] = _text(_field(record, name, source), f"{source}.{name}")
    counts, estimates = _statistics(_field(record, "valid", source), f"{source}.valid", interval=True)
    row.update({f"pentanomial_{index}": count for index, count in enumerate(counts)})
    row.update(score_rate=estimates["score_rate"], elo=estimates["elo"],
               elo_lower=estimates["lower"], elo_upper=estimates["upper"])
    for name in ("without_move_forfeits", "discarded_as_losses", "discarded_as_wins"):
        counts, estimates = _statistics(_field(record, name, source), f"{source}.{name}", interval=False)
        if name != "without_move_forfeits" and sum(counts) == 0:
            _fail(f"{source}.{name}.elo", "破棄ペアを含むEloは空欄にできません")
        row[f"elo_{name}"] = estimates["elo"]
    row["affected_pair_fraction"] = _metric(
        _field(record, "affected_pair_fraction", source), f"{source}.affected_pair_fraction",
        probability=True,
    )
    _check_strength(row, source)
    return row


def _validation_row(entry, root, where):
    path = _relative_file(root, entry["path"], f"{where}.path")
    record = _read_json(path)
    data = _field(record, "data", str(path))
    row = {"cycle": entry["cycle"],
           "data_sha256": _text(_field(data, "sha256", f"{path}: data"), f"{path}: data.sha256")}
    diagnostics = _field(record, "diagnostics", str(path))
    for name in ("game_count", "position_count"):
        row[name] = _integer(_field(diagnostics, name, f"{path}: diagnostics"),
                             f"{path}: diagnostics.{name}", 1)
    for name in ("value_result", "policy_agreement"):
        row[name] = _metric(_field(diagnostics, name, f"{path}: diagnostics"),
                            f"{path}: diagnostics.{name}", probability=name == "policy_agreement")
    bands = _array(_field(record, "bands", str(path)), f"{path}: bands")
    expected = {(first, last): suffix for first, last, suffix in BANDS}
    seen = set()
    for index, band in enumerate(bands):
        source = f"{path}: bands[{index}]"
        first = _integer(_field(band, "first_move", source), f"{source}.first_move", 1)
        last = _field(band, "last_move", source)
        if last is not None:
            _integer(last, f"{source}.last_move", 1)
        key = (first, last)
        if key not in expected or key in seen:
            _fail(source, f"帯が不正または重複しています: {key}")
        seen.add(key)
        count = _integer(_field(band, "position_count", source), f"{source}.position_count")
        for name in ("value_result", "policy_agreement"):
            row[f"{name}_{expected[key]}"] = _nullable(
                _field(band, name, source), count, f"{source}.{name}",
                lambda value, label: _metric(value, label, probability=name == "policy_agreement"),
            )
    if seen != set(expected):
        _fail(f"{path}: bands", "4つの帯をちょうど1回ずつ指定してください")
    return row


def _strength_key(row):
    return row["opponent"], row["opponent_limit"], row["condition"]


def _check_strength(row, where):
    _integer(row["attempted_pairs"], f"{where}.attempted_pairs", 1)
    _integer(row["max_ply"], f"{where}.max_ply", 1)
    if sum(row[f"pentanomial_{index}"] for index in range(5)) != row["valid_pairs"]:
        _fail(f"{where}.valid_pairs/pentanomial", "有効ペア数と度数の和が一致しません")
    if row["valid_pairs"] + row["discarded_pairs"] != row["attempted_pairs"]:
        _fail(f"{where}.attempted_pairs/valid_pairs/discarded_pairs", "ペア数の合計が一致しません")
    if not row["discarded_pairs"] <= row["affected_pairs"] <= row["attempted_pairs"]:
        _fail(f"{where}.affected_pairs", "破棄ペア数以上、試行ペア数以下でなければなりません")
    for name in ("score_rate", "elo", "elo_lower", "elo_upper"):
        if (row[name] is None) != (row["valid_pairs"] == 0):
            _fail(f"{where}.{name}", "有効ペアが0件のときだけ空欄にしてください")
    if row["elo"] is not None and not row["elo_lower"] <= row["elo"] <= row["elo_upper"]:
        _fail(f"{where}.elo_lower/elo/elo_upper", "信頼区間が点推定を含んでいません")
    if row["valid_pairs"] == 0 and row["elo_without_move_forfeits"] is not None:
        _fail(f"{where}.elo_without_move_forfeits", "有効ペアが0件なら除外後も空欄が必要です")


def _check_strength_series(rows, where):
    seen, conditions = set(), {}
    for index, row in enumerate(rows):
        key = _strength_key(row)
        identity = (row["cycle"], *key)
        if identity in seen:
            _fail(f"{where}[{index}].cycle/opponent/opponent_limit/condition", "同じ対局が重複しています")
        seen.add(identity)
        condition = (row["rules"], row["max_ply"])
        if key in conditions and conditions[key] != condition:
            _fail(f"{where}[{index}].rules/max_ply", "同じ組の対局で条件が一致しません")
        conditions[key] = condition


def _check_validation_series(rows, where):
    seen = set()
    hashes = set()
    for index, row in enumerate(rows):
        if row["cycle"] in seen:
            _fail(f"{where}[{index}].cycle", "診断のサイクルが重複しています")
        seen.add(row["cycle"])
        hashes.add(row["data_sha256"])
        if len(hashes) > 1:
            _fail(f"{where}[{index}].data_sha256", "系列内で固定の検証用データが一致しません")


def _write_csv(path, columns, rows):
    with path.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=columns, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def collect(directory: Path, root: Path) -> None:
    """全入力の検査を終えてから、測定のCSVだけを上書きする。"""
    directory, root = Path(directory), Path(root)
    path = directory / "series.toml"
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        _fail(f"{path}: TOML", str(error))
    extra = set(document) - {"strength", "validation"}
    if extra:
        _fail(str(path), f"未知の最上位の欄です: {sorted(extra)}")
    rows = {"strength": [], "validation": []}
    for section in rows:
        if section not in document:
            continue
        entries = _array(document[section], f"{path}: {section}")
        for index, entry in enumerate(entries):
            where = f"{path}: {section}[{index}]"
            fields = ("cycle", "summary", "run_name", "condition") if section == "strength" else ("cycle", "path")
            _exact_fields(entry, fields, where)
            _integer(entry["cycle"], f"{where}.cycle")
            if section == "strength":
                _text(entry["condition"], f"{where}.condition")
                row = _strength_row(entry, root, where)
            else:
                row = _validation_row(entry, root, where)
            rows[section].append(row)
    _check_strength_series(rows["strength"], f"{path}: strength")
    _check_validation_series(rows["validation"], f"{path}: validation")
    rows["strength"].sort(key=lambda row: (row["cycle"], *_strength_key(row)))
    rows["validation"].sort(key=lambda row: row["cycle"])
    _write_csv(directory / "strength.csv", STRENGTH_COLUMNS, rows["strength"])
    _write_csv(directory / "validation.csv", VALIDATION_COLUMNS, rows["validation"])


@dataclass
class Series:
    """3つのCSVの検査を通った、作図に使う系列の値を保持する。"""

    directory: Path
    cycles: list[dict]
    strength: list[dict]
    validation: list[dict]
    settings: list[str]


def _read_csv(path, columns, integers, strings, optional, *, settings=False):
    try:
        with path.open(encoding="utf-8", newline="") as source:
            reader = csv.reader(source, strict=True)
            try:
                header = next(reader)
            except StopIteration:
                _fail(f"{path}: header", "ヘッダーがありません")
            if len(header) != len(set(header)):
                _fail(f"{path}: header", "列名が重複しています")
            if settings:
                if (not set(columns) <= set(header)
                        or any(name not in columns and not name.startswith("setting_") for name in header)):
                    _fail(f"{path}: header", "必須列の欠落または未知の列があります")
            elif header != list(columns):
                _fail(f"{path}: header", f"列の集合と順序が一致しません。必要な列={columns}")
            rows = []
            for values in reader:
                where = f"{path}:{reader.line_num}"
                if len(values) != len(header):
                    _fail(f"{where}: row", "行の列数がヘッダーと一致しません")
                row = {}
                for name, value in zip(header, values, strict=True):
                    label = f"{where}: {name}"
                    if settings and name.startswith("setting_"):
                        row[name] = value
                    elif value == "" and name in optional:
                        row[name] = None
                    elif name in strings:
                        row[name] = _text(value, label)
                    elif name in integers:
                        if re.fullmatch(r"[+-]?[0-9]+", value) is None:
                            _fail(label, "整数として読めません")
                        try:
                            number = int(value)
                        except ValueError as error:
                            _fail(label, str(error))
                        row[name] = _integer(number, label)
                    else:
                        try:
                            number = float(value)
                        except ValueError:
                            _fail(label, "実数として読めません")
                        row[name] = _number(number, label, elo=name.startswith("elo"))
                rows.append(row)
            return header, rows
    except (OSError, UnicodeError, csv.Error) as error:
        _fail(f"{path}: CSV", str(error))


def read_series(directory: Path) -> Series:
    """CSVの型・欠測条件・サイクルの対応を検査し、数値を変換して返す。"""
    directory = Path(directory)
    path = directory / "cycles.csv"
    header, cycles = _read_csv(path, CYCLE_COLUMNS, set(CYCLE_COLUMNS[:8]), set(), set(), settings=True)
    total = 0
    for expected_cycle, row in enumerate(cycles, 1):
        where = f"{path}:{expected_cycle + 1}"
        if row["cycle"] != expected_cycle:
            _fail(f"{where}: cycle", "サイクルは1から連続する順に並べてください")
        total += row["training_positions"]
        if row["cumulative_training_positions"] != total:
            _fail(f"{where}: cumulative_training_positions", "training_positionsの累計と一致しません")
        _integer(row["games"], f"{where}: games", 1)
        for name in ("draws", "max_ply_games", "resigned_games"):
            if row[name] > row["games"]:
                _fail(f"{where}: {name}", "対局数を超えています")
        if sum(row[name] for name in ("draws", "max_ply_games", "resigned_games")) > row["games"]:
            _fail(f"{where}: draws/max_ply_games/resigned_games", "終わり方の件数の和が対局数を超えています")
        for name in CYCLE_COLUMNS[8:]:
            _metric(row[name], f"{where}: {name}", probability=name.endswith("probability_mass"))

    path = directory / "strength.csv"
    _, strength = _read_csv(
        path, STRENGTH_COLUMNS,
        {"cycle", "max_ply", "attempted_pairs", "valid_pairs", "discarded_pairs", "affected_pairs",
         *(f"pentanomial_{index}" for index in range(5))},
        {"opponent", "opponent_limit", "condition", "rules", "run_name", "network_sha256"},
        STRENGTH_OPTIONAL,
    )
    for index, row in enumerate(strength, 2):
        where = f"{path}:{index}"
        if row["opponent"] not in ("alphabeta", "hachu"):
            _fail(f"{where}: opponent", "alphabetaまたはhachuが必要です")
        _check_strength(row, where)
        for name in ("score_rate", "affected_pair_fraction"):
            if row[name] is not None:
                _metric(row[name], f"{where}: {name}", probability=True)
    _check_strength_series(strength, str(path))

    path = directory / "validation.csv"
    _, validation = _read_csv(path, VALIDATION_COLUMNS, {"cycle", "game_count", "position_count"},
                               {"data_sha256"}, set(VALIDATION_COLUMNS[6:]))
    for index, row in enumerate(validation, 2):
        where = f"{path}:{index}"
        for name in ("game_count", "position_count"):
            _integer(row[name], f"{where}: {name}", 1)
        for name in VALIDATION_COLUMNS[4:]:
            if row[name] is not None:
                _metric(row[name], f"{where}: {name}", probability=name.startswith("policy_agreement"))
        for _, _, suffix in BANDS:
            if (row[f"value_result_{suffix}"] is None) != (row[f"policy_agreement_{suffix}"] is None):
                _fail(f"{where}: value_result_{suffix}/policy_agreement_{suffix}", "帯の2列は同時に空欄にしてください")
    _check_validation_series(validation, str(path))
    cycle_numbers = {0, *(row["cycle"] for row in cycles)}
    for name, rows in (("strength", strength), ("validation", validation)):
        for index, row in enumerate(rows, 2):
            if row["cycle"] not in cycle_numbers:
                _fail(f"{directory / (name + '.csv')}:{index}: cycle",
                      f"サイクル{row['cycle']}は0でもcycles.csvにある番号でもありません")
        rows.sort(key=lambda row: row["cycle"])
    return Series(directory, cycles, strength, validation,
                  [name for name in header if name.startswith("setting_")])


def _positions(series):
    return {0: 0.0, **{row["cycle"]: row["cumulative_training_positions"] / 1_000_000
                      for row in series.cycles}}


def _strength_limits(rows, where):
    finite = [row[name] for row in rows if row["elo"] is not None
              for name in ("elo", "elo_lower", "elo_upper") if math.isfinite(row[name])]
    if not finite:
        _fail(f"{where}: elo/elo_lower/elo_upper", "行がありますが、描く有限の値がありません")
    lower, upper = min(finite), max(finite)
    margin = max((upper - lower) * 0.08, 1.0)
    return lower - margin, upper + margin


def _draw_strength(axis, rows, positions, key, color, role):
    rows = sorted((row for row in rows if _strength_key(row) == key and row["elo"] is not None),
                  key=lambda row: row["cycle"])
    if not rows:
        return
    alpha = 1.0 if role == "main" else 0.3
    label = ", ".join(key) + (" (reference)" if role == "reference" else "")
    gid = f"strength:{role}:{','.join(key)}"
    lower_edge, upper_edge = axis.get_ylim()

    def bounded(value):
        if value == -math.inf:
            return lower_edge
        if value == math.inf:
            return upper_edge
        return value

    axis.plot([positions[row["cycle"]] for row in rows], [bounded(row["elo"]) for row in rows],
              color=color, alpha=alpha, label=label, gid=f"{gid}:line", linewidth=1.2)
    for row in rows:
        x, estimate = positions[row["cycle"]], row["elo"]
        y, lower, upper = (bounded(row[name]) for name in ("elo", "elo_lower", "elo_upper"))
        point_gid = f"{gid}:c{row['cycle']}"
        marker = "o" if math.isfinite(estimate) else ("v" if estimate < 0 else "^")
        hollow = row["score_rate"] < 0.1 or row["score_rate"] > 0.9
        axis.plot([x], [y], marker=marker, linestyle="none", color=color, alpha=alpha,
                  markerfacecolor="none" if hollow else color, markersize=6,
                  clip_on=False, gid=f"{point_gid}:point")
        if lower != upper:
            errors = axis.errorbar([x], [y], yerr=[[y - lower], [upper - y]], fmt="none",
                                  ecolor=color, alpha=alpha, capsize=3, linewidth=1)
            for bar in errors.lines[2]:
                bar.set_gid(f"{point_gid}:interval")
        if math.isfinite(estimate):
            for name, edge, direction in (("lower", lower_edge, 1), ("upper", upper_edge, -1)):
                if math.isinf(row[f"elo_{name}"]):
                    arrow = axis.annotate(
                        "", xy=(x, edge), xytext=(x, edge + direction * (upper_edge - lower_edge) * 0.05),
                        arrowprops={"arrowstyle": "-|>", "color": color, "alpha": alpha,
                                    "linewidth": 1, "shrinkA": 0, "shrinkB": 0},
                        annotation_clip=False,
                    )
                    arrow.set_gid(f"{point_gid}:{name}-arrow")


def _draw_validation(axes, series, role):
    if not series.validation:
        return
    positions = _positions(series)
    x = [positions[row["cycle"]] for row in series.validation]
    metrics = [(2, "value_result", "Fixed validation (per game)", COLORS[2]),
               (3, "policy_agreement", "All moves", COLORS[0])]
    metrics += [(3, f"policy_agreement_{suffix}", f"Moves {first}-{last}" if last is not None else "Moves 201+",
                 COLORS[index + 1]) for index, (first, last, suffix) in enumerate(BANDS)]
    for panel, name, label, color in metrics:
        values = [math.nan if row[name] is None else row[name] for row in series.validation]
        axes[panel].plot(x, values, marker="o", markersize=3, color=color,
                         alpha=1.0 if role == "main" else 0.3,
                         label=label + (" (reference)" if role == "reference" else ""),
                         gid=f"validation:{role}:{name}")


def _draw_settings(axes, series, positions):
    for previous, current in zip(series.cycles, series.cycles[1:]):
        changes = [f"{name.removeprefix('setting_')}={current[name]}" for name in series.settings
                   if previous[name] != "" and current[name] != "" and previous[name] != current[name]]
        if not changes:
            continue
        x = positions[previous["cycle"]]
        for axis in axes:
            axis.axvline(x, color="0.5", linestyle=":", linewidth=0.8,
                         gid=f"setting:c{current['cycle']}")
        axes[0].annotate("\n".join(changes), xy=(x, 0.98), xycoords=("data", "axes fraction"),
                         xytext=(4, -2), textcoords="offset points", va="top", rotation=90,
                         fontsize=8, gid=f"setting-label:c{current['cycle']}")


def build_figure(directory: Path, reference: Path | None = None) -> Figure:
    """CSVだけからFigureを作り、保存は呼び出し元に任せる。"""
    main = read_series(directory)
    reference_series = read_series(reference) if reference is not None else None
    positions = _positions(main)
    keys = sorted({_strength_key(row) for row in main.strength})
    reference_strength = []
    if reference_series is not None:
        reference_strength = [row for row in reference_series.strength if _strength_key(row) in keys]
    displayed = main.strength + reference_strength
    limits = None
    if displayed:
        paths = str(main.directory / "strength.csv")
        if reference_strength:
            paths += f", {reference_series.directory / 'strength.csv'}"
        limits = _strength_limits(displayed, paths)

    figure = Figure(figsize=(12, 19), layout="constrained")
    axes = figure.subplots(6, 1, sharex=True, gridspec_kw={"height_ratios": [1.6, 1, 1, 1, 1, 1]})
    labels = ("Elo difference", "Policy loss", "Value loss", "Policy agreement", "Plies per game", "Game endings")
    for axis, label in zip(axes, labels, strict=True):
        axis.set_ylabel(label)
        axis.grid(alpha=0.2)
    axes[-1].set_xlabel("Cumulative training positions (millions)")
    if limits is None:
        axes[0].text(0.5, 0.5, "no data", transform=axes[0].transAxes, ha="center", va="center")
    else:
        axes[0].set_ylim(*limits)
        for index, key in enumerate(keys):
            color = COLORS[index % len(COLORS)]
            _draw_strength(axes[0], main.strength, positions, key, color, "main")
            if reference_series is not None:
                _draw_strength(axes[0], reference_strength, _positions(reference_series), key, color, "reference")

    x = [positions[row["cycle"]] for row in main.cycles]
    for panel, names in ((1, ("train_policy", "validation_policy")),
                         (2, ("train_value_result", "validation_value_result"))):
        for index, name in enumerate(names):
            label = "Training sample" if index == 0 else "Validation positions"
            if panel == 2:
                label += " (per position)"
            axes[panel].plot(x, [row[name] for row in main.cycles], color=COLORS[index],
                             label=label, gid=f"cycles:{name}")
    axes[2].axhline(math.log(2), color="0.4", linestyle="--", label="ln 2", gid="value:ln2")
    _draw_validation(axes, main, "main")
    if (reference_series is not None and main.validation and reference_series.validation
            and main.validation[0]["data_sha256"] == reference_series.validation[0]["data_sha256"]):
        _draw_validation(axes, reference_series, "reference")
    axes[4].plot(x, [row["plies"] / row["games"] for row in main.cycles], color=COLORS[0],
                 label="Self-play", gid="cycles:plies_per_game")
    for index, (name, label) in enumerate((("draws", "Rules draw"), ("max_ply_games", "Ply limit"),
                                          ("resigned_games", "Resignation"))):
        axes[5].plot(x, [row[name] / row["games"] for row in main.cycles], color=COLORS[index],
                     label=label, gid=f"cycles:{name}")
    axes[3].yaxis.set_major_formatter(PercentFormatter(xmax=1))
    axes[5].yaxis.set_major_formatter(PercentFormatter(xmax=1))
    _draw_settings(axes, main, positions)

    ticks = {}
    for cycle in sorted({row["cycle"] for row in main.strength + main.validation}):
        position = positions[cycle]
        if position not in ticks:
            ticks[position] = []
        ticks[position].append(f"c{cycle}")
    top = axes[0].secondary_xaxis("top")
    top.set_xticks(list(ticks), [" / ".join(labels) for labels in ticks.values()])
    top.set_xlabel("Measured cycles (main series)")
    for axis in axes:
        if axis.get_legend_handles_labels()[0]:
            axis.legend(fontsize=8, ncols=2, loc="best")
    return figure


def plot(directory: Path, reference: Path | None = None) -> Path:
    """進捗図をPNGで上書きする。"""
    figure = build_figure(directory, reference)
    path = Path(directory) / "progress.png"
    figure.savefig(path, dpi=150)
    return path


def main():
    parser = argparse.ArgumentParser(description="学習の系列の測定を収集し、進捗図を作ります。")
    commands = parser.add_subparsers(dest="command", required=True)
    collect_parser = commands.add_parser("collect", help="測定からstrength.csvとvalidation.csvを作る")
    collect_parser.add_argument("directory", type=Path)
    plot_parser = commands.add_parser("plot", help="3つのCSVからprogress.pngを作る")
    plot_parser.add_argument("directory", type=Path)
    plot_parser.add_argument("--reference", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "collect":
            collect(args.directory, Path(__file__).resolve().parents[3])
        else:
            plot(args.directory, args.reference)
    except (ValueError, OSError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
