#!/bin/bash
# dl_st.sh — curl 直下 5 个 AWQ safetensors (绕过 xet CAS 401)
cd /workspace/qwen38_awq
for i in 1 2 3 4 5; do
  F="model-0000${i}-of-00005.safetensors"
  if [ ! -f "$F" ]; then
    echo "downloading $F"
    curl -sL -o "$F" "https://hf-mirror.com/cyankiwi/Qwen3.8-27B-AWQ-INT4/resolve/main/$F"
  fi
done
echo "ST_DL_COMPLETE"
ls -la *.safetensors
