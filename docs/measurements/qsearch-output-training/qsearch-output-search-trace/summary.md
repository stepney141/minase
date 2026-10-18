# 探索の見落としの追跡結果

値と損失の単位はセンチポーンである。判定差 m は142.1とする。
経路の s は提案手、a は教師最善手から始まる。上下界は各訪問の入口の窓で判定する。
機構名は TraceDisable の識別子であり、事前登録の介入の表に対応する。

## 根 202610280-278-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 6 | 4j5j | 8j11j | 複数機構の疑い | correction, futility, lmr, null_move, qs_delta, qs_see |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |
| a | 1 | 経路上の機構 | tt_cutoff | 500 | 500 | 655 | (631, 632) | lower | aspiration, tt_cutoff |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 8j11j | 0 | True |
| iir | 4j5j | 235 | False |
| null_move | 8j11j | 0 | True |
| futility | 8j11j | 0 | True |
| see | 4j5j | 235 | False |
| lmr | 8j11j | 0 | True |
| qs_delta | 8j11j | 0 | True |
| qs_see | 8j11j | 0 | True |
| qs_tt | 4j5j | 235 | False |
| aspiration | 8j11j | 0 | True |
| correction | 8j11j | 0 | True |
| 同時: aspiration, tt_cutoff | 8j11j | 0 | True |

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 4j5j | searched | -782 | (-778, -777) | upper |
| tt_cutoff | a | 8j11j | searched | -287 | (-686, -170) | exact |
| iir | s | 4j5j | searched | -632 | (-923, -170) | exact |
| iir | a | 8j11j | searched | -655 | (-632, -631) | upper |
| null_move | s | 4j5j | searched | -805 | (-308, -307) | upper |
| null_move | a | 8j11j | searched | -287 | (-308, -170) | exact |
| futility | s | 4j5j | searched | -875 | (-923, -170) | exact |
| futility | a | 8j11j | searched | -287 | (-700, -170) | exact |
| see | s | 4j5j | searched | -609 | (-923, -170) | exact |
| see | a | 8j11j | searched | -655 | (-609, -608) | upper |
| lmr | s | 4j5j | searched | -349 | (-308, -307) | upper |
| lmr | a | 8j11j | searched | -287 | (-308, -170) | exact |
| qs_delta | s | 4j5j | searched | -302 | (-287, -286) | upper |
| qs_delta | a | 8j11j | searched | -287 | (-308, -170) | exact |
| qs_see | s | 4j5j | searched | -257 | (-232, -231) | upper |
| qs_see | a | 8j11j | searched | -232 | (-262, -170) | exact |
| qs_tt | s | 4j5j | searched | -609 | (-923, -170) | exact |
| qs_tt | a | 8j11j | searched | -619 | (-609, -608) | upper |
| aspiration | s | 4j5j | searched | -363 | (-287, -286) | upper |
| aspiration | a | 8j11j | searched | -287 | (-30001, 30001) | exact |
| correction | s | 4j5j | searched | -767 | (-923, -170) | exact |
| correction | a | 8j11j | searched | -287 | (-733, -170) | exact |
| 同時: aspiration, tt_cutoff | s | 4j5j | searched | -363 | (-287, -286) | upper |
| 同時: aspiration, tt_cutoff | a | 8j11j | searched | -287 | (-30001, 30001) | exact |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |
| tt_cutoff | 8j11j | 5 | 0 |
| null_move | 8j11j | 6 | 0 |
| futility | 8j11j | 5 | 0 |
| lmr | 8j11j | 5 | 0 |
| qs_delta | 8j11j | 6 | 0 |
| qs_see | 8j11j | 6 | 0 |
| aspiration | 8j11j | 7 | 0 |
| correction | 8j11j | 5 | 0 |

