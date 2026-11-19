// All parameter offsets follow Parameter::ALL; W and V remain separate.
struct Layout {
    unsigned virt, ftb, w1, b1, w2, b2, w3, b3;
    __device__ Layout(unsigned H, unsigned R, unsigned K) {
        virt = R * H; ftb = virt + K * H; w1 = ftb + H;
        b1 = w1 + 64 * H; w2 = b1 + 32; b2 = w2 + 1024;
        w3 = b2 + 32; b3 = w3 + 32;
    }
};
// Forward: a[2H], x[2H], y[32], h1[32], u[32], h2[32], output.
// Backward: da[2H], dy[32], du[32], gy.
__device__ float clip(float x) { return fminf(1.0f, fmaxf(0.0f, x)); }
__device__ float deriv(float x) { return x > 0.0f && x < 1.0f ? 1.0f : 0.0f; }
__device__ float warp_sum(float x) {
    for (unsigned d = 16; d; d /= 2) x += __shfl_down_sync(0xffffffffu, x, d);
    return x;
}
__device__ double block_sum(double x, double* scratch) {
    unsigned t = threadIdx.x;
    scratch[t] = x;
    __syncthreads();
    for (unsigned d = 64; d; d /= 2) {
        if (t < d) scratch[t] += scratch[t + d];
        __syncthreads();
    }
    return scratch[0];
}
extern "C" __global__ void simple_forward(
    const unsigned* features, const float* weights, float* forward,
    unsigned H, unsigned R, unsigned K, unsigned columns, unsigned* nonfinite) {
    unsigned b = blockIdx.x, t = threadIdx.x, lane = t % 32, warp = t / 32;
    Layout l(H, R, K);
    float* a = forward + (unsigned long long)b * (4 * H + 129);
    float* x = a + 2 * H; float* y = x + 2 * H;
    float* h1 = y + 32; float* u = h1 + 32; float* h2 = u + 32;
    for (unsigned j = t; j < 2 * H; j += 128) {
        unsigned v = j / H, unit = j % H;
        float sum = weights[l.ftb + unit];
        for (unsigned c = 0; c < columns; ++c) {
            unsigned r = features[(unsigned long long)(2 * b + v) * columns + c];
            if (r != 0xffffffffu)
                sum += weights[r * H + unit] + weights[l.virt + (r % K) * H + unit];
        }
        a[j] = sum; x[j] = clip(sum);
        if (!isfinite(sum)) atomicOr(nonfinite, 1u);
    }
    __syncthreads();
    for (unsigned k = warp; k < 32; k += 4) {
        float sum = 0;
        for (unsigned j = lane; j < 2 * H; j += 32)
            sum += weights[l.w1 + k * 2 * H + j] * x[j];
        sum = warp_sum(sum);
        if (!lane) {
            y[k] = weights[l.b1 + k] + sum; h1[k] = clip(y[k]);
            if (!isfinite(y[k])) atomicOr(nonfinite, 1u);
        }
    }
    __syncthreads();
    for (unsigned k = warp; k < 32; k += 4) {
        float sum = warp_sum(weights[l.w2 + k * 32 + lane] * h1[lane]);
        if (!lane) {
            u[k] = weights[l.b2 + k] + sum; h2[k] = clip(u[k]);
            if (!isfinite(u[k])) atomicOr(nonfinite, 1u);
        }
    }
    __syncthreads();
    if (!warp) {
        float sum = warp_sum(weights[l.w3 + lane] * h2[lane]);
        if (!lane) {
            h2[32] = weights[l.b3] + sum;
            if (!isfinite(h2[32])) atomicOr(nonfinite, 1u);
        }
    }
}
// Loss followed by lower/upper saturation counts for FT, L1 and L2.
extern "C" __global__ void simple_statistics(
    const float* forward, const float* targets, double* partials,
    unsigned B, unsigned H, double ratio, unsigned* nonfinite) {
    unsigned b = blockIdx.x * 128 + threadIdx.x, column = blockIdx.y;
    __shared__ double scratch[128];
    double value = 0;
    if (b < B) {
        const float* a = forward + (unsigned long long)b * (4 * H + 129);
        if (!column) {
            double difference = 1.0 / (1.0 + exp(-ratio * (double)a[4 * H + 128])) - targets[b];
            value = difference * difference;
        } else {
            const float* values = column <= 2 ? a : a + 4 * H + (column <= 4 ? 0 : 64);
            unsigned count = column <= 2 ? 2 * H : 32;
            for (unsigned j = 0; j < count; ++j)
                value += column % 2 ? values[j] <= 0 : values[j] >= 1;
        }
        if (!isfinite(value)) atomicOr(nonfinite, 1u);
    }
    double sum = block_sum(value, scratch);
    if (!threadIdx.x) partials[blockIdx.x * 7 + column] = sum;
}
extern "C" __global__ void simple_statistics_reduce(
    const double* partials, double* sums, unsigned blocks) {
    __shared__ double scratch[128];
    double sum = 0;
    for (unsigned b = threadIdx.x; b < blocks; b += 128) sum += partials[b * 7 + blockIdx.x];
    sum = block_sum(sum, scratch);
    if (!threadIdx.x) sums[blockIdx.x] = sum;
}
