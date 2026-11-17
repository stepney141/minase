// Parameters are contiguous in Parameter::ALL order; all offsets depend on H/R.
struct Layout {
    unsigned ftb, ps, w1, b1, ws, bs, w2, b2, w3, b3;
    __device__ Layout(unsigned H, unsigned R) {
        ftb = R * H; ps = ftb + H / 2; w1 = ps + R;
        b1 = w1 + 16 * H; ws = b1 + 16; bs = ws + 16 * H;
        w2 = bs + 16; b2 = w2 + 960; w3 = b2 + 32; b3 = w3 + 32;
    }
};
// Per-position forward: a[2H], p[H], y[16], z[30], u[32], n, psqt[2], Y.
// Backward: da[2H], dy[16], du[32], gY. No per-position weight gradients.
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

extern "C" __global__ void ls_forward(
    const unsigned* features, const float* weights, float* forward,
    unsigned H, unsigned R, unsigned columns, unsigned p, unsigned* nonfinite) {
    unsigned b = blockIdx.x, t = threadIdx.x, lane = t % 32, warp = t / 32;
    Layout l(H, R);
    float* a = forward + (unsigned long long)b * (3 * H + 82);
    float* product = a + 2 * H;
    float* y = product + H; float* z = y + 16; float* u = z + 30;
    float* n = u + 32; float* psqt = n + 1;
    for (unsigned j = t; j < 2 * H; j += 128) {
        unsigned v = j / H, unit = j % H;
        float sum = weights[l.ftb + unit % (H / 2)];
        for (unsigned c = 0; c < columns; ++c) {
            unsigned f = features[(unsigned long long)(2 * b + v) * columns + c];
            if (f == 0xffffffffu) continue;
            unsigned row = p ? f / 2 : f;
            unsigned k = p && (f & 1) ? (unit + H / 2) % H : unit;
            sum += weights[row * H + k];
        }
        a[j] = sum;
        if (!isfinite(sum)) atomicOr(nonfinite, 1u);
    }
    // PSQT is a small scalar sum. Compensated f32 avoids losing the residual
    // when large positive and negative piece values cancel.
    if (t < 2) {
        float sum = 0, correction = 0;
        for (unsigned c = 0; c < columns; ++c) {
            unsigned f = features[(unsigned long long)(2 * b + t) * columns + c];
            if (f == 0xffffffffu) continue;
            float x = weights[l.ps + (p ? f / 2 : f)], next = sum + x;
            correction += fabsf(sum) >= fabsf(x) ? (sum - next) + x : (x - next) + sum;
            sum = next;
        }
        psqt[t] = sum + correction;
        if (!isfinite(psqt[t])) atomicOr(nonfinite, 1u);
    }
    __syncthreads();
    for (unsigned j = t; j < H; j += 128) {
        unsigned base = (j / (H / 2)) * H + j % (H / 2);
        product[j] = (127.0f / 128.0f) * clip(a[base]) * clip(a[base + H / 2]);
    }
    __syncthreads();
    for (unsigned k = warp; k < 16; k += 4) {
        float sum = 0;
        for (unsigned j = lane; j < H; j += 32)
            sum += (weights[l.w1 + k * H + j] + weights[l.ws + k * H + j]) * product[j];
        sum = warp_sum(sum);
        if (!lane) {
            y[k] = weights[l.b1 + k] + weights[l.bs + k] + sum;
            if (!isfinite(y[k])) atomicOr(nonfinite, 1u);
        }
    }
    __syncthreads();
    if (t < 30) z[t] = t < 15 ? clip((127.0f / 128.0f) * y[t] * y[t]) : clip(y[t - 15]);
    __syncthreads();
    for (unsigned k = warp; k < 32; k += 4) {
        float sum = lane < 30 ? weights[l.w2 + k * 30 + lane] * z[lane] : 0;
        sum = warp_sum(sum);
        if (!lane) {
            u[k] = weights[l.b2 + k] + sum;
            if (!isfinite(u[k])) atomicOr(nonfinite, 1u);
        }
    }
    __syncthreads();
    if (!warp) {
        float sum = warp_sum(weights[l.w3 + lane] * clip(u[lane]));
        if (!lane) {
            *n = weights[l.b3] + y[15] + sum;
            psqt[2] = *n + (psqt[0] - psqt[1]) * 0.5f;
            if (!isfinite(*n) || !isfinite(psqt[2])) atomicOr(nonfinite, 1u);
        }
    }
}

// 14 statistics: the shared Validation's 12 columns, then squared-branch
// lower/upper saturation. Compute one column per block of 128 positions.
extern "C" __global__ void ls_statistics(
    const float* forward, const float* targets, const unsigned char* outcomes,
    double* partials, unsigned B, unsigned H, double ratio, double output_scale,
    unsigned* nonfinite) {
    unsigned b = blockIdx.x * 128 + threadIdx.x, column = blockIdx.y;
    __shared__ double scratch[128];
    double value = 0;
    if (b < B) {
        const float* a = forward + (unsigned long long)b * (3 * H + 82);
        const float* y = a + 3 * H; const float* u = y + 46;
        double output = u[35];
        if (column == 0) {
            double difference = 1.0 / (1.0 + exp(-ratio * output)) - targets[b];
            value = difference * difference;
        } else if (column == 1) value = outcomes[b] != 1 && ((output > 0) == (outcomes[b] == 2));
        else if (column == 2) value = outcomes[b] != 1;
        else if (column == 3) value = (double)u[32] * output_scale;
        else if (column == 4) { double cp = (double)u[32] * output_scale; value = cp * cp; }
        else if (column == 5 || column == 6) {
            unsigned count = 0;
            for (unsigned j = 0; j < 2 * H; ++j) count += column == 5 ? a[j] <= 0 : a[j] >= 1;
            value = count;
        } else if (column == 7 || column == 8 || column >= 12) {
            unsigned count = 0;
            for (unsigned j = 0; j < 15; ++j) {
                float x = column >= 12 ? (127.0f / 128.0f) * y[j] * y[j] : y[j];
                count += (column == 7 || column == 12) ? x <= 0 : x >= 1;
            }
            value = count;
        } else if (column == 9 || column == 10) {
            unsigned count = 0;
            for (unsigned j = 0; j < 32; ++j) count += column == 9 ? u[j] <= 0 : u[j] >= 1;
            value = count;
        } else {
            unsigned count = 0;
            for (unsigned j = 0; j < H; ++j) {
                unsigned base = (j / (H / 2)) * H + j % (H / 2);
                count += a[base] > 0 && a[base + H / 2] > 0;
            }
            value = count;
        }
        if (!isfinite(value)) atomicOr(nonfinite, 1u);
    }
    double sum = block_sum(value, scratch);
    if (!threadIdx.x) partials[blockIdx.x * 14 + column] = sum;
}

extern "C" __global__ void ls_statistics_reduce(
    const double* partials, double* sums, unsigned blocks) {
    __shared__ double scratch[128];
    double sum = 0;
    for (unsigned b = threadIdx.x; b < blocks; b += 128) sum += partials[b * 14 + blockIdx.x];
    sum = block_sum(sum, scratch);
    if (!threadIdx.x) sums[blockIdx.x] = sum;
}