## 根 202610489-236-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 6 | 9i8h | 4k5k | 1機構の特定 | futility, iir, qs_delta, qs_see, tt_cutoff |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 1 | 減深 | searched | 853 | 853 | 378 | (332, 400) | exact | lmr, null_move |
| a | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 7j7k | 24 | True |
| iir | 7j7k | 24 | True |
| null_move | 9i8h | 391 | False |
| futility | 7j7k | 24 | True |
| see | 9i8h | 391 | False |
| lmr | 1g1f | 28 | True |
| qs_delta | 7j7k | 24 | True |
| qs_see | 7j7k | 24 | True |
| qs_tt | 9i8h | 391 | False |
| aspiration | 9i8h | 391 | False |
| correction | 9i8h | 391 | False |
| 同時: lmr, null_move | 6j7i | 10 | True |

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 9i8h | searched | -403 | (-400, -399) | upper |
| tt_cutoff | a | 4k5k | searched | -428 | (-400, -399) | upper |
| iir | s | 9i8h | searched | -401 | (-400, -399) | upper |
| iir | a | 4k5k | searched | -417 | (-400, -399) | upper |
| null_move | s | 9i8h | searched | -391 | (-401, -309) | exact |
| null_move | a | 4k5k | searched | -402 | (-391, -390) | upper |
| futility | s | 9i8h | searched | -401 | (-400, -332) | upper |
| futility | a | 4k5k | searched | -417 | (-400, -399) | upper |
| see | s | 9i8h | searched | -378 | (-400, -332) | exact |
| see | a | 4k5k | searched | -417 | (-378, -377) | upper |
| lmr | s | 9i8h | searched | -393 | (-392, -332) | upper |
| lmr | a | 4k5k | searched | -394 | (-392, -391) | upper |
| qs_delta | s | 9i8h | searched | -403 | (-400, -399) | upper |
| qs_delta | a | 4k5k | searched | -417 | (-400, -399) | upper |
| qs_see | s | 9i8h | searched | -401 | (-400, -332) | upper |
| qs_see | a | 4k5k | searched | -417 | (-400, -399) | upper |
| qs_tt | s | 9i8h | searched | -378 | (-400, -332) | exact |
| qs_tt | a | 4k5k | searched | -388 | (-378, -332) | upper |
| aspiration | s | 9i8h | searched | -378 | (-400, 30001) | exact |
| aspiration | a | 4k5k | searched | -417 | (-378, -377) | upper |
| correction | s | 9i8h | searched | -378 | (-400, -332) | exact |
| correction | a | 4k5k | searched | -417 | (-378, -377) | upper |
| 同時: lmr, null_move | s | 9i8h | searched | -402 | (-402, -310) | upper |
| 同時: lmr, null_move | a | 4k5k | searched | -405 | (-402, -401) | upper |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |
| tt_cutoff | 7j7k | 5 | 24 |
| iir | 7j7k | 6 | 24 |
| futility | 7j7k | 5 | 24 |
| lmr | 6k6l | 5 | -42 |
| qs_delta | 7j7k | 6 | 24 |
| qs_see | 7j7k | 5 | 24 |

## 根 202612388-210-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 6 | 8k8b+ | 9f10e | 特定できない | lmr, qs_delta |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 5 | 乖離点の未確認 | sibling_beta_cutoff | 681 | 960 | 835 | (393, 394) | lower | なし |
| a | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 8k8b+ | 384 | False |
| iir | 8k8b+ | 384 | False |
| null_move | 8k8b+ | 384 | False |
| futility | 8k8b+ | 384 | False |
| see | 8k8b+ | 384 | False |
| lmr | 9f10e | 0 | True |
| qs_delta | 9f10e | 0 | True |
| qs_see | 8k8b+ | 384 | False |
| qs_tt | 8k8b+ | 384 | False |
| aspiration | 8k8b+ | 384 | False |
| correction | 8k8b+ | 384 | False |

