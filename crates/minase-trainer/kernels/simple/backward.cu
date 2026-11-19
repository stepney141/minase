extern "C" __global__ void simple_backward(
    const float* weights, const float* forward, const float* targets,
    float* backward, unsigned B, unsigned H, unsigned R, unsigned K,
    double ratio, unsigned* nonfinite) {
    unsigned b = blockIdx.x, t = threadIdx.x, lane = t % 32, warp = t / 32;
    Layout l(H, R, K);
    const float* a = forward + (unsigned long long)b * (4 * H + 129);
    const float* y = a + 4 * H; const float* u = y + 64;
    float* da = backward + (unsigned long long)b * (2 * H + 65);
    float* dy = da + 2 * H; float* du = dy + 32; float* gy = du + 32;
    if (!t) {
        double sigma = 1.0 / (1.0 + exp(-ratio * (double)u[64]));
        *gy = (float)(2.0 / B * (sigma - targets[b]) * sigma * (1.0 - sigma) * ratio);
        if (!isfinite(*gy)) atomicOr(nonfinite, 1u);
    }
    __syncthreads();
    if (t < 32) du[t] = *gy * weights[l.w3 + t] * deriv(u[t]);
    __syncthreads();
    for (unsigned j = warp; j < 32; j += 4) {
        float sum = warp_sum(du[lane] * weights[l.w2 + lane * 32 + j]);
        if (!lane) dy[j] = sum * deriv(y[j]);
    }
    __syncthreads();
    for (unsigned j = t; j < 2 * H; j += 128) {
        float sum = 0;
        for (unsigned k = 0; k < 32; ++k) sum += dy[k] * weights[l.w1 + k * 2 * H + j];
        da[j] = sum * deriv(a[j]);
        if (!isfinite(da[j])) atomicOr(nonfinite, 1u);
    }
    if (t < 32 && (!isfinite(dy[t]) || !isfinite(du[t]))) atomicOr(nonfinite, 1u);
}
// Each warp reads adjacent components of one position. Four contiguous
// position ranges are accumulated in f64, then combined by the tree (2, 1).
// Every lane reaches both barriers, including padded columns.
__device__ double column_sum(double value, double* scratch) {
    unsigned t = threadIdx.x, warp = t / 32;
    scratch[t] = value;
    __syncthreads();
    if (warp < 2) scratch[t] += scratch[t + 64];
    __syncthreads();
    if (!warp) scratch[t] += scratch[t + 32];
    __syncthreads();
    return scratch[t % 32];
}
extern "C" __global__ void simple_head_partials(
    const float* forward, const float* backward, double* partials,
    unsigned B, unsigned H, unsigned columns) {
    unsigned lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    unsigned column = blockIdx.x * 32 + lane;
    unsigned begin = blockIdx.y * 128 + warp * 32;
    __shared__ double scratch[128];
    double value = 0;
    if (column < columns) {
        for (unsigned b = begin; b < min(begin + 32, B); ++b) {
            unsigned c = column;
            const float* a = forward + (unsigned long long)b * (4 * H + 129);
            const float* x = a + 2 * H; const float* h1 = a + 4 * H + 32;
            const float* h2 = h1 + 64;
            const float* da = backward + (unsigned long long)b * (2 * H + 65);
            const float* dy = da + 2 * H; const float* du = dy + 32; float gy = du[32];
            if (c < H) value += (double)da[c] + da[c + H];
            else {
                c -= H;
                if (c < 64 * H) value += (double)(dy[c / (2 * H)] * x[c % (2 * H)]);
                else if ((c -= 64 * H) < 32) value += (double)dy[c];
                else if ((c -= 32) < 1024) value += (double)(du[c / 32] * h1[c % 32]);
                else if ((c -= 1024) < 32) value += (double)du[c];
                else if ((c -= 32) < 32) value += (double)(gy * h2[c]);
                else value += (double)gy;
            }
        }
    }
    double sum = column_sum(value, scratch);
    if (!warp && column < columns)
        partials[(unsigned long long)blockIdx.y * columns + column] = sum;
}
extern "C" __global__ void simple_head_reduce(
    const double* partials, float* gradients, unsigned blocks,
    unsigned H, unsigned R, unsigned K, unsigned columns, unsigned* nonfinite) {
    unsigned lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    unsigned column = blockIdx.x * 32 + lane, chunk = (blocks + 3) / 4;
    __shared__ double scratch[128];
    double sum = 0;
    if (column < columns)
        for (unsigned b = warp * chunk; b < min((warp + 1) * chunk, blocks); ++b)
            sum += partials[(unsigned long long)b * columns + column];
    sum = column_sum(sum, scratch);
    if (!warp && column < columns) {
        float value = (float)sum;
        if (!isfinite(value)) atomicOr(nonfinite, 1u);
        Layout l(H, R, K);
        gradients[l.ftb + column] = value;
    }
}
// Launch over existing segments, not all real rows. Only the first segment
// owns the row; the other segments return uniformly before any barrier.
// Empty rows retain the zero written by the host's stream-ordered memset.
extern "C" __global__ void simple_sparse_reduce(
    const unsigned* starts, const unsigned* offsets, const unsigned* entries,
    const unsigned* features, const float* backward, float* gradients,
    unsigned H, unsigned columns,
    unsigned* nonfinite) {
    unsigned segment = blockIdx.x;
    __shared__ double scratch[128];
    // Every segment is nonempty. Its first occurrence identifies the row
    // directly through the original feature array, including lion flags.
    unsigned first = entries[offsets[segment]];
    unsigned b = first >> 10, v = (first >> 9) & 1, c = (first >> 1) & 255;
    unsigned row = features[(unsigned long long)(2 * b + v) * columns + c];
    if (segment != starts[row]) return;
    unsigned lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    unsigned begin = offsets[starts[row]], end = offsets[starts[row + 1]];
    unsigned chunk = (end - begin + 3) / 4;
    for (unsigned tile = 0; tile < H; tile += 32) {
        unsigned j = tile + lane;
        double sum = 0;
        if (j < H) {
            for (unsigned e = begin + warp * chunk; e < min(begin + (warp + 1) * chunk, end); ++e) {
                unsigned word = entries[e], b = word >> 10, v = (word >> 9) & 1;
                sum += (double)backward[(unsigned long long)b * (2 * H + 65) + v * H + j];
            }
        }
        sum = column_sum(sum, scratch);
        if (!warp && j < H) {
            float value = (float)sum;
            if (!isfinite(value)) atomicOr(nonfinite, 1u);
            gradients[row * H + j] = value;
        }
        // The next tile must not overwrite scratch before warp 0 reads it.
        __syncthreads();
    }
}
// A virtual row receives the gradients of every king bucket, in bucket order.
extern "C" __global__ void simple_virtual_reduce(
    float* gradients, unsigned H, unsigned R, unsigned K, unsigned* nonfinite) {
    unsigned row = blockIdx.x;
    for (unsigned j = threadIdx.x; j < H; j += 128) {
        double sum = 0;
        for (unsigned r = row; r < R; r += K) sum += (double)gradients[r * H + j];
        float value = (float)sum;
        if (!isfinite(value)) atomicOr(nonfinite, 1u);
        gradients[R * H + row * H + j] = value;
    }
}
