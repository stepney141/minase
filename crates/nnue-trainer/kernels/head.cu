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
// Products are formed by adjacent lanes, then added by lane zero in unit order.
// Keeping the CPU's f32 addition order also preserves logits near zero.
__device__ float head_ordered_warp_sum(float value, float sum, unsigned count) {
    for (unsigned source = 0; source < count; ++source) {
        float term = __shfl_sync(0xffffffffu, value, source);
        if ((threadIdx.x & 31) == 0) sum += term;
    }
    return sum;
}
__device__ unsigned head_warp_count(unsigned value) {
    for (unsigned distance = 16; distance != 0; distance /= 2)
        value += __shfl_down_sync(0xffffffffu, value, distance);
    return value;
}
// All 32 lanes of a warp must call this function for the same position.
__device__ float nonlinear_output_warp(const float* acc, const float* output,
                                      float bias, unsigned H, unsigned p) {
    unsigned lane = threadIdx.x & 31;
    unsigned units = p ? H / 2 : H;
    float views[2] = {0, 0};
    for (unsigned v = 0; v < 2; ++v) {
        for (unsigned base = 0; base < units; base += 32) {
            unsigned j = base + lane;
            float term = 0;
            if (j < units) {
                float a = screlu(acc[v * H + j]);
                if (p) a += screlu(acc[v * H + j + units]);
                term = a * output[v * H + j];
            }
            views[v] = head_ordered_warp_sum(term, views[v], min(32u, units - base));
        }
    }
    float result = (views[0] + views[1]) + bias;
    return __shfl_sync(0xffffffffu, result, 0);
}
__device__ float logit_value(const float* lin, float nonlinear, float u, float K) {
    return (u * lin[0] + (1.0f - u) * lin[1]) / K + nonlinear;
}
__device__ float bce(float x, float target) {
    return fmaxf(x, 0.0f) - x * target + log1pf(expf(-fabsf(x)));
}

extern "C" __global__ void output_backward(
    const float* acc, const float* lin, const unsigned char* counts,
    const float* output, const float* bias, const float* targets,
    const float* S, const float* removal_acc, const float* removal_output,
    const float* removal_linear, const double* removal_loss,
    float* f, float* h, float* logits,
    float* loss, float* g, float* d_acc, float* d_output, float* c,
    double* loss_partials, unsigned B, unsigned H, unsigned p, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    float l = 0;
    if (b < B) {
        unsigned long long offset = (unsigned long long)b * 2 * H;
        float nonlinear = nonlinear_output_warp(acc + offset, output, bias[0], H, p);
        float grad = 0;
        if (lane == 0) {
            float u = phase(counts[b]);
            float x = logit_value(lin + b * 2, nonlinear, u, K);
            grad = (1.0f / (1.0f + expf(-x)) - targets[b]) / (float)B;
            logits[b] = x;
            l = bce(x, targets[b]);
            loss[b] = l;
            g[b] = grad;
            c[b * 2] = grad * u / K + removal_linear[b * 2];
            c[b * 2 + 1] = grad * (1.0f - u) / K + removal_linear[b * 2 + 1];
        }
        grad = __shfl_sync(0xffffffffu, grad, 0);
        // CPU reference computes these small products in double precision.
        double difference = (double)grad - (double)S[b];
        for (unsigned vj = lane; vj < 2 * H; vj += 32) {
            unsigned long long i = offset + vj;
            float a = screlu(acc[i]);
            float dh = derivative(acc[i]);
            f[i] = a;
            h[i] = dh;
            d_acc[i] = (float)(difference * (double)output[vj] * (double)dh + (double)removal_acc[i]);
            d_output[i] = (float)(difference * (double)a + (double)removal_output[i]);
        }
    }
    if (lane == 0) {
        shared[warp] = (double)l;
        shared[warps + warp] = b < B ? removal_loss[b] : 0.0;
    }
    __syncthreads();
    if (threadIdx.x < 2) {
        double sum = 0;
        for (unsigned w = 0; w < warps; ++w)
            sum += shared[threadIdx.x * warps + w];
        loss_partials[blockIdx.x * 2 + threadIdx.x] = sum;
    }
}

// Seven columns: BCE sum, correct, decisive, nonlinear cp sum, square sum,
// acc <= 0 count, acc >= 1 count. Draws are excluded from sign accuracy.
extern "C" __global__ void validation_head(
    const float* acc, const float* lin, const unsigned char* counts,
    const unsigned char* outcomes, const float* output, const float* bias,
    const float* targets, double* partials,
    unsigned B, unsigned H, unsigned p, float K) {
    extern __shared__ double shared[];
    unsigned lane = threadIdx.x & 31;
    unsigned warp = threadIdx.x / 32;
    unsigned warps = blockDim.x / 32;
    unsigned b = blockIdx.x * warps + warp;
    double values[7] = {0, 0, 0, 0, 0, 0, 0};
    if (b < B) {
        const float* a = acc + (unsigned long long)b * 2 * H;
        float nonlinear = nonlinear_output_warp(a, output, bias[0], H, p);
        unsigned lower = 0;
        unsigned upper = 0;
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
        }
    }
    if (lane == 0)
        for (unsigned j = 0; j < 7; ++j) shared[j * warps + warp] = values[j];
    __syncthreads();
    if (threadIdx.x < 7) {
        double sum = 0;
        for (unsigned w = 0; w < warps; ++w) sum += shared[threadIdx.x * warps + w];
        partials[blockIdx.x * 7 + threadIdx.x] = sum;
    }
}
