// One 128-thread block owns one position, with one warp per removal column.
// Scratch is 2,008 bytes, independent of H; accumulator components keep their
// fixed-order sums in double registers.
struct RemovalScratch {
    long long reference_sum[2];
    double linear_sum[2];
    double before_linear;
    double before_e;
    float before_head[2];
    float d[144];
    double penalty[144];
    unsigned char active[144];
    unsigned count_partials[4];
    double sum_partials[4];
    double loss_partials[4];
};

__device__ long long removal_integer(long long numerator) {
    // C++ and Rust both divide signed integers toward zero before clamping.
    long long value = numerator / 720;
    return value < -28999 ? -28999 : (value > 28999 ? 28999 : value);
}

__device__ float removal_head(
    const float* acc, const float* weights, const float* output,
    unsigned feature, unsigned H, unsigned p) {
    unsigned units = p ? H / 2 : H;
    unsigned lane = threadIdx.x % 32;
    float sum = 0;
    for (unsigned base = 0; base < units; base += 32) {
        unsigned j = base + lane;
        float product = 0;
        if (j < units) {
            float activation = screlu(acc[j] - feature_weight(weights, feature, j, H, p));
            // P shares output weights across the two halves. Match reference::head
            // by adding the activations before multiplying by the shared weight.
            if (p) {
                activation += screlu(acc[j + units]
                    - feature_weight(weights, feature, j + units, H, p));
            }
            product = activation * output[j];
        }
        // A tree changes the f32 rounding of after-before, which K amplifies.
        // Compute products in parallel, then retain the CPU's ascending order.
        // Every lane participates in each shuffle; only lane 0 owns the sum.
        for (unsigned source = 0; source < min(32u, units - base); ++source) {
            float value = __shfl_sync(0xffffffffu, product, source);
            if (lane == 0) sum += value;
        }
    }
    // The result is consumed only by lane 0.
    return sum;
}

__device__ unsigned removal_warp_sum(unsigned value) {
    for (unsigned distance = 16; distance > 0; distance /= 2)
        value += __shfl_down_sync(0xffffffffu, value, distance);
    return value;
}

__device__ double removal_warp_sum(double value) {
    for (unsigned distance = 16; distance > 0; distance /= 2)
        value += __shfl_down_sync(0xffffffffu, value, distance);
    return value;
}

