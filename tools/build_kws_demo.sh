#!/bin/sh
# build_kws_demo.sh —— 编译官方 KWS demo (补齐缺失依赖: kaldi-native-fbank + kissfft + 自写 audio_utils)
set -e
K=/home/radxa/npu_demos/kws_npu_demo
SRC=/home/radxa/npu_demos/voice_assistant/convert/kws
INC=$K/inc/kaldi-native-fbank/csrc
VIP=/home/radxa/npu-drv/aw_nna_vip/vip2/inc

g++ -O2 -std=c++17 \
  "$SRC/src/main.cpp" "$SRC/src/aw_zipformer.cpp" "$SRC/src/kws_decode.cpp" "$SRC/src/process.cpp" \
  "$K/audio_utils_nosndfile.c" \
  "$INC/feature-fbank.cc" "$INC/feature-functions.cc" "$INC/feature-mfcc.cc" \
  "$INC/feature-window.cc" "$INC/online-feature.cc" "$INC/rfft.cc" "$INC/log.cc" \
  "$INC/feature-raw-audio-samples.cc" \
  "$K/kissfft/kiss_fft.c" "$K/kissfft/kiss_fftr.c" \
  -I"$SRC/include" -I"$K/inc" -I"$INC" -I"$K/kissfft" -I"$VIP" \
  -L/home/radxa/lib -lVIPhal -lNBGlinker -lm -lpthread \
  -o "$K/kws_npu_demo_a733"
ls -la "$K/kws_npu_demo_a733"
