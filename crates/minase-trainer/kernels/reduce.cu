// Each block scans a consecutive range of positions in a fixed order.
// Columns: output weights [2, units], biases [units], output bias [1].
extern "C" __global__ void gradient_partials(
    const float* d_acc, const float* d_output, const float* g,
    double* partials, unsigned B, unsigned H, unsigned units, unsigned chunk) {
    unsigned count = 3 * units + 1;
    unsigned end = min(B, (blockIdx.x + 1) * chunk);
    for (unsigned j = threadIdx.x; j < count; j += blockDim.x) {
        double sum = 0;
        for (unsigned b = blockIdx.x * chunk; b < end; ++b) {
            unsigned long long offset = (unsigned long long)b * 2 * H;
            if (j < 2 * units) {
                for (unsigned k = j % units; k < H; k += units)
                    sum += (double)d_output[offset + (j / units) * H + k];
            } else if (j < 3 * units) {
                for (unsigned v = 0; v < 2; ++v)
                    for (unsigned k = j - 2 * units; k < H; k += units)
                        sum += (double)d_acc[offset + v * H + k];
            } else {
                sum += (double)g[b];
            }
        }
        partials[blockIdx.x * count + j] = sum;
    }
}

// Training loss has one BCE column; validation has seven statistics columns.
extern "C" __global__ void reduce_partials(
    const double* partials, double* sums, unsigned blocks, unsigned values) {
    unsigned j = blockIdx.x * blockDim.x + threadIdx.x;
    if (j >= values) return;
    double sum = 0;
    for (unsigned b = 0; b < blocks; ++b) sum += partials[b * values + j];
    sums[j] = sum;
}
