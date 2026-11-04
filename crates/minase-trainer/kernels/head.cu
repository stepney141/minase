// One warp per position. Forward arithmetic is f32 without fused multiply-add.
// Only the per-position gradient products and loss partials use f64.
__device__ float clipped_relu(float x) {
    return fminf(1.0f, fmaxf(0.0f, x));
}
__device__ double clipped_relu_derivative(float x) {
    return x >= 0.0f && x <= 1.0f ? 1.0 : 0.0;
}
__device__ double dual_square_derivative(float z) {
    return (127.0f / 128.0f * z) * z <= 1.0f ? 2.0 * (127.0 / 128.0) * (double)z : 0.0;
}
__device__ float phase(unsigned char count) {
    return (float)min(90, max(0, (int)count - 2)) / 90.0f;
}
__device__ float head_warp_sum(float value) {
    for (unsigned distance = 16; distance != 0; distance /= 2)
        value += __shfl_down_sync(0xffffffffu, value, distance);
    return __shfl_sync(0xffffffffu, value, 0);
}
__device__ unsigned head_warp_count(unsigned value) {
    for (unsigned distance = 16; distance != 0; distance /= 2)
        value += __shfl_down_sync(0xffffffffu, value, distance);
    return value;
}
__device__ unsigned pair_left(unsigned i, unsigned H) {
    unsigned Q = H / 4;
    return i / Q * 2 * Q + i % Q;
}
__device__ float pairwise(const float* acc, unsigned i, unsigned H) {
    unsigned left = pair_left(i, H);
    return clipped_relu(acc[left]) * clipped_relu(acc[left + H / 4]);
}
// Cache at most 16 folded inputs per lane, once per position, for all 15 W1 rows.
// W1 uses concatenated view indices lane+32*t. The direct dots use that same
// cache, but rotate the second view's lanes to restore its own j=lane+32*t tree.
__device__ float nonlinear_output_warp(
    const float* acc, const float* output, const float* bias,
    const float* w1, const float* b1, const float* w2, const float* b2, const float* v,
    unsigned H, unsigned U, float* local_f, float* z, float* u, float* a, float* y) {
    unsigned lane = threadIdx.x & 31;
    unsigned inputs = 2 * U;
    float views[2] = {0, 0};
    for (unsigned i = lane, t = 0; i < inputs; i += 32, ++t) {
        unsigned view = i / U;
        const float* view_acc = acc + view * H;
        float f = pairwise(view_acc, i % U, H);
        if (U != H / 2) f += pairwise(view_acc, i % U + U, H);
        local_f[t] = f;
        views[view] += output[i] * f;
    }
    float second = __shfl_sync(0xffffffffu, views[1], (lane + U) & 31);
    float direct = head_warp_sum(views[0]) + head_warp_sum(second);
    for (unsigned k = 0; k < 15; ++k) {
        float sum = 0;
        for (unsigned i = lane, t = 0; i < inputs; i += 32, ++t)
            sum += w1[k * inputs + i] * local_f[t];
        z[k] = head_warp_sum(sum) + b1[k];
        u[k] = fminf((127.0f / 128.0f * z[k]) * z[k], 1.0f);
        u[15 + k] = clipped_relu(z[k]);
    }
    // All lanes compute each row in ascending column order, and have the same y.
    float hidden = 0;
    for (unsigned k = 0; k < 32; ++k) {
        float sum = 0;
        for (unsigned i = 0; i < 30; ++i) sum += w2[k * 30 + i] * u[i];
        a[k] = sum + b2[k];
        y[k] = clipped_relu(a[k]);
        hidden += v[k] * y[k];
    }
    return (direct + hidden) + bias[0];
}
__device__ float logit_value(const float* lin, float nonlinear, float phi, float K) {
    return (phi * lin[0] + (1.0f - phi) * lin[1]) / K + nonlinear;
}
__device__ float bce(float x, float target) {
    return fmaxf(x, 0.0f) - x * target + log1pf(expf(-fabsf(x)));
}

