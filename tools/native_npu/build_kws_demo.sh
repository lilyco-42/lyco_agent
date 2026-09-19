#!/bin/sh
# build_kws_demo.sh —— 编译官方 KWS demo (补齐缺失依赖: kaldi-native-fbank + kissfft + 自写 audio_utils)
set -e
K=/home/radxa/npu_demos/kws_npu_demo
SRC=/home/radxa/npu_demos/voice_assistant/convert/kws
INC=$K/inc/kaldi-native-fbank/csrc
VIP=/home/radxa/npu-drv/aw_nna_vip/vip2/inc
VSDK=/home/radxa/npu-sdk/viplite-tina/lib/aarch64-none-linux-gnu/v2.0/inc

KNFSRC=$(ls "$INC"/*.cc | grep -v "/test-" | tr '\n' ' ')
echo "knf 源: $(echo $KNFSRC | wc -w) 个"

# shellcheck disable=SC2086
g++ -O2 -std=c++17 \
  "$SRC/src/main.cpp" "$SRC/src/aw_zipformer.cpp" "$SRC/src/kws_decode.cpp" "$SRC/src/process.cpp" \
  "$K/npulib.cpp" "$K/npu_util.cpp" \
  "$K/audio_utils_nosndfile.c" \
  $KNFSRC \
  "$K/kissfft/kiss_fft.c" "$K/kissfft/kiss_fftr.c" \
  -I"$SRC/include" -I"$K/inc" -I"$INC" -I"$K/kissfft" -I"$K" -I"$VIP" -I"$VSDK" \
  -L/home/radxa/lib /home/radxa/lib/libVIPhal.so /home/radxa/lib/libNBGlinker.so -lm -lpthread \
  -o "$K/kws_npu_demo_a733"
ls -la "$K/kws_npu_demo_a733"