関与機構がないため、同時の介入は行わない。

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 8k8b+ | searched | -394 | (-405, -313) | exact |
| tt_cutoff | a | 9f10e | searched | -407 | (-394, -393) | upper |
| iir | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| iir | a | 9f10e | searched | -407 | (-394, -393) | upper |
| null_move | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| null_move | a | 9f10e | searched | -402 | (-394, -393) | upper |
| futility | s | 8k8b+ | searched | -394 | (-481, -251) | exact |
| futility | a | 9f10e | searched | -407 | (-394, -393) | upper |
| see | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| see | a | 9f10e | searched | -407 | (-394, -393) | upper |
| lmr | s | 8k8b+ | searched | -267 | (-260, -259) | upper |
| lmr | a | 9f10e | searched | -260 | (-325, -233) | exact |
| qs_delta | s | 8k8b+ | searched | -285 | (-244, -243) | upper |
| qs_delta | a | 9f10e | searched | -244 | (-247, -155) | exact |
| qs_see | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| qs_see | a | 9f10e | searched | -403 | (-394, -393) | upper |
| qs_tt | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| qs_tt | a | 9f10e | searched | -407 | (-394, -393) | upper |
| aspiration | s | 8k8b+ | searched | -394 | (-412, 30001) | exact |
| aspiration | a | 9f10e | searched | -407 | (-394, -393) | upper |
| correction | s | 8k8b+ | searched | -394 | (-409, -271) | exact |
| correction | a | 9f10e | searched | -407 | (-394, -393) | upper |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |
| lmr | 9f11g | 4 | 27 |
| qs_delta | 9f10e | 6 | 0 |

## 根 202612852-212-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 6 | 7g6h | 1g2h | 1機構の特定 | lmr, qs_delta, see, tt_cutoff |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |
| a | 1 | 兄弟手による打ち切り | sibling_beta_cutoff | -1546 | -1546 | -1195 | (-1336, -1335) | lower | aspiration, null_move |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 1g2h | 0 | True |
| iir | 7g6h | 241 | False |
| null_move | 7g6h | 241 | False |
| futility | 7g6h | 241 | False |
| see | 1g2h | 0 | True |
| lmr | 1g2h | 0 | True |
| qs_delta | 1g2h | 0 | True |
| qs_see | 7g6h | 241 | False |
| qs_tt | 7g6h | 241 | False |
| aspiration | 1g2h | 0 | True |
| correction | 7g6h | 241 | False |
| 同時: aspiration, null_move | 7g6h | 241 | False |

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 7g6h | searched | 1586 | (1631, 1632) | upper |
| tt_cutoff | a | 1g2h | searched | 1631 | (1310, 1724) | exact |
| iir | s | 7g6h | searched | 1335 | (1212, 1350) | exact |
| iir | a | 1g2h | searched | 1195 | (1335, 1336) | upper |
| null_move | s | 7g6h | searched | 1335 | (1212, 1350) | exact |
| null_move | a | 1g2h | searched | 1134 | (1335, 1336) | upper |
| futility | s | 7g6h | searched | 1335 | (1212, 1350) | exact |
| futility | a | 1g2h | searched | 1335 | (1335, 1336) | upper |
| see | s | 7g6h | searched | 1603 | (1631, 1632) | upper |
| see | a | 1g2h | searched | 1631 | (1310, 1724) | exact |
| lmr | s | 7g6h | searched | 1586 | (1637, 1638) | upper |
| lmr | a | 1g2h | searched | 1637 | (1310, 1724) | exact |
| qs_delta | s | 7g6h | searched | 1618 | (1637, 1638) | upper |
| qs_delta | a | 1g2h | searched | 1637 | (1310, 1724) | exact |
| qs_see | s | 7g6h | searched | 1335 | (1264, 1356) | exact |
| qs_see | a | 1g2h | searched | 1305 | (1335, 1336) | upper |
| qs_tt | s | 7g6h | searched | 1335 | (1212, 1350) | exact |
| qs_tt | a | 1g2h | searched | 1195 | (1335, 1336) | upper |
| aspiration | s | 7g6h | searched | 1318 | (1124, 30001) | exact |
| aspiration | a | 1g2h | searched | 1482 | (1318, 30001) | exact |
| correction | s | 7g6h | searched | 1335 | (1212, 1350) | exact |
| correction | a | 1g2h | searched | 1195 | (1335, 1336) | upper |
| 同時: aspiration, null_move | s | 7g6h | searched | 1318 | (1094, 30001) | exact |
| 同時: aspiration, null_move | a | 1g2h | searched | 1284 | (1318, 1319) | upper |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |
| tt_cutoff | 1g2h | 7 | 0 |
| see | 1g2h | 7 | 0 |
| lmr | 1g2h | 5 | 0 |
| qs_delta | 1g2h | 7 | 0 |
| aspiration | 7g8h | 5 | 269 |

