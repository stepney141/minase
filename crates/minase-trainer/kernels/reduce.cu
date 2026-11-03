// Columns follow HEAD_PARAMETERS: o 2U, accumulator biases bias_units, b 1,
// W1 15*2U, b1 15, W2 32*30, b2 32, v 32. Only accumulator biases need folding.
// Each block scans at most 128 consecutive positions in f32. The resulting
// partial is stored as f64; reduce_partials adds blocks in ascending order.
extern "C" __global__ void gradient_partials(
    const float* d_acc, const float* d_output, const float* d_output_bias,
    const float* d_hidden1_weights, const float* d_hidden1_biases,
    const float* d_hidden2_weights, const float* d_hidden2_biases, const float* d_hidden2_output,
    double* partials, unsigned B, unsigned H, unsigned bias_units, unsigned output_units,
    unsigned chunk) {
    unsigned inputs = 2 * output_units;
    unsigned direct_bias = inputs + bias_units;
    unsigned weights1 = direct_bias + 1;
    unsigned biases1 = weights1 + 15 * inputs;
    unsigned weights2 = biases1 + 15;
    unsigned biases2 = weights2 + 32 * 30;
    unsigned output2 = biases2 + 32;
    unsigned count = output2 + 32;
    unsigned end = min(B, (blockIdx.x + 1) * chunk);
    for (unsigned j = threadIdx.x; j < count; j += blockDim.x) {
        float sum = 0;
        for (unsigned b = blockIdx.x * chunk; b < end; ++b) {
            if (j < inputs) {
                sum += d_output[(unsigned long long)b * inputs + j];
            } else if (j < direct_bias) {
                unsigned long long offset = (unsigned long long)b * 2 * H;
                for (unsigned view = 0; view < 2; ++view)
                    for (unsigned k = j - inputs; k < H; k += bias_units)
                        sum += d_acc[offset + view * H + k];
            } else if (j < weights1) {
                sum += d_output_bias[b];
            } else if (j < biases1) {
                sum += d_hidden1_weights[(unsigned long long)b * 15 * inputs + j - weights1];
            } else if (j < weights2) {
                sum += d_hidden1_biases[(unsigned long long)b * 15 + j - biases1];
            } else if (j < biases2) {
                sum += d_hidden2_weights[(unsigned long long)b * 32 * 30 + j - weights2];
            } else if (j < output2) {
                sum += d_hidden2_biases[(unsigned long long)b * 32 + j - biases2];
            } else {
                sum += d_hidden2_output[(unsigned long long)b * 32 + j - output2];
            }
        }
        partials[(unsigned long long)blockIdx.x * count + j] = (double)sum;
    }
}

// Training loss has one column; validation has twelve statistics columns.
extern "C" __global__ void reduce_partials(
    const double* partials, double* sums, unsigned blocks, unsigned values) {
    unsigned j = blockIdx.x * blockDim.x + threadIdx.x;
    if (j >= values) return;
    double sum = 0;
    for (unsigned b = 0; b < blocks; ++b) sum += partials[b * values + j];
    sums[j] = sum;
}
