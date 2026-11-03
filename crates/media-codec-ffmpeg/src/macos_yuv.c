#include <Accelerate/Accelerate.h>
#include <stdint.h>

// Convert directly from the shared RGBA frame into reusable NV12 planes. vImage
// uses the system SIMD implementation; no intermediate RGBA AVFrame is needed.
long lightos_rgba_to_nv12(const uint8_t *rgba, size_t width, size_t height,
                         uint8_t *y, size_t y_stride,
                         uint8_t *uv, size_t uv_stride) {
    const vImage_YpCbCrPixelRange range = {
        .Yp_bias = 16, .CbCr_bias = 128, .YpRangeMax = 235,
        .CbCrRangeMax = 240, .YpMax = 235, .YpMin = 16,
        .CbCrMax = 240, .CbCrMin = 16
    };
    vImage_ARGBToYpCbCr conversion;
    vImage_Error status = vImageConvert_ARGBToYpCbCr_GenerateConversion(
        kvImage_ARGBToYpCbCrMatrix_ITU_R_709_2, &range, &conversion,
        kvImageARGB8888, kvImage420Yp8_CbCr8, kvImageNoFlags);
    if (status != kvImageNoError) return status;
    const vImage_Buffer source = {(void *)rgba, height, width, width * 4};
    const vImage_Buffer luma = {y, height, width, y_stride};
    const vImage_Buffer chroma = {uv, height / 2, width / 2, uv_stride};
    const uint8_t argb_from_rgba[4] = {3, 0, 1, 2};
    return vImageConvert_ARGB8888To420Yp8_CbCr8(
        &source, &luma, &chroma, &conversion, argb_from_rgba, kvImageNoFlags);
}
