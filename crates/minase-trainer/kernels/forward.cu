// Array layouts and launch contracts are documented in ../README.md.
__device__ float feature_weight(const float* weights, unsigned feature,
                               unsigned j, unsigned H, unsigned p) {
    if (feature == 0xffffffffu) return 0.0f;
    unsigned row = p ? feature / 2 : feature;
    unsigned unit = p && (feature & 1) ? (j + H / 2) % H : j;
    return weights[(unsigned long long)row * H + unit];
}

extern "C" __global__ void forward_acc(
    const unsigned* features, const float* weights, const float* bias,
    float* acc, unsigned B, unsigned H, unsigned p) {
    unsigned long long t = (unsigned long long)blockIdx.x * blockDim.x + threadIdx.x;
    unsigned groups = (H + 3) / 4;
    if (t >= (unsigned long long)B * 2 * groups) return;
    unsigned bv = t / groups;
    unsigned first = (t % groups) * 4;
    const unsigned* row = features + (unsigned long long)bv * 145;
    float sums[4] = {0, 0, 0, 0};
    // Match reference::feature_sum, including its mirrored-square pairing.
    for (unsigned rank = 0; rank < 12; ++rank) {
        for (unsigned file = 0; file < 6; ++file) {
            unsigned left = row[rank * 12 + file];
            unsigned right = row[rank * 12 + 11 - file];
            for (unsigned k = 0; k < 4 && first + k < H; ++k) {
                sums[k] += feature_weight(weights, left, first + k, H, p)
                         + feature_weight(weights, right, first + k, H, p);
            }
        }
    }
    for (unsigned k = 0; k < 4 && first + k < H; ++k) {
        acc[(unsigned long long)bv * H + first + k] =
            (sums[k] + feature_weight(weights, row[144], first + k, H, p)) + bias[first + k];
    }
}

__device__ float linear_weight(const float* weights, unsigned feature, unsigned e) {
    return feature == 0xffffffffu ? 0.0f : weights[feature * 2 + e];
}

extern "C" __global__ void forward_linear(
    const unsigned* features, const float* weights, float* lin, unsigned B) {
    unsigned t = blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= B * 2) return;
    const unsigned* row = features + (unsigned long long)(t / 2) * 145;
    unsigned e = t % 2;
    float sum = 0;
    for (unsigned rank = 0; rank < 12; ++rank) {
        for (unsigned file = 0; file < 6; ++file) {
            sum += linear_weight(weights, row[rank * 12 + file], e)
                 + linear_weight(weights, row[rank * 12 + 11 - file], e);
        }
    }
    lin[t] = sum + linear_weight(weights, row[144], e);
}