extern "C" __global__ void output_backward(
    const float* acc, const float* lin, const unsigned char* counts,
    const float* output, const float* bias, const float* w1, const float* b1,
    const float* w2, const float* b2, const float* v, const float* targets,
    float* f, float* logits, float* loss, float* g, float* d_acc, float* c,
    float* d_output, float* d_output_bias, float* d_hidden1_weights, float* d_hidden1_biases,
    float* d_hidden2_weights, float* d_hidden2_biases, float* d_hidden2_output,
    float* hidden1_z, float* hidden1_u, float* hidden2_a, float* hidden2_y,
    double* loss_partials, unsigned B, unsigned H, unsigned p, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    unsigned U = p ? H / 4 : H / 2;
    unsigned inputs = 2 * U;
    float l = 0;
    if (b < B) {
        unsigned long long offset = (unsigned long long)b * 2 * H;
        const float* position_acc = acc + offset;
        float local_f[16], z[15], u[30], a[32], y[32];
        float nonlinear = nonlinear_output_warp(position_acc, output, bias,
            w1, b1, w2, b2, v, H, U, local_f, z, u, a, y);
        float grad = 0;
        if (lane == 0) {
            float phi = phase(counts[b]);
            float x = logit_value(lin + b * 2, nonlinear, phi, K);
            grad = (1.0f / (1.0f + expf(-x)) - targets[b]) / (float)B;
            logits[b] = x;
            l = bce(x, targets[b]);
            loss[b] = l;
            g[b] = grad;
            c[b * 2] = grad * phi / K;
            c[b * 2 + 1] = grad * (1.0f - phi) / K;
            d_output_bias[b] = grad;
        }
        grad = __shfl_sync(0xffffffffu, grad, 0);
        double dg = (double)grad;
        // Lane k owns da[k]. All lanes participate in the broadcasts, even 30/31.
        double da = (double)v[lane] * clipped_relu_derivative(a[lane]);
        double du = 0;
        for (unsigned k = 0; k < 32; ++k) {
            double d = __shfl_sync(0xffffffffu, da, k);
            if (lane < 30) du += (double)w2[k * 30 + lane] * d;
        }
        double dz[15];
        for (unsigned k = 0; k < 15; ++k) {
            double square = __shfl_sync(0xffffffffu, du, k);
            double relu = __shfl_sync(0xffffffffu, du, 15 + k);
            dz[k] = square * dual_square_derivative(z[k]) + relu * clipped_relu_derivative(z[k]);
        }
        unsigned long long second = (unsigned long long)b * 32 + lane;
        hidden2_a[second] = a[lane];
        hidden2_y[second] = y[lane];
        d_hidden2_biases[second] = (float)(dg * da);
        d_hidden2_output[second] = (float)(dg * (double)y[lane]);
        for (unsigned i = 0; i < 30; ++i)
            d_hidden2_weights[second * 30 + i] = (float)(dg * da * (double)u[i]);
        if (lane < 30) hidden1_u[(unsigned long long)b * 30 + lane] = u[lane];
        if (lane < 15) {
            unsigned long long first = (unsigned long long)b * 15 + lane;
            hidden1_z[first] = z[lane];
            d_hidden1_biases[first] = (float)(dg * dz[lane]);
        }
        for (unsigned i = lane, t = 0; i < inputs; i += 32, ++t) {
            unsigned long long input = (unsigned long long)b * inputs + i;
            f[input] = local_f[t];
            d_output[input] = (float)(dg * (double)local_f[t]);
            double ds = (double)output[i];
            for (unsigned k = 0; k < 15; ++k) {
                d_hidden1_weights[((unsigned long long)b * 15 + k) * inputs + i] =
                    (float)(dg * dz[k] * (double)local_f[t]);
                ds += dz[k] * (double)w1[k * inputs + i];
            }
            // Each folded input owns one pair (HalfKP) or two pairs (P).
            // These pairs partition every accumulator element, so writes are disjoint.
            unsigned view = i / U * H;
            for (unsigned pair = i % U; pair < H / 2; pair += U) {
                unsigned left = view + pair_left(pair, H);
                unsigned right = left + H / 4;
                d_acc[offset + left] = (float)(dg * ds * clipped_relu_derivative(position_acc[left])
                    * (double)clipped_relu(position_acc[right]));
                d_acc[offset + right] = (float)(dg * ds * clipped_relu_derivative(position_acc[right])
                    * (double)clipped_relu(position_acc[left]));
            }
        }
    }
    if (lane == 0) shared[warp] = (double)l;
    __syncthreads();
    if (threadIdx.x == 0) {
        double sum = 0;
        for (unsigned w = 0; w < warps; ++w) sum += shared[w];
        loss_partials[blockIdx.x] = sum;
    }
}

// Same 12 columns as Validation::sums: BCE, correct, decisive, nonlinear cp,
// its square, acc lower/upper, z lower/upper, a lower/upper, both-positive pairs.
extern "C" __global__ void validation_head(
    const float* acc, const float* lin, const unsigned char* counts,
    const unsigned char* outcomes, const float* output, const float* bias,
    const float* w1, const float* b1, const float* w2, const float* b2, const float* v,
    const float* targets, double* partials, unsigned B, unsigned H, unsigned p, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    double values[12] = {};
    if (b < B) {
        const float* position_acc = acc + (unsigned long long)b * 2 * H;
        float local_f[16], z[15], u[30], a[32], y[32];
        float nonlinear = nonlinear_output_warp(position_acc, output, bias,
            w1, b1, w2, b2, v, H, p ? H / 4 : H / 2, local_f, z, u, a, y);
        unsigned lower = 0, upper = 0, positive = 0;
        for (unsigned j = lane; j < 2 * H; j += 32) {
            lower += position_acc[j] <= 0;
            upper += position_acc[j] >= 1;
        }
        for (unsigned i = lane; i < H; i += 32) {
            unsigned left = i / (H / 2) * H + pair_left(i % (H / 2), H);
            positive += position_acc[left] > 0 && position_acc[left + H / 4] > 0;
        }
        lower = head_warp_count(lower);
        upper = head_warp_count(upper);
        positive = head_warp_count(positive);
        unsigned z_lower = head_warp_count(lane < 15 && z[lane] <= 0);
        unsigned z_upper = head_warp_count(lane < 15 && z[lane] >= 1);
        unsigned a_lower = head_warp_count(a[lane] <= 0);
        unsigned a_upper = head_warp_count(a[lane] >= 1);
        if (lane == 0) {
            float x = logit_value(lin + b * 2, nonlinear, phase(counts[b]), K);
            values[0] = (double)bce(x, targets[b]);
            values[1] = outcomes[b] != 1 && ((x > 0) == (outcomes[b] == 2));
            values[2] = outcomes[b] != 1;
            values[3] = (double)nonlinear * (double)K;
            values[4] = values[3] * values[3];
            values[5] = lower;
            values[6] = upper;
            values[7] = z_lower;
            values[8] = z_upper;
            values[9] = a_lower;
            values[10] = a_upper;
            values[11] = positive;
        }
    }
    if (lane == 0)
        for (unsigned j = 0; j < 12; ++j) shared[j * warps + warp] = values[j];
    __syncthreads();
    if (threadIdx.x < 12) {
        double sum = 0;
        for (unsigned w = 0; w < warps; ++w) sum += shared[threadIdx.x * warps + w];
        partials[blockIdx.x * 12 + threadIdx.x] = sum;
    }
}