## 根 202613669-264-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 6 | 4f3d | 8k8d+ | 複数機構の疑い | なし |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |
| a | 2 | 減深 | searched | 2553 | 2693 | 2291 | (2410, 2411) | upper | aspiration, lmr, null_move |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 4f5d | 203 | False |
| iir | 4f5d | 203 | False |
| null_move | 4f3d | 171 | False |
| futility | 4f3d | 171 | False |
| see | 4f5d | 203 | False |
| lmr | 4f5d | 203 | False |
| qs_delta | 4f5d | 203 | False |
| qs_see | 4f5d | 203 | False |
| qs_tt | 4f5d | 203 | False |
| aspiration | 4f3d | 171 | False |
| correction | 4f5d | 203 | False |
| 同時: aspiration, lmr, null_move | 12i11h | 123 | True |

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 4f3d | searched | 2521 | (2524, 2525) | upper |
| tt_cutoff | a | 8k8d+ | searched | 2291 | (2524, 2525) | upper |
| iir | s | 4f3d | searched | 2490 | (2493, 2494) | upper |
| iir | a | 8k8d+ | searched | 2113 | (2493, 2494) | upper |
| null_move | s | 4f3d | searched | 2473 | (2401, 2493) | exact |
| null_move | a | 8k8d+ | searched | 2291 | (2473, 2474) | upper |
| futility | s | 4f3d | searched | 2423 | (2410, 2631) | exact |
| futility | a | 8k8d+ | searched | 2228 | (2410, 2411) | upper |
| see | s | 4f3d | searched | 2359 | (2500, 2501) | upper |
| see | a | 8k8d+ | searched | 2291 | (2500, 2501) | upper |
| lmr | s | 4f3d | searched | 2521 | (2524, 2525) | upper |
| lmr | a | 8k8d+ | searched | 2305 | (2524, 2525) | upper |
| qs_delta | s | 4f3d | searched | 2521 | (2524, 2525) | upper |
| qs_delta | a | 8k8d+ | searched | 2305 | (2524, 2525) | upper |
| qs_see | s | 4f3d | searched | 2521 | (2524, 2525) | upper |
| qs_see | a | 8k8d+ | searched | 2291 | (2524, 2525) | upper |
| qs_tt | s | 4f3d | searched | 2521 | (2524, 2525) | upper |
| qs_tt | a | 8k8d+ | searched | 2291 | (2524, 2525) | upper |
| aspiration | s | 4f3d | searched | 2473 | (2420, 30001) | exact |
| aspiration | a | 8k8d+ | searched | 2291 | (2473, 2474) | upper |
| correction | s | 4f3d | searched | 2227 | (2495, 2496) | upper |
| correction | a | 8k8d+ | searched | 2291 | (2495, 2496) | upper |
| 同時: aspiration, lmr, null_move | s | 4f3d | searched | 2328 | (2335, 2336) | upper |
| 同時: aspiration, lmr, null_move | a | 8k8d+ | searched | 2305 | (2335, 2336) | upper |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |

## 根 202614203-172-r

| d0 | s0 | a* | 分類 | 介入だけで成功 |
| --- | --- | --- | --- | --- |
| 5 | 8i9h | 9j9i | 1機構の特定 | qs_see, tt_cutoff |

| 経路 | D | 停止の理由 | 停止時の outcome | t_D（N_D手番側） | t'_D（N_D手番側） | N_Dの値（N_D手番側） | 窓（N_D手番側） | 上下界 | 関与機構 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| s | 1 | 減深 | searched | 1464 | 1464 | 1255 | (1184, 1265) | exact | lmr, null_move |
| a | 該当なし | 乖離なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | 該当なし | なし |

