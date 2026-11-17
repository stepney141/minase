// All active elements update, including unobserved rows, as in CPU AdamW.
// Bias correction denominators are computed once on the host, then divided
// here (not multiplied by reciprocals) to preserve the reference arithmetic.
extern "C" __global__ void ls_adamw(
    float* weights, const float* gradients, float* moment, float* variance,
    unsigned offset, unsigned count, float limit, unsigned bounded,
    double correction1, double correction2, double lr, float decay,
    unsigned* nonfinite) {
    unsigned j = blockIdx.x * 128 + threadIdx.x;
    if (j >= count) return;
    unsigned i = offset + j;
    float g = gradients[i], m = moment[i], v = variance[i];
    float value = weights[i] * decay;
    m += (g - m) * 0.1f;
    v = v * 0.999f + (g * g) * 0.001f;
    value -= (float)(lr * ((double)m / correction1) / (sqrt((double)v / correction2) + 1e-8));
    if (!isfinite(g) || !isfinite(value) || !isfinite(m) || !isfinite(v)) {
        atomicOr(nonfinite, 2u);
        return;
    }
    weights[i] = bounded ? fminf(limit, fmaxf(-limit, value)) : value;
    moment[i] = m;
    variance[i] = v;
}
