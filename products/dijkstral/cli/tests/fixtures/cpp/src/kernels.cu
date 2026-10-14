#include "kernels.h"

__global__ void scale_kernel(float *data, int n, float k) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n) data[i] *= k;
}

void launch_scale(float *data, int n, float k) {
    scale_kernel<<<(n + 255) / 256, 256>>>(data, n, k);
}
