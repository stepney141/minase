// Columns: direct weights 2U, accumulator biases units, direct bias 1,
// then end weights 2U and bias 1, W1 hidden*2U, b1 hidden,
// w2 hidden, w2_end hidden*phase2. Absent sections occupy zero columns.
// Each block scans at most 128 consecutive positions in f32. The resulting
// partial is stored as f64; reduce_partials adds blocks in ascending order.
extern "C" __global__ void gradient_partials(
    const float* d_acc, const float* d_output, const float* d_output_bias,
    const float* d_output_end, const float* d_output_bias_end, const float* d_hidden_weights,
    const float* d_hidden_biases, const float* d_hidden_output, const float* d_hidden_output_end,
    double* partials, unsigned B, unsigned H, unsigned units, unsigned chunk,
    unsigned phase2, unsigned hidden) {
    unsigned end_weights = 3 * units + 1;
    unsigned end_bias = end_weights + phase2 * 2 * units;
    unsigned weights = end_bias + phase2;
    unsigned biases = weights + hidden * 2 * units;
    unsigned output = biases + hidden;
    unsigned output_end = output + hidden;
    unsigned count = output_end + phase2 * hidden;
    unsigned end = min(B, (blockIdx.x + 1) * chunk);
    for (unsigned j = threadIdx.x; j < count; j += blockDim.x) {
        float sum = 0;
        for (unsigned b = blockIdx.x * chunk; b < end; ++b) {
            unsigned long long offset = (unsigned long long)b * 2 * H;
            if (j < 2 * units) {
                for (unsigned k = j % units; k < H; k += units)
                    sum += d_output[offset + (j / units) * H + k];
            } else if (j < 3 * units) {
                for (unsigned v = 0; v < 2; ++v)
                    for (unsigned k = j - 2 * units; k < H; k += units)
                        sum += d_acc[offset + v * H + k];
            } else if (j < end_weights) {
                sum += d_output_bias[b];
            } else if (j < end_bias) {
                unsigned i = j - end_weights;
                for (unsigned k = i % units; k < H; k += units)
                    sum += d_output_end[offset + (i / units) * H + k];
            } else if (j < weights) {
                sum += d_output_bias_end[b];
            } else if (j < biases) {
                sum += d_hidden_weights[(unsigned long long)b * hidden * 2 * units + j - weights];
            } else if (j < output) {
                sum += d_hidden_biases[b * hidden + j - biases];
            } else if (j < output_end) {
                sum += d_hidden_output[b * hidden + j - output];
            } else {
                sum += d_hidden_output_end[b * hidden + j - output_end];
            }
        }
        partials[blockIdx.x * count + j] = (double)sum;
    }
}

// Training loss has one column; validation has ten statistics columns.
extern "C" __global__ void reduce_partials(
    const double* partials, double* sums, unsigned blocks, unsigned values) {
    unsigned j = blockIdx.x * blockDim.x + threadIdx.x;
    if (j >= values) return;
    double sum = 0;
    for (unsigned b = 0; b < blocks; ++b) sum += partials[b * values + j];
    sums[j] = sum;
}
