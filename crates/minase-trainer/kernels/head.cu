// One warp per position. All lanes participate in the fixed reduction tree.
__device__ float screlu(float x) {
    float c = fminf(1.0f, fmaxf(0.0f, x));
    return c * c;
}
__device__ float derivative(float x) {
    return x >= 0.0f && x <= 1.0f ? 2.0f * x : 0.0f;
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
__device__ float folded_activation(const float* acc, unsigned i, unsigned H, unsigned U) {
    unsigned j = i / U * H + i % U;
    float s = screlu(acc[j]);
    if (U != H) s += screlu(acc[j + U]);
    return s;
}
// Each lane accumulates lane+32*t in f32, then distances 16,8,4,2,1.
// Direct dots reduce each view separately; W1 reduces the concatenated views.
// w2 is added in k order; endpoints are (direct + hidden) + bias.
__device__ float nonlinear_output_warp(
    const float* acc, const float* output, const float* bias,
    const float* output_end, const float* bias_end, const float* w1,
    const float* b1, const float* w2, const float* w2_end,
    unsigned H, unsigned U, unsigned phase2, unsigned hidden, float phi,
    float* z, float* h) {
    unsigned lane = threadIdx.x & 31;
    for (unsigned k = 0; k < hidden; ++k) {
        float sum = 0;
        for (unsigned i = lane; i < 2 * U; i += 32)
            sum += w1[k * 2 * H + i / U * H + i % U] * folded_activation(acc, i, H, U);
        z[k] = head_warp_sum(sum) + b1[k];
        h[k] = fminf(1.0f, fmaxf(0.0f, z[k]));
    }
    float endpoints[2];
    for (unsigned e = 0; e <= phase2; ++e) {
        const float* o = e ? output_end : output;
        const float* w = e ? w2_end : w2;
        float views[2];
        for (unsigned v = 0; v < 2; ++v) {
            float sum = 0;
            for (unsigned j = lane; j < U; j += 32)
                sum += o[v * H + j] * folded_activation(acc, v * U + j, H, U);
            views[v] = head_warp_sum(sum);
        }
        float inner = 0;
        for (unsigned k = 0; k < hidden; ++k) inner += h[k] * w[k];
        endpoints[e] = ((views[0] + views[1]) + inner) + (e ? bias_end[0] : bias[0]);
    }
    return phase2 ? phi * endpoints[0] + (1.0f - phi) * endpoints[1] : endpoints[0];
}
__device__ float logit_value(const float* lin, float nonlinear, float u, float K) {
    return (u * lin[0] + (1.0f - u) * lin[1]) / K + nonlinear;
}
__device__ float bce(float x, float target) {
    return fmaxf(x, 0.0f) - x * target + log1pf(expf(-fabsf(x)));
}

extern "C" __global__ void output_backward(
    const float* acc, const float* lin, const unsigned char* counts,
    const float* output, const float* bias,
    const float* output_end, const float* bias_end, const float* w1,
    const float* b1, const float* w2, const float* w2_end,
    const float* targets, float* f, float* h, float* logits,
    float* loss, float* g, float* d_acc, float* d_output, float* c,
    float* d_output_bias, float* d_output_end, float* d_output_bias_end,
    float* d_hidden_weights, float* d_hidden_biases, float* d_hidden_output, float* d_hidden_output_end,
    float* hidden_z, float* hidden_h,
    double* loss_partials, unsigned B, unsigned H, unsigned p,
    unsigned phase2, unsigned hidden, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    unsigned U = p ? H / 2 : H;
    float l = 0;
    if (b < B) {
        unsigned long long offset = (unsigned long long)b * 2 * H;
        float z[16], activation[16];
        float u = phase(counts[b]);
        float phi = phase2 ? u : 1.0f;
        float end = phase2 ? 1.0f - u : 0.0f;
        float nonlinear = nonlinear_output_warp(acc + offset, output, bias, output_end,
            bias_end, w1, b1, w2, w2_end, H, U, phase2, hidden, u, z, activation);
        float grad = 0;
        if (lane == 0) {
            float x = logit_value(lin + b * 2, nonlinear, u, K);
            grad = (1.0f / (1.0f + expf(-x)) - targets[b]) / (float)B;
            logits[b] = x;
            l = bce(x, targets[b]);
            loss[b] = l;
            g[b] = grad;
            c[b * 2] = grad * u / K;
            c[b * 2 + 1] = grad * (1.0f - u) / K;
            d_output_bias[b] = grad * phi;
            if (phase2) d_output_bias_end[b] = grad * end;
        }
        grad = __shfl_sync(0xffffffffu, grad, 0);
        // Match the CPU reference's f64 derivative and contribution products.
        double dz[16];
        for (unsigned k = 0; k < hidden; ++k) {
            double dh = (double)phi * (double)w2[k];
            if (phase2) dh += (double)end * (double)w2_end[k];
            dz[k] = z[k] >= 0 && z[k] <= 1 ? dh : 0;
            if (lane == k) {
                unsigned i = b * hidden + k;
                hidden_z[i] = z[k];
                hidden_h[i] = activation[k];
                d_hidden_biases[i] = (float)((double)grad * dz[k]);
                d_hidden_output[i] = (float)((double)grad * (double)phi * (double)activation[k]);
                if (phase2) d_hidden_output_end[i] = (float)((double)grad * (double)end * (double)activation[k]);
            }
            for (unsigned i = lane; i < 2 * U; i += 32)
                d_hidden_weights[((unsigned long long)b * hidden + k) * 2 * U + i] =
                    (float)((double)grad * dz[k] * (double)folded_activation(acc + offset, i, H, U));
        }
        for (unsigned vj = lane; vj < 2 * H; vj += 32) {
            unsigned long long i = offset + vj;
            float a = screlu(acc[i]);
            float dh = derivative(acc[i]);
            double ds = (double)phi * (double)output[vj];
            if (phase2) {
                ds += (double)end * (double)output_end[vj];
                d_output_end[i] = (float)((double)grad * (double)end * (double)a);
            }
            for (unsigned k = 0; k < hidden; ++k) ds += dz[k] * (double)w1[k * 2 * H + vj];
            f[i] = a;
            h[i] = dh;
            d_acc[i] = (float)((double)grad * ds * (double)dh);
            d_output[i] = (float)((double)grad * (double)phi * (double)a);
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

// BCE, correct, decisive, nonlinear cp and its square, acc lower/upper,
// hidden lower/upper and hidden element count. Absent hidden layers give zeros.
extern "C" __global__ void validation_head(
    const float* acc, const float* lin, const unsigned char* counts,
    const unsigned char* outcomes, const float* output, const float* bias,
    const float* output_end, const float* bias_end, const float* w1,
    const float* b1, const float* w2, const float* w2_end,
    const float* targets, double* partials,
    unsigned B, unsigned H, unsigned p, unsigned phase2, unsigned hidden, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    double values[10] = {};
    if (b < B) {
        const float* a = acc + (unsigned long long)b * 2 * H;
        float z[16], h[16];
        float nonlinear = nonlinear_output_warp(a, output, bias, output_end, bias_end,
            w1, b1, w2, w2_end, H, p ? H / 2 : H, phase2, hidden, phase(counts[b]), z, h);
        unsigned lower = 0, upper = 0;
        for (unsigned j = lane; j < 2 * H; j += 32) {
            lower += a[j] <= 0;
            upper += a[j] >= 1;
        }
        lower = head_warp_count(lower);
        upper = head_warp_count(upper);
        if (lane == 0) {
            float x = logit_value(lin + b * 2, nonlinear, phase(counts[b]), K);
            values[0] = (double)bce(x, targets[b]);
            values[1] = outcomes[b] != 1 && ((x > 0) == (outcomes[b] == 2));
            values[2] = outcomes[b] != 1;
            values[3] = (double)nonlinear * (double)K;
            values[4] = values[3] * values[3];
            values[5] = lower;
            values[6] = upper;
            for (unsigned k = 0; k < hidden; ++k) {
                values[7] += h[k] <= 0;
                values[8] += h[k] >= 1;
            }
            values[9] = hidden;
        }
    }
    if (lane == 0)
        for (unsigned j = 0; j < 10; ++j) shared[j * warps + warp] = values[j];
    __syncthreads();
    if (threadIdx.x < 10) {
        double sum = 0;
        for (unsigned w = 0; w < warps; ++w) sum += shared[threadIdx.x * warps + w];
        partials[blockIdx.x * 10 + threadIdx.x] = sum;
    }
}
