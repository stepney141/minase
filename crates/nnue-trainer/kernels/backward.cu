// Array layouts and launch contracts are documented in ../README.md.
// Reverse-index words match data::PackedOccurrence: position:22, view:1,
// column:8, rotation:1, from the most significant to least significant bit.
extern "C" __global__ void first_layer_backward(
    const unsigned* first_starts, const unsigned* first_entries,
    const unsigned* features, const float* weights, const float* acc,
    const float* output, const float* d_r, const float* d_acc,
    float* d_first, unsigned rows, unsigned H, unsigned p) {
    unsigned long long t = (unsigned long long)blockIdx.x * blockDim.x + threadIdx.x;
    unsigned groups = (H + 3) / 4;
    if (t >= (unsigned long long)rows * groups) return;
    unsigned row = t / groups;
    unsigned first = (t % groups) * 4;
    double sums[4] = {0, 0, 0, 0};
    // HalfKP's inverse index omits its padding row, so do not read its bounds.
    unsigned begin = row + 1 < rows ? first_starts[row] : 0;
    unsigned end = row + 1 < rows ? first_starts[row + 1] : 0;
    for (unsigned e = begin; e < end; ++e) {
        unsigned word = first_entries[e];
        unsigned b = word >> 10;
        unsigned v = (word >> 9) & 1;
        unsigned col = (word >> 1) & 255;
        unsigned bv = b * 2 + v;
        float d = col < 144 ? d_r[(unsigned long long)b * 144 + col] : 0.0f;
        unsigned feature = features[(unsigned long long)bv * 145 + col];
        for (unsigned k = 0; k < 4 && first + k < H; ++k) {
            unsigned j = first + k;
            unsigned unit = p && (word & 1) ? (j + H / 2) % H : j;
            unsigned long long i = (unsigned long long)bv * H + unit;
            float removed = acc[i] - feature_weight(weights, feature, unit, H, p);
            // Inverse rotation applies to A, the accumulator, and output weights.
            sums[k] += (double)d_acc[i]
                     - (double)d * (double)output[v * H + unit]
                       * (double)derivative(removed);
        }
    }
    for (unsigned k = 0; k < 4 && first + k < H; ++k) {
        // Empty ranges, including the padding row, are explicitly zeroed.
        d_first[(unsigned long long)row * H + first + k] = (float)sums[k];
    }
}

extern "C" __global__ void linear_backward(
    const unsigned* linear_starts, const unsigned* linear_entries,
    const unsigned char* counts, const float* d_r, const float* c,
    float* d_linear, unsigned rows, float K) {
    unsigned row = blockIdx.x * blockDim.x + threadIdx.x;
    if (row >= rows) return;
    double sums[2] = {0, 0};
    for (unsigned e = linear_starts[row]; e < linear_starts[row + 1]; ++e) {
        unsigned word = linear_entries[e];
        unsigned b = word >> 10;
        unsigned col = (word >> 1) & 255;
        float d = col < 144 ? d_r[(unsigned long long)b * 144 + col] : 0.0f;
        float phi = (float)min(90, max(0, (int)counts[b] - 3)) / 90.0f;
        float u[2] = {phi, 1.0f - phi};
        for (unsigned endpoint = 0; endpoint < 2; ++endpoint) {
            sums[endpoint] += (double)c[b * 2 + endpoint]
                            - (double)d * (double)u[endpoint] / (double)K;
        }
    }
    d_linear[row * 2] = (float)sums[0];
    d_linear[row * 2 + 1] = (float)sums[1];
}

extern "C" __global__ void reduce_virtual(
    const float* d_first, float* d_virtual, unsigned H) {
    const unsigned feature_count = 13680;
    unsigned long long t = (unsigned long long)blockIdx.x * blockDim.x + threadIdx.x;
    unsigned groups = (H + 3) / 4;
    if (t >= (unsigned long long)(feature_count + 1) * groups) return;
    unsigned base = t / groups;
    unsigned first = (t % groups) * 4;
    double sums[4] = {0, 0, 0, 0};
    if (base < feature_count) {
        for (unsigned bucket = 0; bucket < 8; ++bucket) {
            unsigned long long offset = (unsigned long long)(bucket * feature_count + base) * H;
            for (unsigned k = 0; k < 4 && first + k < H; ++k) {
                sums[k] += (double)d_first[offset + first + k];
            }
        }
    }
    for (unsigned k = 0; k < 4 && first + k < H; ++k) {
        d_virtual[(unsigned long long)base * H + first + k] = (float)sums[k];
    }
}
