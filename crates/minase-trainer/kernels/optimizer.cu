// モーメントの各演算はf32で行い、積和を融合せず、バイアス補正した商はf64で求める。
// 補正係数はホストがf64の逆数で渡す。
// 非有限フラグの値1は勾配の非有限で、未観測行の勾配も検査する。
// 値2は更新後の値、モーメント、または合成重みCの非有限であり、フラグだけをビットORで集める。
// HalfKPの埋め草行109,440ではCに0を書き、optimizer_virtualを別の起動としてoptimizer_firstより先に完了させる。
__device__ void check_gradient(float gradient, unsigned* nonfinite) {
    if (!isfinite(gradient)) atomicOr(nonfinite, 1u);
}

__device__ float adamw_value(float value, float gradient, float* moment,
                           float* variance, double correction1, double correction2,
                           double lr, double decay, unsigned* nonfinite) {
    value *= (float)(1.0 - lr * decay);
    *moment += (gradient - *moment) * 0.1f;
    *variance = *variance * 0.999f + (gradient * gradient) * 0.001f;
    double corrected_m = (double)*moment * correction1;
    double corrected_v = (double)*variance * correction2;
    value -= (float)(lr * corrected_m / (sqrt(corrected_v) + 1e-8));
    // fmin/fmax can hide a NaN. Check the unclipped result and both moments.
    if (!isfinite(value) || !isfinite(*moment) || !isfinite(*variance))
        atomicOr(nonfinite, 2u);
    return value;
}

// Linear rows and the canonical first layer of P. Every observed element is
// updated, including elements whose gradient is zero in this batch.
extern "C" __global__ void optimizer_rows(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, float* expanded, unsigned count,
    unsigned row_width, float lower, float upper, double correction1, double correction2,
    double lr, double decay, unsigned* nonfinite) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    check_gradient(gradient[i], nonfinite);
    if (observed[i / row_width]) {
        float value = adamw_value(values[i], gradient[i], moment + i,
                                 variance + i, correction1, correction2, lr, decay, nonfinite);
        values[i] = fminf(upper, fmaxf(lower, value));
    }
    expanded[i] = values[i];
}

// A separate launch establishes the virtual-row update before W reads V.
extern "C" __global__ void optimizer_virtual(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, unsigned count, unsigned H,
    double correction1, double correction2, double lr, double decay, unsigned* nonfinite) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    check_gradient(gradient[i], nonfinite);
    if (!observed[i / H]) return;
    float value = adamw_value(values[i], gradient[i], moment + i,
                             variance + i, correction1, correction2, lr, decay, nonfinite);
    values[i] = fminf(8.0f, fmaxf(-8.0f, value));
}

extern "C" __global__ void optimizer_first(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, const float* virtual_rows, float* expanded,
    unsigned count, unsigned H, double correction1, double correction2,
    double lr, double decay, unsigned* nonfinite) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    check_gradient(gradient[i], nonfinite);
    unsigned row = i / H;
    // Folded padding is zero, independently of the frozen free parameters.
    if (row == 109440) {
        expanded[i] = 0.0f;
        return;
    }
    unsigned base = row % 13680;
    float v = virtual_rows[base * H + i % H];
    if (observed[row]) {
        float value = adamw_value(values[i], gradient[i], moment + i,
                                 variance + i, correction1, correction2, lr, decay, nonfinite);
        values[i] = fminf(8.0f - v, fmaxf(-8.0f - v, value));
    }
    // Unobserved W stays fixed, but C must still inherit the current V.
    expanded[i] = values[i] + v;
    if (!isfinite(expanded[i])) atomicOr(nonfinite, 2u);
}

// gradient_sums already contains the sum of both P halves. Keep one free
// parameter per unit and expand both copies from the same updated value.
// This also expands every W1 row. Unshared biases and w2 use units=H=1.
// The host passes limits from Model::constrain: W1=127/64, biases=8, outputs=1.98.
extern "C" __global__ void optimizer_head(
    float* values, const double* gradient_sums, float* moment, float* variance,
    float* expanded, unsigned gradient_offset, unsigned count, unsigned units,
    unsigned H, float limit, double correction1, double correction2,
    double lr, double decay, unsigned* nonfinite) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    float gradient = (float)gradient_sums[gradient_offset + i];
    check_gradient(gradient, nonfinite);
    float value = adamw_value(values[i], gradient, moment + i,
                             variance + i, correction1, correction2, lr, decay, nonfinite);
    value = fminf(limit, fmaxf(-limit, value));
    values[i] = value;
    for (unsigned j = i % units; j < H; j += units)
        expanded[i / units * H + j] = value;
}
