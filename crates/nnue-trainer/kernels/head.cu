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
__device__ float nonlinear_output(const float* acc, const float* output,
                                 float bias, unsigned H, unsigned p) {
    unsigned units = p ? H / 2 : H;
    float views[2] = {0, 0};
    for (unsigned v = 0; v < 2; ++v) {
        for (unsigned j = 0; j < units; ++j) {
            float a = screlu(acc[v * H + j]);
            if (p) a += screlu(acc[v * H + j + units]);
            views[v] += a * output[v * H + j];
        }
    }
    return (views[0] + views[1]) + bias;
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
    const float* removal_linear, float* f, float* h, float* logits,
    float* loss, float* g, float* d_acc, float* d_output, float* c,
    double* loss_partials, unsigned B, unsigned H, unsigned p, float K) {
    extern __shared__ double shared[];
    unsigned b = blockIdx.x * blockDim.x + threadIdx.x;
    float l = 0;
    if (b < B) {
        unsigned long long offset = (unsigned long long)b * 2 * H;
        float u = phase(counts[b]);
        float nonlinear = nonlinear_output(acc + offset, output, bias[0], H, p);
        float x = logit_value(lin + b * 2, nonlinear, u, K);
        float grad = (1.0f / (1.0f + expf(-x)) - targets[b]) / (float)B;
        logits[b] = x;
        l = bce(x, targets[b]);
        loss[b] = l;
        g[b] = grad;
        for (unsigned vj = 0; vj < 2 * H; ++vj) {
            unsigned long long i = offset + vj;
            float a = screlu(acc[i]);
            float dh = derivative(acc[i]);
            f[i] = a;
            h[i] = dh;
            // CPU reference computes these small products in double precision.
            double difference = (double)grad - (double)S[b];
            d_acc[i] = (float)(difference * (double)output[vj] * (double)dh + (double)removal_acc[i]);
            d_output[i] = (float)(difference * (double)a + (double)removal_output[i]);
        }
        c[b * 2] = grad * u / K + removal_linear[b * 2];
        c[b * 2 + 1] = grad * (1.0f - u) / K + removal_linear[b * 2 + 1];
    }
    shared[threadIdx.x] = (double)l;
    __syncthreads();
    if (threadIdx.x == 0) {
        double sum = 0;
        for (unsigned t = 0; t < blockDim.x; ++t) sum += shared[t];
        loss_partials[blockIdx.x] = sum;
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
    unsigned b = blockIdx.x * blockDim.x + threadIdx.x;
    double values[7] = {0, 0, 0, 0, 0, 0, 0};
    if (b < B) {
        const float* a = acc + (unsigned long long)b * 2 * H;
        float nonlinear = nonlinear_output(a, output, bias[0], H, p);
        float x = logit_value(lin + b * 2, nonlinear, phase(counts[b]), K);
        values[0] = (double)bce(x, targets[b]);
        values[1] = outcomes[b] != 1 && ((x > 0) == (outcomes[b] == 2));
        values[2] = outcomes[b] != 1;
        values[3] = (double)nonlinear * (double)K;
        values[4] = values[3] * values[3];
        for (unsigned j = 0; j < 2 * H; ++j) {
            values[5] += a[j] <= 0;
            values[6] += a[j] >= 1;
        }
    }
    for (unsigned j = 0; j < 7; ++j) shared[j * blockDim.x + threadIdx.x] = values[j];
    __syncthreads();
    if (threadIdx.x < 7) {
        double sum = 0;
        for (unsigned t = 0; t < blockDim.x; ++t) sum += shared[threadIdx.x * blockDim.x + t];
        partials[blockIdx.x * 7 + threadIdx.x] = sum;
    }
}
