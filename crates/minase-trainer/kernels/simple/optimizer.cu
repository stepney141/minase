// All active elements update, including unobserved rows, as in CPU AdamW.
// Step constants are computed once in f64 on the host and rounded to f32.
// Element arithmetic matches the Simple CPU reference, without fused products.
extern "C" __global__ void simple_adamw(
    float* weights, const float* gradients, float* moment, float* variance,
    unsigned offset, unsigned count, float limit, unsigned bounded,
    float correction1, float correction2, float lr, float decay,
    unsigned* nonfinite) {
    unsigned j = blockIdx.x * 128 + threadIdx.x;
    if (j >= count) return;
    unsigned i = offset + j;
    float g = gradients[i], m = moment[i], v = variance[i];
    float value = weights[i] * decay;
    m += (g - m) * 0.1f;
    v = v * 0.999f + (g * g) * 0.001f;
    value -= lr * (m / correction1) / (sqrtf(v / correction2) + 1e-8f);
    if (!isfinite(g) || !isfinite(value) || !isfinite(m) || !isfinite(v)) {
        atomicOr(nonfinite, 2u);
        return;
    }
    weights[i] = bounded ? fminf(limit, fmaxf(-limit, value)) : value;
    moment[i] = m;
    variance[i] = v;
}
