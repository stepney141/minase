extern "C" __global__ void ls_backward(
    const float* weights, const float* forward, const float* targets,
    float* backward, unsigned B, unsigned H, unsigned R, double ratio,
    unsigned* nonfinite) {
    unsigned b = blockIdx.x, t = threadIdx.x, lane = t % 32, warp = t / 32;
    Layout l(H, R);
    const float* a = forward + (unsigned long long)b * (3 * H + 82);
    const float* y = a + 3 * H; const float* z = y + 16; const float* u = z + 30;
    float* da = backward + (unsigned long long)b * (2 * H + 49);
    float* dy = da + 2 * H; float* du = dy + 16; float* gy = du + 32;
    __shared__ float dz[30];
    if (!t) {
        double sigma = 1.0 / (1.0 + exp(-ratio * (double)u[35]));
        *gy = (float)(2.0 / B * (sigma - targets[b]) * sigma * (1.0 - sigma) * ratio);
        if (!isfinite(*gy)) atomicOr(nonfinite, 1u);
    }
    __syncthreads();
    if (t < 32) du[t] = *gy * weights[l.w3 + t] * deriv(u[t]);
    __syncthreads();
    for (unsigned j = warp; j < 30; j += 4) {
        float sum = warp_sum(du[lane] * weights[l.w2 + lane * 30 + j]);
        if (!lane) dz[j] = sum;
    }
    __syncthreads();
    if (t < 16) dy[t] = t == 15 ? *gy :
        dz[t] * (2.0f * (127.0f / 128.0f) * y[t] * deriv((127.0f / 128.0f) * y[t] * y[t]))
        + dz[t + 15] * deriv(y[t]);
    __syncthreads();
    for (unsigned j = t; j < H; j += 128) {
        float sum = 0;
        for (unsigned k = 0; k < 16; ++k)
            sum += dy[k] * (weights[l.w1 + k * H + j] + weights[l.ws + k * H + j]);
        unsigned base = (j / (H / 2)) * H + j % (H / 2);
        da[base] = sum * (127.0f / 128.0f) * clip(a[base + H / 2]) * deriv(a[base]);
        da[base + H / 2] = sum * (127.0f / 128.0f) * clip(a[base]) * deriv(a[base + H / 2]);
        if (!isfinite(da[base]) || !isfinite(da[base + H / 2])) atomicOr(nonfinite, 1u);
    }
    if (t < 16 && !isfinite(dy[t])) atomicOr(nonfinite, 1u);
    if (t < 32 && !isfinite(du[t])) atomicOr(nonfinite, 1u);
}

// Columns: FT bias H/2, L1 16H+16, L2 960+32, L3 32+1.
// Shared L1 is copied only at final reduction. One block reduces 128 samples
// for one parameter. Scratch is shared memory, never a [B,16,H] tensor.
extern "C" __global__ void ls_head_partials(
    const float* forward, const float* backward, double* partials,
    unsigned B, unsigned H, unsigned columns) {
    unsigned b = blockIdx.x * 128 + threadIdx.x, c = blockIdx.y;
    __shared__ double scratch[128];
    double value = 0;
    if (b < B) {
        const float* a = forward + (unsigned long long)b * (3 * H + 82);
        const float* product = a + 2 * H; const float* z = product + H + 16;
        const float* u = z + 30;
        const float* da = backward + (unsigned long long)b * (2 * H + 49);
        const float* dy = da + 2 * H; const float* du = dy + 16; float gy = du[32];
        if (c < H / 2) value = (double)da[c] + da[c + H / 2] + da[c + H] + da[c + 3 * H / 2];
        else {
            c -= H / 2;
            if (c < 16 * H) value = dy[c / H] * product[c % H];
            else if ((c -= 16 * H) < 16) value = dy[c];
            else if ((c -= 16) < 960) value = du[c / 30] * z[c % 30];
            else if ((c -= 960) < 32) value = du[c];
            else if ((c -= 32) < 32) value = gy * clip(u[c]);
            else value = gy;
        }
    }
    double sum = block_sum(value, scratch);
    if (!threadIdx.x) partials[blockIdx.x * columns + blockIdx.y] = sum;
}

extern "C" __global__ void ls_head_reduce(
    const double* partials, float* gradients, unsigned blocks,
    unsigned H, unsigned R, unsigned columns, unsigned* nonfinite) {
    unsigned c = blockIdx.x;
    __shared__ double scratch[128];
    double sum = 0;
    for (unsigned b = threadIdx.x; b < blocks; b += 128) sum += partials[b * columns + c];
    sum = block_sum(sum, scratch);
    if (threadIdx.x) return;
    float value = (float)sum;
    if (!isfinite(value)) atomicOr(nonfinite, 1u);
    Layout l(H, R);
    if (c < H / 2) gradients[l.ftb + c] = value;
    else {
        c -= H / 2;
        if (c < 16 * H) { gradients[l.w1 + c] = value; gradients[l.ws + c] = value; }
        else if ((c -= 16 * H) < 16) { gradients[l.b1 + c] = value; gradients[l.bs + c] = value; }
        else gradients[l.w2 + c - 16] = value;
    }
}

// The inverse-index segments contain at most 128 occurrences, in a fixed order.
// Products/deltas stay f32; sums across occurrences and stored partials are f64.
extern "C" __global__ void ls_sparse_partials(
    const unsigned* offsets, const unsigned* entries, const float* backward,
    double* partials, unsigned H, unsigned p) {
    unsigned s = blockIdx.x, lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    for (unsigned j = warp; j <= H; j += 4) {
        double sum = 0;
        for (unsigned e = offsets[s] + lane; e < offsets[s + 1]; e += 32) {
            unsigned word = entries[e], b = word >> 10, v = (word >> 9) & 1;
            const float* d = backward + (unsigned long long)b * (2 * H + 49);
            if (j == H) sum += (double)((v ? -0.5f : 0.5f) * d[2 * H + 48]);
            else {
                unsigned unit = p && (word & 1) ? (j + H / 2) % H : j;
                sum += (double)d[v * H + unit];
            }
        }
        for (unsigned distance = 16; distance; distance /= 2)
            sum += __shfl_down_sync(0xffffffffu, sum, distance);
        if (!lane) partials[(unsigned long long)s * (H + 1) + j] = sum;
    }
}

extern "C" __global__ void ls_sparse_reduce(
    const unsigned* starts, const double* partials, float* gradients,
    unsigned H, unsigned R, unsigned* nonfinite) {
    unsigned row = blockIdx.x;
    Layout l(H, R);
    unsigned lane = threadIdx.x % 32, warp = threadIdx.x / 32;
    for (unsigned j = warp; j <= H; j += 4) {
        double sum = 0;
        if (row + 1 < R)
            for (unsigned s = starts[row] + lane; s < starts[row + 1]; s += 32)
                sum += (double)partials[(unsigned long long)s * (H + 1) + j];
        // Fixed 16,8,4,2,1 tree over the already compact segment sums.
        for (unsigned d = 16; d; d /= 2)
            sum += __shfl_down_sync(0xffffffffu, sum, d);
        if (!lane) {
            float value = (float)sum;
            if (!isfinite(value)) atomicOr(nonfinite, 1u);
            gradients[j == H ? l.ps + row : row * H + j] = value;
        }
    }
}
