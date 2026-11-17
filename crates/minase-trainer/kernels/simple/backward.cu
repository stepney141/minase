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
// Dense gradients: f32 products, f64 fixed trees across positions.
extern "C" __global__ void simple_head_partials(
    const float* forward, const float* backward, double* partials,
    unsigned B, unsigned H, unsigned columns) {
    unsigned b = blockIdx.y * 128 + threadIdx.x, c = blockIdx.x;
    __shared__ double scratch[128];
    double value = 0;
    if (b < B) {
        const float* a = forward + (unsigned long long)b * (4 * H + 129);
        const float* x = a + 2 * H; const float* h1 = a + 4 * H + 32;
        const float* h2 = h1 + 64;
        const float* da = backward + (unsigned long long)b * (2 * H + 65);
        const float* dy = da + 2 * H; const float* du = dy + 32; float gy = du[32];
        if (c < H) value = (double)da[c] + da[c + H];
        else {
            c -= H;
            if (c < 64 * H) value = dy[c / (2 * H)] * x[c % (2 * H)];
            else if ((c -= 64 * H) < 32) value = dy[c];
            else if ((c -= 32) < 1024) value = du[c / 32] * h1[c % 32];
            else if ((c -= 1024) < 32) value = du[c];
            else if ((c -= 32) < 32) value = gy * h2[c];
            else value = gy;
        }
    }
    double sum = block_sum(value, scratch);
    if (!threadIdx.x) partials[(unsigned long long)blockIdx.y * columns + blockIdx.x] = sum;
}
extern "C" __global__ void simple_head_reduce(
    const double* partials, float* gradients, unsigned blocks,
    unsigned H, unsigned R, unsigned K, unsigned columns, unsigned* nonfinite) {
    __shared__ double scratch[128];
    double sum = 0;
    for (unsigned b = threadIdx.x; b < blocks; b += 128)
        sum += partials[(unsigned long long)b * columns + blockIdx.x];
    sum = block_sum(sum, scratch);
    if (!threadIdx.x) {
        float value = (float)sum;
        if (!isfinite(value)) atomicOr(nonfinite, 1u);
        Layout l(H, R, K);
        gradients[l.ftb + blockIdx.x] = value;
    }
}
// One block owns a real row. Each warp sums its strided occurrences in f64,
// then applies a fixed tree; no segment-by-width temporary is allocated.
extern "C" __global__ void simple_sparse_reduce(
    const unsigned* starts, const unsigned* offsets, const unsigned* entries,
    const float* backward, float* gradients, unsigned H, unsigned* nonfinite) {
    unsigned row = blockIdx.x, lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    unsigned begin = offsets[starts[row]], end = offsets[starts[row + 1]];
    for (unsigned j = warp; j < H; j += 4) {
        double sum = 0;
        for (unsigned e = begin + lane; e < end; e += 32) {
            unsigned word = entries[e], b = word >> 10, v = (word >> 9) & 1;
            sum += (double)backward[(unsigned long long)b * (2 * H + 65) + v * H + j];
        }
        for (unsigned d = 16; d; d /= 2) sum += __shfl_down_sync(0xffffffffu, sum, d);
        if (!lane) {
            float value = (float)sum;
            if (!isfinite(value)) atomicOr(nonfinite, 1u);
            gradients[row * H + j] = value;
        }
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
