#!/usr/bin/env bash
# lyco_agent NPU 环境体检 — Radxa Cubie A7A / 全志 A733 (Vivante VIP9000)
#
# 只读: 不加载驱动、不改系统、不跑推理。跑完把整段输出贴回去做下一步决策。
# 用法: bash scripts/npu_probe.sh
#
# 背景: NPU 在 5.15.147-21-a733 / 6.6.98-sun60iw2 上可用,
#       在 6.6.98-4-aw2511 (Radxa 6.6 BSP) 上已知 hang。内核分支决定一切。

set -u

hr()  { printf '%s\n' "------------------------------------------------------------"; }
kv()  { printf '%-24s %s\n' "$1" "${2:-<none>}"; }
have() { command -v "$1" >/dev/null 2>&1; }

hr
printf 'lyco_agent NPU probe   %s\n' "$(date -Is 2>/dev/null || date)"
hr

echo "[1] 内核 / 系统"
kv "uname -r"  "$(uname -r)"
kv "arch"      "$(uname -m)"
if [ -r /etc/os-release ]; then
  kv "os" "$(. /etc/os-release; printf '%s' "${PRETTY_NAME:-unknown}")"
fi
kv "cores" "$(nproc 2>/dev/null || echo '?')"

case "$(uname -r)" in
  5.15.147-21-a733*) KSTAT="YES (Radxa vendor, petayyyy 验证可用)" ;;
  6.6.98-sun60iw2*)  KSTAT="YES (Orange Pi 分支, 验证可用)" ;;
  6.6.98-4-aw2511*)  KSTAT="NO (Radxa 6.6 BSP, 已知 hang: VIPDRV_WAIT_TASK status=-1)" ;;
  *)                 KSTAT="UNKNOWN (未验证分支, 需实测)" ;;
esac
kv "kernel supported?" "$KSTAT"

echo
echo "[2] NPU 设备节点"
if [ -c /dev/vipcore ]; then
  kv "/dev/vipcore" "present ($(stat -c 'major=%t minor=%T' /dev/vipcore 2>/dev/null)) 期望 major=c7(199) minor=0"
  VIPCORE=yes
else
  kv "/dev/vipcore" "MISSING  <- 驱动没起来, G0 FAIL"
  VIPCORE=no
fi
if [ -c /dev/galcore ]; then
  kv "/dev/galcore" "present (galcore/TIM-VX 路线 — 本方案不用, 忽略)"
else
  kv "/dev/galcore" "absent (正常, 我们走 VIPLite)"
fi

echo
echo "[3] VIPLite 运行时"
VIPHAL=$(find /usr /opt /home -name 'libVIPhal.so'    2>/dev/null | head -1)
NBGLNK=$(find /usr /opt /home -name 'libNBGlinker.so' 2>/dev/null | head -1)
VPM=$(   find /usr /opt /home -type f -name 'vpm_run' 2>/dev/null | head -1)
SDK=$(   find /home /opt /root -maxdepth 3 -type d -name 'ai-sdk' 2>/dev/null | head -1)
kv "libVIPhal.so"    "${VIPHAL:-<not found>}"
kv "libNBGlinker.so" "${NBGLNK:-<not found>}"
kv "vpm_run"         "${VPM:-<not found>}"
kv "ai-sdk dir"      "${SDK:-<not found>}"

echo
echo "[4] 内存 / 磁盘 (NBG 常驻吃 RAM, /tmp 是 tmpfs 也吃 RAM)"
free -h 2>/dev/null | sed -n '1,2p'
df -h /tmp 2>/dev/null | sed -n '1,2p'
have swapon && { echo "swap:"; swapon --show 2>/dev/null || echo "  (无 swap)"; }

echo
echo "[5] 现成示例模型 (SDK 自带, 最小验证用)"
if [ -n "$SDK" ] && [ -d "$SDK/models" ]; then
  ls -1 "$SDK/models" 2>/dev/null | grep -v '\.sh$\|\.py$\|\.txt$\|README' | head -10
else
  echo "  <ai-sdk 未就位: git clone https://github.com/ZIFENG278/ai-sdk ~/ai-sdk>"
fi

echo
hr
echo "判定"
if [ "$VIPCORE" = yes ]; then
  echo "  G0 设备节点      : PASS"
else
  echo "  G0 设备节点      : FAIL  -> 见 lyco-ops 仓 NPU_SETUP.md §1 (github.com/lilyco-42/lyco-ops), 大概率要刷 5.15 内核"
fi
if [ -n "$VIPHAL" ] && [ -n "$VPM" ]; then
  echo "  G1 VIPLite 运行时: PASS  -> 可以跑现成 NBG 了"
else
  echo "  G1 VIPLite 运行时: FAIL  -> git clone https://github.com/ZIFENG278/ai-sdk ~/ai-sdk"
fi
echo "  G2 现成 NBG 推理 : 未测 (过 G1 后跑 vpm_run + models/mobilenet_v1_1.0_224_quant)"
hr
echo "把上面整段输出贴回, 再决定要不要继续投 NPU。"