| 無効化した機構 | 最善手 | 損失（根手番側） | 成功 |
| --- | --- | --- | --- |
| tt_cutoff | 9g6j | 8 | True |
| iir | 8i9h | 171 | False |
| null_move | 8i9h | 171 | False |
| futility | 8i9h | 171 | False |
| see | 8i9i | 187 | False |
| lmr | 9g10h | 37 | True |
| qs_delta | 8i9h | 171 | False |
| qs_see | 9g9i | 7 | True |
| qs_tt | 8i9h | 171 | False |
| aspiration | 8i9h | 171 | False |
| correction | 8i9h | 171 | False |
| 同時: lmr, null_move | 9g11g | 37 | True |

| 介入 | 経路 | 根の手 | outcome | 最後の値（根手番側） | 窓（根手番側） | 上下界 |
| --- | --- | --- | --- | --- | --- | --- |
| tt_cutoff | s | 8i9h | searched | -1247 | (-1247, -1246) | upper |
| tt_cutoff | a | 9j9i | searched | -1270 | (-1276, -1184) | exact |
| iir | s | 8i9h | searched | -1255 | (-1265, -1184) | exact |
| iir | a | 9j9i | searched | -1270 | (-1276, -1184) | exact |
| null_move | s | 8i9h | searched | -1229 | (-1240, -1184) | exact |
| null_move | a | 9j9i | searched | -1300 | (-1276, -1275) | upper |
| futility | s | 8i9h | searched | -1255 | (-1265, -1184) | exact |
| futility | a | 9j9i | searched | -1300 | (-1276, -1275) | upper |
| see | s | 8i9h | searched | -1265 | (-1265, -1264) | upper |
| see | a | 9j9i | searched | -1270 | (-1276, -1184) | exact |
| lmr | s | 8i9h | searched | -1290 | (-1290, -1289) | upper |
| lmr | a | 9j9i | searched | -1300 | (-1290, -1289) | upper |
| qs_delta | s | 8i9h | searched | -1255 | (-1265, -1184) | exact |
| qs_delta | a | 9j9i | searched | -1300 | (-1276, -1275) | upper |
| qs_see | s | 8i9h | searched | -1230 | (-1224, -1223) | upper |
| qs_see | a | 9j9i | searched | -1248 | (-1224, -1223) | upper |
| qs_tt | s | 8i9h | searched | -1255 | (-1265, -1184) | exact |
| qs_tt | a | 9j9i | searched | -1270 | (-1276, -1184) | exact |
| aspiration | s | 8i9h | searched | -1229 | (-1241, 30001) | exact |
| aspiration | a | 9j9i | searched | -1300 | (-1285, -1284) | upper |
| correction | s | 8i9h | searched | -1255 | (-1265, -1184) | exact |
| correction | a | 9j9i | searched | -1270 | (-1276, -1184) | exact |
| 同時: lmr, null_move | s | 8i9h | searched | -1290 | (-1290, -1289) | upper |
| 同時: lmr, null_move | a | 9j9i | searched | -1300 | (-1290, -1289) | upper |

次の固定100,000ノードの結果は分類に使わない。

| 無効化した機構 | 最善手 | 完了深さ | 損失（根手番側） |
| --- | --- | --- | --- |
| tt_cutoff | 9g6j | 5 | 8 |
| lmr | 9g5k | 4 | 14 |
| qs_see | 9g9i | 5 | 7 |

## 機構ごとの件数

| 機構 | 件数（根） |
| --- | --- |
| tt_cutoff | 1 |
| iir | 0 |
| null_move | 0 |
| futility | 0 |
| see | 0 |
| lmr | 2 |
| qs_delta | 0 |
| qs_see | 0 |
| qs_tt | 0 |
| aspiration | 2 |
| correction | 0 |

3根以上で数えられた機構は、なし。

静止探索の3機構はいずれも0根であり、件数規則による注記は不要である。
