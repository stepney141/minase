// Keep the f32 moment operations separate, as in the CPU reference. NVRTC is
// compiled with --fmad=false; the bias-corrected quotient uses f64.
__device__ float adamw_value(float value, float gradient, float* moment,
                           float* variance, unsigned long long step,
                           double lr, double decay) {
    value *= (float)(1.0 - lr * decay);
    *moment += (gradient - *moment) * 0.1f;
    *variance = *variance * 0.999f + (gradient * gradient) * 0.001f;
    double corrected_m = (double)*moment / (1.0 - pow(0.9, (double)step));
    double corrected_v = (double)*variance / (1.0 - pow(0.999, (double)step));
    return value - (float)(lr * corrected_m / (sqrt(corrected_v) + 1e-8));
}

// Linear rows and the canonical first layer of P. Every observed element is
// updated, including elements whose gradient is zero in this batch.
extern "C" __global__ void optimizer_rows(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, float* expanded, unsigned count,
    unsigned row_width, float lower, float upper, unsigned long long step,
    double lr, double decay) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    if (observed[i / row_width]) {
        float value = adamw_value(values[i], gradient[i], moment + i,
                                 variance + i, step, lr, decay);
        values[i] = fminf(upper, fmaxf(lower, value));
    }
    expanded[i] = values[i];
}

// A separate launch establishes the virtual-row update before W reads V.
extern "C" __global__ void optimizer_virtual(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, unsigned count, unsigned H,
    unsigned long long step, double lr, double decay) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count || !observed[i / H]) return;
    float value = adamw_value(values[i], gradient[i], moment + i,
                             variance + i, step, lr, decay);
    values[i] = fminf(8.0f, fmaxf(-8.0f, value));
}

extern "C" __global__ void optimizer_first(
    float* values, const float* gradient, float* moment, float* variance,
    const unsigned char* observed, const float* virtual_rows, float* expanded,
    unsigned count, unsigned H, unsigned long long step, double lr,
    double decay) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
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
                                 variance + i, step, lr, decay);
        values[i] = fminf(8.0f - v, fmaxf(-8.0f - v, value));
    }
    // Unobserved W stays fixed, but C must still inherit the current V.
    expanded[i] = values[i] + v;
}

// gradient_sums already contains the sum of both P halves. Keep one free
// parameter per unit and expand both copies from the same updated value.
extern "C" __global__ void optimizer_head(
    float* values, const double* gradient_sums, float* moment, float* variance,
    float* expanded, unsigned gradient_offset, unsigned count, unsigned units,
    unsigned H, float limit, unsigned long long step, double lr,
    double decay) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= count) return;
    float gradient = (float)gradient_sums[gradient_offset + i];
    float value = adamw_value(values[i], gradient, moment + i,
                             variance + i, step, lr, decay);
    value = fminf(limit, fmaxf(-limit, value));
    values[i] = value;
    for (unsigned j = i % units; j < H; j += units)
        expanded[i / units * H + j] = value;
}
