#!/usr/bin/env bash
# 恢复出库资产: smoke 测试夹具 + vnn CNN 权重。
# 来源: GitHub Release fixtures-2026-09-20; 清单与校验和: smoke/fixtures.lock
# 用法: bash scripts/fetch_fixtures.sh   (在仓库根目录执行; CI 在 cargo test 前调用)
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="fixtures-2026-09-20"
BASE="https://github.com/lilyco-42/lyco_agent/releases/download/${TAG}"
LOCK="smoke/fixtures.lock"

fetch() { # $1=资产名 $2=期望sha256
    local name="$1" want="$2" got
    local url="${BASE}/${name}"
    echo "[fetch_fixtures] ${url}"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "${name}" "${url}"
    else
        wget -q -O "${name}" "${url}"
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        got=$(sha256sum "${name}" | cut -d' ' -f1)
        if [ "${got}" != "${want}" ]; then
            echo "[fetch_fixtures] sha256 不匹配: got=${got} want=${want}" >&2
            exit 1
        fi
    fi
    tar -xzf "${name}"
    rm -f "${name}"
    echo "[fetch_fixtures] ${name} 解压完成"
}

# 从 lock 解析: <资产名>  <解压目标>  <sha256>  <文件数>
parsed=$(grep -E '^(smoke-fixtures|vnn-weights)\.tar\.gz' "${LOCK}" | awk '{print $1, $3}')
while read -r name want; do
    [ -z "${name}" ] && continue
    fetch "${name}" "${want}"
done <<< "${parsed}"

echo "[fetch_fixtures] 全部就绪: smoke/ 夹具 + lycore/assets/ 权重"
