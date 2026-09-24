#!/usr/bin/env bash
# 构建 lyco-agent APK —— **无 Android Studio**（借鉴 lilyco-42/radxa-monitor）
#
# aapt2 link → javac(-encoding UTF-8) → d8 → aapt add dex → zipalign → apksigner
#
# ⚠️ d8 必须用 build-tools 36.1+：34 的 d8 对某些字节码会 NPE（radxa-monitor README 实测）
#
# 用法:
#   ANDROID_HOME=... bash android/build.sh
# 产物: android/build/lyco-agent-signed.apk

set -euo pipefail
cd "$(dirname "$0")"

AH="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$AH" ]; then
  echo "需要 ANDROID_HOME（或 ANDROID_SDK_ROOT）" >&2
  exit 2
fi
BT="$AH/build-tools/36.1.0"
PLAT="$AH/platforms/android-36"
if [ ! -d "$BT" ] || [ ! -d "$PLAT" ]; then
  echo "缺 $BT 或 $PLAT —— 请先 sdkmanager --install 'build-tools;36.1.0' 'platforms;android-36'" >&2
  exit 2
fi

# liblycore.so 必须先就位（由 CI 或手动从 lycore/target 拷来）
if [ ! -f jniLibs/arm64-v8a/liblycore.so ]; then
  echo "缺 jniLibs/arm64-v8a/liblycore.so —— 先跑 cargo build --target aarch64-linux-android" >&2
  exit 2
fi

rm -rf build
mkdir -p build/gen build/classes

echo "== 1/5 aapt2 link =="
"$BT/aapt2" link \
  -o build/unsigned.apk \
  -I "$PLAT/android.jar" \
  --manifest AndroidManifest.xml \
  --java build/gen \
  -A assets \
  --min-sdk-version 24 \
  --target-sdk-version 34 \
  --auto-add-overlay

# JSch —— SSH 执行通道（`hw` 在板子上，必须远程跑）。jar 不入库，按需下载。
# 0.2.17 是 radxa-monitor 验证过的版本；它是 mwiede 的维护分支（原 JCraft 已停更）。
JSCH="libs/jsch-0.2.17.jar"
if [ ! -f "$JSCH" ]; then
  echo "== 下载 JSch =="
  mkdir -p libs
  curl -fsSL -o "$JSCH" \
    https://repo1.maven.org/maven2/com/github/mwiede/jsch/0.2.17/jsch-0.2.17.jar
fi

echo "== 2/5 javac =="
# --release 8 是必需的：JDK 21 默认产 class v65，d8 不认
javac -encoding UTF-8 --release 8 \
  -classpath "$PLAT/android.jar:$JSCH" \
  -d build/classes \
  $(find src build/gen -name '*.java')

echo "== 3/5 d8 =="
"$BT/d8" --lib "$PLAT/android.jar" --lib "$JSCH" --output build/ \
  $(find build/classes -name '*.class')

echo "== 4/5 打进 APK + zipalign =="
(cd build && zip -q unsigned.apk classes.dex)
# .so 也要进包（aapt2 link 不会自动收 jniLibs/）
(cd build && mkdir -p lib/arm64-v8a && cp ../jniLibs/arm64-v8a/liblycore.so lib/arm64-v8a/ \
  && zip -q unsigned.apk lib/arm64-v8a/liblycore.so)
"$BT/zipalign" -f 4 build/unsigned.apk build/aligned.apk

echo "== 5/5 apksigner =="
if [ ! -f build/debug.keystore ]; then
  keytool -genkeypair -v -keystore build/debug.keystore -alias androiddebugkey \
    -keyalg RSA -keysize 2048 -validity 10000 \
    -storepass android -keypass android \
    -dname "CN=lyco, O=lyco, C=CN" >/dev/null
fi
"$BT/apksigner" sign \
  --ks build/debug.keystore --ks-pass pass:android --key-pass pass:android \
  --out build/lyco-agent-signed.apk build/aligned.apk

echo
echo "✅ $(ls -la build/lyco-agent-signed.apk | awk '{print $5}') bytes → android/build/lyco-agent-signed.apk"
