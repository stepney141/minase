// 逆索引の1語は上位から局面22、視点1、列8、回転1ビットであり、PackedOccurrenceと一致する。
// 第1層は1ブロックが128要素以下の区分を担当し、隣接スレッドが隣接単位を計算する。
// 区分内はf32、区分間は昇順にf64で足し、空行と埋め草には0を書く。
// 線形は1ワープが1区分を担当し、回転せず正準行へ縮約する。
// 特徴の番兵0xffffffffはfeature_weightが重み0として読む。
extern "C" __global__ void first_layer_backward_partials(
    const unsigned* first_segment_offsets, const unsigned* first_entries,
    const unsigned* features, const float* weights, const float* acc,
    const float* output, const float* d_r, const float* d_acc,
    double* first_partials, unsigned H, unsigned p) {
    unsigned segment = blockIdx.x;
    unsigned begin = first_segment_offsets[segment];
    unsigned end = first_segment_offsets[segment + 1];
    // A block owns one interval of at most 128 inverse occurrences. Adjacent
    // lanes own adjacent units, including after a P-type half-width rotation.
    for (unsigned j = threadIdx.x; j < H; j += blockDim.x) {
        float sum = 0;
        for (unsigned e = begin; e < end; ++e) {
            unsigned word = first_entries[e];
            unsigned b = word >> 10;
            unsigned v = (word >> 9) & 1;
            unsigned col = (word >> 1) & 255;
            unsigned bv = b * 2 + v;
            float d = col < 144 ? d_r[(unsigned long long)b * 144 + col] : 0.0f;
            unsigned feature = features[(unsigned long long)bv * 145 + col];
            unsigned unit = p && (word & 1) ? (j + H / 2) % H : j;
            unsigned long long i = (unsigned long long)bv * H + unit;
            float removed = acc[i] - feature_weight(weights, feature, unit, H, p);
            // Inverse rotation applies to A, the accumulator, and output weights.
            sum += d_acc[i] - d * output[v * H + unit] * derivative(removed);
        }
        first_partials[(unsigned long long)segment * H + j] = sum;
    }
}

extern "C" __global__ void first_layer_backward_reduce(
    const unsigned* first_segment_starts, const double* first_partials,
    float* d_first, unsigned rows, unsigned H) {
    unsigned long long t = (unsigned long long)blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= (unsigned long long)rows * H) return;
    unsigned row = t / H;
    unsigned j = t % H;
    // HalfKP's inverse index omits its padding row, so do not read its bounds.
    unsigned begin = row + 1 < rows ? first_segment_starts[row] : 0;
    unsigned end = row + 1 < rows ? first_segment_starts[row + 1] : 0;
    double sum = 0;
    for (unsigned segment = begin; segment < end; ++segment) {
        sum += first_partials[(unsigned long long)segment * H + j];
    }
    // Reduce the f32 segment sums in f64; empty rows and the padding row write 0.
    d_first[t] = (float)sum;
}

extern "C" __global__ void linear_backward_partials(
    const unsigned* linear_segment_offsets, const unsigned* linear_entries,
    const unsigned char* counts, const float* d_r, const float* c,
    double* linear_partials, unsigned segments, float K) {
    unsigned lane = threadIdx.x % 32;
    unsigned segment = blockIdx.x * (blockDim.x / 32) + threadIdx.x / 32;
    // The branch is uniform within each warp, including the final block.
    if (segment >= segments) return;
    unsigned begin = linear_segment_offsets[segment];
    unsigned end = linear_segment_offsets[segment + 1];
    float sums[2] = {0, 0};
    for (unsigned e = begin + lane; e < end; e += 32) {
        unsigned word = linear_entries[e];
        unsigned b = word >> 10;
        unsigned col = (word >> 1) & 255;
        float d = col < 144 ? d_r[(unsigned long long)b * 144 + col] : 0.0f;
        float phi = (float)min(90, max(0, (int)counts[b] - 3)) / 90.0f;
        float u[2] = {phi, 1.0f - phi};
        for (unsigned endpoint = 0; endpoint < 2; ++endpoint) {
            sums[endpoint] += c[b * 2 + endpoint] - d * u[endpoint] / K;
        }
    }
    // Every lane participates, with zero for an incomplete interval. This fixed
    // 16,8,4,2,1 tree has no order dependence on warp scheduling.
    for (unsigned offset = 16; offset > 0; offset /= 2) {
        sums[0] += __shfl_down_sync(0xffffffffu, sums[0], offset);
        sums[1] += __shfl_down_sync(0xffffffffu, sums[1], offset);
    }
    if (lane == 0) {
        linear_partials[(unsigned long long)segment * 2] = sums[0];
        linear_partials[(unsigned long long)segment * 2 + 1] = sums[1];
    }
}

extern "C" __global__ void linear_backward_reduce(
    const unsigned* linear_segment_starts, const double* linear_partials,
    float* d_linear, unsigned rows) {
    unsigned row = blockIdx.x * blockDim.x + threadIdx.x;
    if (row >= rows) return;
    double sums[2] = {0, 0};
    for (unsigned segment = linear_segment_starts[row];
         segment < linear_segment_starts[row + 1]; ++segment) {
        sums[0] += linear_partials[(unsigned long long)segment * 2];
        sums[1] += linear_partials[(unsigned long long)segment * 2 + 1];
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
