# AdamWの更新をCPUとGPUで比べるときは、CPUの参照の順伝播をGPUと同じ順の単精度にし、局面方向の和だけを倍精度にする

## 症状

NNUEのLayerStackの学習器で、CPUの参照実装を順伝播から倍精度で計算し、GPUを単精度で計算して局面方向の和を加算の順を固定した木で足したところ、順伝播、損失、および勾配は許容差（`|GPU − CPU| ≤ 10⁻⁶ ＋ 10⁻⁵×|CPU|`）に収まったが、1回と3回のAdamWの更新の後の第1層の重みの1成分が1.3×10⁻⁶ずれて許容差を超えた。

## 原因

倍精度の参照と単精度のGPUでは、各項の丸めの段階から値が違う。
多くの項が打ち消し合って0に近くなる勾配の成分では、各項の絶対誤差は小さくても、和の相対誤差が大きくなる。
AdamWの更新は勾配を2次のモーメントの平方根で割った比で決まるので、その相対誤差が、学習率に比例する大きさの更新の差へ拡大される。

## 以後の規則

CPUの参照実装は、順伝播と局面ごとの逆伝播の積をGPUのカーネルと同じ演算の順の単精度で計算し、局面方向の和（後段の重みとバイアスの勾配、第1層とPSQTの行ごとの勾配、損失の和）だけを倍精度で足す。
GPUも局面方向の和を倍精度で固定の木の順に足し、カーネルの内側の積は単精度のままにする（[倍精度の教訓](no-f64-in-gpu-kernel-inner-loops.md)、[縮約の順序の教訓](define-reduction-order-on-the-gpu.md)）。
有限差分の試験で単精度の精度が足りなければ、試験の中にだけ倍精度の補助の計算を置く。
許容差は緩めない。

## 出典

- [tatara化とHalfKP_vmの設計書](../plans/nnue-tatara-framework-and-halfkp-vm.md)のフェーズ3
- `crates/minase-trainer/src/nnue/layerstack/reference.rs`と`layerstack/gpu/tests.rs`の`gpu_one_three_updates_reproducibility_and_snapshot_resume`