extern "C" __global__ void removal_forward(
    const unsigned* features, const unsigned* linear_features,
    const unsigned char* counts, const float* weights, const float* linear,
    const float* output, const float* bias, const float* acc, const float* lin,
    const long long* reference,
    double* e_r, double* delta_r, int* ref_r, float* s_r,
    unsigned char* eligible, unsigned char* wrong_r, double* pen_r, float* d_r,
    float* S, float* removal_acc, float* removal_output, float* removal_linear,
    double* removal_loss, unsigned B, unsigned H, unsigned p, float K) {
    unsigned b = blockIdx.x;
    if (b >= B) return;
    unsigned lane = threadIdx.x % 32;
    unsigned warp = threadIdx.x / 32;
    __shared__ RemovalScratch shared;
    unsigned long long offset = (unsigned long long)b * 2 * H;
    unsigned long long columns_offset = (unsigned long long)b * 144;
    const unsigned* linear_rows = linear_features + (unsigned long long)b * 145;
    const unsigned* position_features = features + (unsigned long long)b * 2 * 145;
    unsigned q = min(90, max(0, (int)counts[b] - 2));
    unsigned after_q = min(90, max(0, (int)counts[b] - 3));

    if (warp < 2) {
        long long reference_sum = 0;
        double linear_sum = 0;
        // The removal reference deliberately sums individual linear rows in
        // double, unlike forward_linear's mirrored pairs in single precision.
        // Two warps handle the endpoints and preserve the reference's order.
        for (unsigned base = 0; base < 145; base += 32) {
            unsigned col = base + lane;
            unsigned row = col < 145 ? linear_rows[col] : 0xffffffffu;
            long long reference_value = row == 0xffffffffu ? 0 : reference[row * 2 + warp];
            double linear_value = (double)linear_weight(linear, row, warp);
            for (unsigned source = 0; source < min(32u, 145 - base); ++source) {
                long long ref_value = __shfl_sync(0xffffffffu, reference_value, source);
                double lin_value = __shfl_sync(0xffffffffu, linear_value, source);
                if (lane == 0) {
                    reference_sum += ref_value;
                    linear_sum += lin_value;
                }
            }
        }
        if (lane == 0) {
            shared.reference_sum[warp] = reference_sum;
            shared.linear_sum[warp] = linear_sum;
        }
    } else {
        unsigned v = warp - 2;
        float value = removal_head(acc + offset + v * H, weights,
            output + v * H, 0xffffffffu, H, p);
        if (lane == 0) shared.before_head[v] = value;
    }
    __syncthreads();
    if (threadIdx.x == 0) {
        shared.before_linear = (double)q * shared.linear_sum[0]
            + (90.0 - (double)q) * shared.linear_sum[1];
        float nonlinear = (shared.before_head[0] + shared.before_head[1]) + bias[0];
        shared.before_e = (double)logit_value(lin + b * 2, nonlinear, phase(counts[b]), K)
            * (double)K;
    }
    __syncthreads();

    long long before_reference = (long long)q * shared.reference_sum[0]
        + (90 - (long long)q) * shared.reference_sum[1];
    for (unsigned col = warp; col < 144; col += blockDim.x / 32) {
        float after[2];
        for (unsigned v = 0; v < 2; ++v) {
            unsigned feature = position_features[v * 145 + col];
            after[v] = removal_head(acc + offset + v * H, weights,
                output + v * H, feature, H, p);
        }
        if (lane != 0) continue;
        unsigned long long i = columns_offset + col;
        unsigned row = linear_rows[col];
        long long reference_weight[2] = {0, 0};
        if (row != 0xffffffffu) {
            reference_weight[0] = reference[row * 2];
            reference_weight[1] = reference[row * 2 + 1];
        }
        long long after_reference = (long long)after_q
                * (shared.reference_sum[0] - reference_weight[0])
            + (90 - (long long)after_q)
                * (shared.reference_sum[1] - reference_weight[1]);
        int reference_delta = (int)(removal_integer(after_reference)
            - removal_integer(before_reference));
        float sign = reference_delta < 0 ? -1.0f : (reference_delta > 0 ? 1.0f : 0.0f);
        // Magnitude is taken before integer truncation and evaluation clipping.
        double magnitude = fabs((double)(after_reference - before_reference) / 720.0);
        unsigned state = row / 72 % 47;
        bool active = row != 0xffffffffu && row < 94 * 72
            && state != 11 && state != 21 && reference_delta != 0;
        bool wrong = (row < 47 * 72 && reference_delta > 0)
            || (row >= 47 * 72 && row < 94 * 72 && reference_delta < 0);
        ref_r[i] = reference_delta;
        s_r[i] = sign;
        eligible[i] = active;
        wrong_r[i] = wrong;
        shared.active[col] = active;

        double after_linear = (double)after_q
                * (shared.linear_sum[0] - (double)linear_weight(linear, row, 0))
            + (90.0 - (double)after_q)
                * (shared.linear_sum[1] - (double)linear_weight(linear, row, 1));
        double delta = (after_linear - shared.before_linear) / 90.0;
        for (unsigned v = 0; v < 2; ++v) {
            delta += (double)K * (double)(after[v] - shared.before_head[v]);
        }
        delta_r[i] = delta;
        e_r[i] = shared.before_e + delta;

        double penalty = 0;
        float slope = 0;
        if (active) {
            double signed_delta = (double)sign * delta;
            double lower = fmin(magnitude, 0.25);
            penalty = fmax(lower - signed_delta, 0.0)
                + (wrong ? fmax(signed_delta - magnitude, 0.0) : 0.0);
            slope = signed_delta < lower ? -sign
                : (wrong && signed_delta > magnitude ? sign : 0.0f);
        }
        shared.penalty[col] = penalty;
        shared.d[col] = slope;
        pen_r[i] = penalty;
    }
    __syncthreads();

    unsigned count = 0;
    for (unsigned col = threadIdx.x; col < 144; col += blockDim.x)
        count += shared.active[col];
    count = removal_warp_sum(count);
    if (lane == 0) shared.count_partials[warp] = count;
    __syncthreads();
    unsigned denominator = max(shared.count_partials[0] + shared.count_partials[1]
        + shared.count_partials[2] + shared.count_partials[3], 1u);
    double scale = 1000.0 / (double)((unsigned long long)B * denominator);
    double sum = 0;
    double loss = 0;
    for (unsigned col = threadIdx.x; col < 144; col += blockDim.x) {
        float d = (float)(scale * (double)shared.d[col]);
        shared.d[col] = d;
        d_r[columns_offset + col] = d;
        if (shared.active[col]) {
            sum += (double)d;
            loss += 1000.0 * shared.penalty[col] / (double)denominator / (double)K;
        }
    }
    // Fixed shuffle trees first combine lanes, then the four warp partials.
    sum = removal_warp_sum(sum);
    loss = removal_warp_sum(loss);
    if (lane == 0) {
        shared.sum_partials[warp] = sum;
        shared.loss_partials[warp] = loss;
    }
    __syncthreads();
    if (warp == 0) {
        sum = removal_warp_sum(lane < 4 ? shared.sum_partials[lane] : 0.0);
        loss = removal_warp_sum(lane < 4 ? shared.loss_partials[lane] : 0.0);
        if (lane == 0) {
            S[b] = (float)sum;
            removal_loss[b] = loss;
            float u = phase(counts[b]);
            float after_u = (float)after_q / 90.0f;
            removal_linear[b * 2] = (float)(sum * (double)(after_u - u) / (double)K);
            removal_linear[b * 2 + 1] = (float)(sum
                * (double)((1.0f - after_u) - (1.0f - u)) / (double)K);
        }
    }
    __syncthreads();

    for (unsigned vj = threadIdx.x; vj < 2 * H; vj += blockDim.x) {
        unsigned v = vj / H;
        unsigned j = vj % H;
        double acc_sum = 0;
        double output_sum = 0;
        for (unsigned col = 0; col < 144; ++col) {
            if (!shared.active[col]) continue;
            unsigned feature = position_features[v * 145 + col];
            float after = acc[offset + vj] - feature_weight(weights, feature, j, H, p);
            double d = (double)shared.d[col];
            acc_sum += d * (double)output[vj] * (double)derivative(after);
            output_sum += d * (double)screlu(after);
        }
        removal_acc[offset + vj] = (float)acc_sum;
        removal_output[offset + vj] = (float)output_sum;
    }
}
